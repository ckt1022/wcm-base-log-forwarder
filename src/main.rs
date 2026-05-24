mod app;
mod config;
mod output;
mod runtime;

use std::path::Path;
use std::sync::{Arc, RwLock};

use config::{AppConfig, BatchConfig, InputMode, PluginSlots, load_app_config, spawn_config_watcher};

fn main() -> wasmtime::Result<()> {
    let config_path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "forwarder.yaml".to_string());

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

    let config_dir = Path::new(&config_path)
        .parent()
        .unwrap_or(Path::new("."));

    let resolve = |p: &str| -> String {
        let pp = Path::new(p);
        if pp.is_absolute() { p.to_string() }
        else { config_dir.join(pp).to_string_lossy().to_string() }
    };

    // Build SharedPlugin slots from config paths
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
