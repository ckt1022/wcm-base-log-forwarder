mod app;
mod capability;
mod config;
mod isolation;
mod native;
mod output;
mod runtime;

use std::path::Path;
use std::sync::{Arc, RwLock};

use capability::{FsGrant, SocketGrant, Stage, StageGrant};
use config::{AppConfig, BatchConfig, InputMode, PluginSlots, is_native_stage, load_app_config, spawn_config_watcher};
use runtime::StageImpl;

/// `--check-plugin <stage> <wasm> [inert-fs] [inert-sockets|endpoint-sockets]`
/// 只做載入時能力檢查並列出 import，不啟動 pipeline（供文件／實驗記錄使用）。
fn check_plugin_cli(args: &[String]) -> ! {
    let usage = "usage: wcm-base-log-forwarder --check-plugin <parse|filter|format|transport> <plugin.wasm> [inert-fs] [inert-sockets|endpoint-sockets]";
    let (Some(stage), Some(path)) = (args.first().and_then(|s| Stage::from_label(s)), args.get(1)) else {
        eprintln!("{}", usage);
        std::process::exit(2);
    };
    let mut grant = StageGrant::default();
    for a in &args[2..] {
        match a.as_str() {
            "inert-fs" => grant.filesystem = FsGrant::Inert,
            "inert-sockets" => grant.sockets = SocketGrant::Inert,
            "endpoint-sockets" => grant.sockets = SocketGrant::Endpoint,
            other => {
                eprintln!("unknown grant '{}'\n{}", other, usage);
                std::process::exit(2);
            }
        }
    }
    // 能力檢查通過後再做 instantiate_pre：確認受限 linker 真的能滿足所有 import（不執行 guest 程式碼）
    let result = if stage == Stage::Transport {
        app::build_transport_runtime(path.clone(), grant)
    } else {
        app::build_runtime(path.clone(), stage, grant)
    }
    .and_then(|(_, component, linker)| linker.instantiate_pre(&component).map(|_| ()));
    // 第二次載入：分段計時（編譯／能力檢查／連結／instantiate_pre），只在 --check-plugin 輸出
    match app::timed_load_breakdown(path, stage, grant) {
        Ok([c, k, l, p]) => println!(
            "TIMING compile_ms={:.3} check_ms={:.3} link_ms={:.3} instantiate_pre_ms={:.3}", c, k, l, p),
        Err(e) => println!("TIMING {}", e.lines().next().unwrap_or("")),
    }
    match result {
        Ok(()) => {
            println!("LOAD_OK stage={} grant={:?} plugin={}", stage, grant, path);
            std::process::exit(0);
        }
        Err(e) => {
            println!("LOAD_REFUSED stage={} grant={:?} plugin={}\n{}", stage, grant, path, e);
            std::process::exit(1);
        }
    }
}

// 主程式入口
fn main() -> wasmtime::Result<()> {
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str) == Some("--check-plugin") {
        check_plugin_cli(&argv[2..]);
    }

    // 服務的設定檔案，可以透過啟動程式碼的第一個參數來輸入
    // 若沒有則使用預設檔案 root/forwarder.yaml
    let config_path = argv
        .get(1)
        .cloned()
        .unwrap_or_else(|| "forwarder.yaml".to_string());

    // 讀取YAML設定檔案
    let app_cfg = match load_app_config(&config_path) {
        Ok(c) => c,
        Err(e) => { eprintln!("[config] {}", e); std::process::exit(1); }
    };

    let batch_cfg = BatchConfig::from(app_cfg.batch.clone());
    if let Err(e) = batch_cfg.validate_and_describe() {
        eprintln!("[config-error] {}", e);
        std::process::exit(1);
    }

    output::print_startup(&batch_cfg, &app_cfg.stages);

    // epoch interruption watchdog 開關（啟動後固定；必須在建立任何 engine 之前設定）
    app::set_epoch_interruption(batch_cfg.epoch_interruption);
    eprintln!("[config] epoch_interruption={}", batch_cfg.epoch_interruption);

    // 以下為讀取設定檔案並建立plugin的插槽

    // 取出config檔案的路徑
    let config_dir = Path::new(&config_path)
        .parent()
        .unwrap_or(Path::new("."));
    
    // 組合成plugin路徑
    let plugin_dir = config_dir.join("plugin/");

    // resolve讀取每個plugin的路徑並組合成絕對路徑
    let resolve = |p: &str| -> String {
        let pp = Path::new(p);
        if pp.is_absolute() { p.to_string() }
        else { plugin_dir.join(pp).to_string_lossy().to_string() }
    };

    // Build SharedPlugin slots from config paths
    // 建立每個元件的插槽，並且送入pipeline函數。
    // plugin 欄位留空或填 `native` 的 stage 改用 host 內建原生實作（不建立 WASM 插槽）；
    // 有填 .wasm 的 stage 在載入時依 capabilities 做 import 檢查，未授權直接拒絕啟動。
    let caps = app_cfg.capabilities.clone();
    let load = |path: &str, stage: Stage| -> wasmtime::Result<Option<app::SharedPlugin>> {
        if is_native_stage(path) {
            return Ok(None);
        }
        let grant = caps.grant(stage);
        let slot = if stage == Stage::Transport {
            app::new_shared_transport_runtime(&resolve(path), grant)
        } else {
            app::new_shared_runtime(&resolve(path), stage, grant)
        };
        match slot {
            Ok(s) => Ok(Some(s)),
            Err(e) => {
                eprintln!("{}", e);
                Err(e)
            }
        }
    };

    let parse_slot = load(&app_cfg.plugins.parse, Stage::Parse)?;

    let parse_noop_slot = match app_cfg.plugins.parse_noop.as_deref() {
        Some(p) => load(p, Stage::Parse)?,
        None => None,
    };

    let filter_slot = if app_cfg.stages.filter { load(&app_cfg.plugins.filter, Stage::Filter)? } else { None };
    let format_slot = if app_cfg.stages.format { load(&app_cfg.plugins.format, Stage::Format)? } else { None };
    let transport_slot = if app_cfg.stages.transport { load(&app_cfg.plugins.transport, Stage::Transport)? } else { None };

    // 每個啟用的 stage 對應的實作（WCM plugin 或原生）
    let to_impl = |enabled: bool, slot: &Option<app::SharedPlugin>| -> Option<StageImpl> {
        if !enabled {
            None
        } else {
            Some(match slot {
                Some(s) => StageImpl::Wasm(Arc::clone(s)),
                None => StageImpl::Native,
            })
        }
    };
    let parse_impl = to_impl(true, &parse_slot).expect("parse stage always enabled");
    let filter_impl = to_impl(app_cfg.stages.filter, &filter_slot);
    let format_impl = to_impl(app_cfg.stages.format, &format_slot);
    let transport_impl = to_impl(app_cfg.stages.transport, &transport_slot);
    output::print_stage_impls(&parse_impl, filter_impl.as_ref(), format_impl.as_ref(), transport_impl.as_ref());

    // Wrap AppConfig in Arc for hot-reload; start config watcher
    let shared_cfg: Arc<RwLock<AppConfig>> = Arc::new(RwLock::new(app_cfg.clone()));

    let slots = PluginSlots {
        parse: parse_slot.as_ref().map(Arc::clone),
        parse_noop: parse_noop_slot.as_ref().map(Arc::clone),
        filter: filter_slot.as_ref().map(Arc::clone),
        format: format_slot.as_ref().map(Arc::clone),
        transport: transport_slot.as_ref().map(Arc::clone),
    };
    spawn_config_watcher(config_path.clone(), Arc::clone(&shared_cfg), slots);

    // Input channel
    let (tx, rx) = std::sync::mpsc::sync_channel::<String>(batch_cfg.channel_capacity);

    match &app_cfg.input.mode {
        InputMode::Tcp => {
            let tcp = app_cfg.input.tcp.as_ref().unwrap_or_else(|| {
                eprintln!("[config] input.tcp section required when mode=tcp");
                std::process::exit(1);
            });
            app::spawn_tcp_reader(tcp.host.clone(), tcp.port, tx);
        }
        InputMode::Tail => {
            let tail = app_cfg.input.tail.as_ref().unwrap_or_else(|| {
                eprintln!("[config] input.tail section required when mode=tail");
                std::process::exit(1);
            });
            app::spawn_tail_reader(tail.path.clone(), tx);
        }
    }

    runtime::run_pipeline(
        rx,
        parse_impl,
        parse_noop_slot,
        filter_impl,
        format_impl,
        transport_impl,
        shared_cfg,
    )?;

    Ok(())
}
