mod app;
mod config;
mod output;
mod runtime;

use config::BatchConfig;

struct PluginPath {
    parse: String,
    format: String,
    transport: String,
}

/// 控制 pipeline 中哪些處理階段要啟用。
pub struct PipelineStages {
    pub format: bool,
    pub transport: bool,
}

fn main() -> wasmtime::Result<()> {
    let path = PluginPath {
        parse: String::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/test-plugins/go-plugin/parser/parser.wasm"
        )),
        format: String::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/test-plugins/go-plugin/format/format.wasm"
        )),
        transport: String::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/test-plugins/rust-plugin/transport/target/wasm32-unknown-unknown/release/transport_component.wasm"
        )),
    };

    let stages = PipelineStages {
        format: true,
        transport: true,
    };

    let cfg = BatchConfig::default();
    let mem_limit_bytes = cfg.mem_limit_mb * 1024 * 1024;
    let safe_data_budget = (mem_limit_bytes as f64 * cfg.safe_data_ratio) as usize;

    output::print_startup(&cfg, safe_data_budget, &stages);

    let (tx, rx) = std::sync::mpsc::sync_channel::<config::LineItem>(cfg.channel_capacity);
    app::spawn_stdin_reader(tx);

    let (engine_parse, component_parse, linker_parse) =
        app::build_runtime(mem_limit_bytes, path.parse)?;

    let format_runtime = if stages.format {
        Some(app::build_runtime(mem_limit_bytes, path.format)?)
    } else {
        None
    };

    let transport_runtime = if stages.transport {
        Some(app::build_transport_runtime(mem_limit_bytes, path.transport)?)
    } else {
        None
    };

    runtime::run_pipeline(
        rx,
        Some((engine_parse, component_parse, linker_parse)),
        format_runtime,
        transport_runtime,
        cfg,
    )?;

    Ok(())
}
