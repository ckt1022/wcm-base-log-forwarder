mod app;
mod config;
mod output;
mod runtime;

use config::BatchConfig;

struct PluginPath {
    parse : String,
    //enrich : String,
    //filter: String,
    //router: String,
    //format: String,
    //transport: String
}

fn main() -> wasmtime::Result<()> {
    // plugin path instance
    let path = PluginPath{
        parse:  String::from("/home/ckt1022/test-plugins/go-plugin/parser/parser.wasm")
    };
    // config instance
    let cfg = BatchConfig::default();
    let mem_limit_bytes = cfg.mem_limit_mb * 1024 * 1024;
    let safe_data_budget = (mem_limit_bytes as f64 * cfg.safe_data_ratio) as usize;

    output::print_startup(cfg, safe_data_budget);

    let (tx, rx) = std::sync::mpsc::sync_channel::<config::LineItem>(cfg.channel_capacity);
    app::spawn_stdin_reader(tx);

    let (engine, linker, component) = app::build_runtime_parse(mem_limit_bytes,path.parse)?;
    runtime::run_batch_loop(&rx, &engine, &component, &linker, cfg)?;

    Ok(())
}