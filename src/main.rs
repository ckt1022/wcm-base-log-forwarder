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

    // 判斷啟動哪些stage
    let stages = PipelineStages {
        format: true,
        transport: false,
    };

    // 
    let cfg = BatchConfig::default();
    let mem_limit_bytes = cfg.mem_limit_mb;
    let safe_data_budget = (mem_limit_bytes as f64 * cfg.safe_data_ratio) as usize;

    output::print_startup(&cfg, safe_data_budget, &stages);

    // 接收輸入與送進channel between    input -> channel -> parse
    let (tx, rx) = std::sync::mpsc::sync_channel::<config::LineItem>(cfg.channel_capacity);
    app::spawn_stdin_reader(tx);

    // stage wasm runtime engine amd component and linker
    // engine,component可以重複使用，用來建立store
    // 不同instance用相同engine，不同store
    let (engine_parse, component_parse, linker_parse) =
        app::build_runtime(path.parse)?;

    let format_runtime = if stages.format {
        Some(app::build_runtime(path.format)?)
    } else {
        None
    };

    let transport_runtime = if stages.transport {
        Some(app::build_transport_runtime(path.transport)?)
    } else {
        None
    };

    // runpipeline
    runtime::run_pipeline(
        rx,
        Some((engine_parse, component_parse, linker_parse)),
        format_runtime,
        transport_runtime,
        cfg,
    )?;

    Ok(())
}
