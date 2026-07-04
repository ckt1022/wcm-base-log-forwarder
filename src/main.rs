mod app;
mod config;
mod output;
mod runtime;

use std::path::Path;
use std::sync::{Arc, RwLock};

use config::{AppConfig, BatchConfig, InputMode, PluginSlots, load_app_config, spawn_config_watcher};

// 主程式入口
fn main() -> wasmtime::Result<()> {
    // 服務的設定檔案，可以透過啟動程式碼的第一個參數來輸入
    // 若沒有則使用預設檔案 root/forwarder.yaml
    let config_path = std::env::args()
        .nth(1)
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

    let mem_limit_bytes = batch_cfg.mem_limit_mb * 1024 * 1024;
    let safe_data_budget = (mem_limit_bytes as f64 * batch_cfg.safe_data_ratio) as usize;
    output::print_startup(&batch_cfg, safe_data_budget, &app_cfg.stages);

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
    // 建立每個元件的插槽，並且送入pipeline函數
    let parse_slot = app::new_shared_runtime(&resolve(&app_cfg.plugins.parse))?;

    let parse_noop_slot = app_cfg.plugins.parse_noop.as_deref()
        .map(|p| app::new_shared_runtime(&resolve(p)))
        .transpose()?;

    let filter_slot = if app_cfg.stages.filter {
        Some(app::new_shared_runtime(&resolve(&app_cfg.plugins.filter))?)
    } else {
        None
    };

    let format_slot = if app_cfg.stages.format {
        Some(app::new_shared_runtime(&resolve(&app_cfg.plugins.format))?)
    } else {
        None
    };

    let transport_slot = if app_cfg.stages.transport {
        Some(app::new_shared_transport_runtime(&resolve(&app_cfg.plugins.transport))?)
    } else {
        None
    };

    // Wrap AppConfig in Arc for hot-reload; start config watcher
    let shared_cfg: Arc<RwLock<AppConfig>> = Arc::new(RwLock::new(app_cfg.clone()));

    let slots = PluginSlots {
        parse: Arc::clone(&parse_slot),
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
        Some(parse_slot),
        parse_noop_slot,
        filter_slot,
        format_slot,
        transport_slot,
        shared_cfg,
    )?;

    Ok(())
}
