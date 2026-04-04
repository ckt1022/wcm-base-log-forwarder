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

/// 控制 pipeline 中哪些處理階段要啟用。
/// 未來加入新元件時在此加入對應欄位即可。
pub struct PipelineStages {
    pub format: bool,
    //pub enrich: bool,
    //pub filter: bool,
    //pub router: bool,
    //pub transport: bool,
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

    let stages = PipelineStages {
        format: true,
    };

    let cfg = BatchConfig::default();
    let mem_limit_bytes = cfg.mem_limit_mb * 1024 * 1024;
    let safe_data_budget = (mem_limit_bytes as f64 * cfg.safe_data_ratio) as usize;

    output::print_startup(cfg, safe_data_budget, &stages);

    // channel存放的是LineItem物件，物件裡面只有一條vec<u8>(byte)
    let (tx, rx) = std::sync::mpsc::sync_channel::<config::LineItem>(cfg.channel_capacity);
    app::spawn_stdin_reader(tx);

    // Parse engine/component/linker 永遠需要
    let (engine_parse, linker_parse, component_parse) =
        app::build_runtime(mem_limit_bytes, path.parse)?;

    // Format engine/component/linker 只在啟用時建立
    let format_runtime = if stages.format {
        Some(app::build_runtime(mem_limit_bytes, path.format)?)
    } else {
        None
    };

    runtime::run_pipeline(
        rx,
        engine_parse, component_parse, linker_parse,
        format_runtime.as_ref().map(|(e, l, c)| (e, c, l)),
        cfg,
    )?;

    Ok(())
}
