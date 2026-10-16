wit_bindgen::generate!({
    path: "../wit",
    world: "reduction-plugin",
});

use local::log_process::pipeline_process::LogLevel;

const MIN_KEEP_LEVEL: LogLevel = LogLevel::Warn;

struct FilterBenchmark;

impl Guest for FilterBenchmark {
    fn filter(struct_data: Vec<LogEntry>) -> Result<Vec<FilterResult>, PluginError> {
        // H1: Canonical ABI and generated bindings have reconstructed every
        // input LogEntry in component memory.
        time_sign();

        let results = struct_data
            .iter()
            .map(|entry| FilterResult {
                id: entry.id,
                keep: entry.level as u8 >= MIN_KEEP_LEVEL as u8,
            })
            .collect();

        // H2: all level decisions and FilterResult allocations are complete.
        // Returning from this function begins result lowering/copy-out.
        time_sign();
        Ok(results)
    }

    fn report_usage() -> u64 {
        0
    }
}

export!(FilterBenchmark);
