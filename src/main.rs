mod app;
mod config;
mod output;
mod runtime;

use config::BatchConfig;

struct PluginPath {
    parse: String,
    //enrich: String,
    //filter: String,
    //router: String,
    format: String,
    //transport: String,
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
    };

    let cfg = BatchConfig::default();
    let mem_limit_bytes = cfg.mem_limit_mb * 1024 * 1024;
    let safe_data_budget = (mem_limit_bytes as f64 * cfg.safe_data_ratio) as usize;

    output::print_startup(cfg, safe_data_budget);

    let (tx, rx) = std::sync::mpsc::sync_channel::<config::LineItem>(cfg.channel_capacity);
    app::spawn_stdin_reader(tx);

    // Engine / Component / Linker 各自獨立：parse thread 擁有 parse 套件，
    // main thread 借用 format 套件。
    let (engine_parse, linker_parse, component_parse) =
        app::build_runtime(mem_limit_bytes, path.parse)?;
    let (engine_fmt, linker_fmt, component_fmt) =
        app::build_runtime(mem_limit_bytes, path.format)?;

    runtime::run_pipeline(
        rx,
        engine_parse, component_parse, linker_parse, // moved → parse thread
        &engine_fmt, &component_fmt, &linker_fmt,    // borrowed → format loop
        cfg,
    )?;

    Ok(())
}
