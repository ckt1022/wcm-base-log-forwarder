use std::cell::Cell;

wit_bindgen::generate!({
    path: "wit",                 // 指向 wit/ 目錄
    world: "reduction-plugin",   // copy_overhead.wit 中的 world 名稱
    generate_all,                // 一併產生 wasi:clocks 等 import 的 bindings
});

// generate! 之後，WIT 型別對應如下：
//   LogEntry / FilterResult / PluginError 因 world 的 use 已匯出到根模組
//   log-level → local::log_process::pipeline_process::LogLevel
//   world-level import `time-sign` → 根模組的 time_sign()

use local::log_process::pipeline_process::LogLevel;
use wasi::clocks::monotonic_clock;

// ── 全域狀態（WASM 單執行緒） ────────────────────────────────────────────

thread_local! {
    static LAST_EXEC_NS: Cell<u64> = Cell::new(0);
}

/// 保留 level >= warn 的 log（丟棄 debug / info），同 C plugin 的 MIN_KEEP_LEVEL
const MIN_KEEP_LEVEL: LogLevel = LogLevel::Warn;

/// 實際過濾邏輯，由 filter 包一層以便在單一出口前後呼叫 time-sign
fn do_filter(struct_data: &[LogEntry]) -> Result<Vec<FilterResult>, PluginError> {
    Ok(struct_data
        .iter()
        .map(|entry| FilterResult {
            id: entry.id,
            keep: entry.level as u8 >= MIN_KEEP_LEVEL as u8,
        })
        .collect())
}

// ── world 匯出實作 ──────────────────────────────────────────────────────

struct Component;

impl Guest for Component {
    fn filter(struct_data: Vec<LogEntry>) -> Result<Vec<FilterResult>, PluginError> {
        // 主函數開頭呼叫 host 匯入的 time-sign
        time_sign();

        let start_ns = monotonic_clock::now();
        let result = do_filter(&struct_data);
        LAST_EXEC_NS.set(monotonic_clock::now().saturating_sub(start_ns));

        // 主函數結尾再呼叫一次 time-sign
        time_sign();
        result
    }

    fn report_usage() -> u64 {
        LAST_EXEC_NS.get()
    }
}

export!(Component);
