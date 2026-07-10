use std::cell::Cell;

wit_bindgen::generate!({
    path: "wit",
    world: "parser-plugin",
    generate_all,
});

// generate! 之後，WIT 型別對應如下：
//   local::log_process::pipeline_process::{ParsedEntry, ParseError, LogLevel}
//   wasi::clocks::monotonic_clock
//   world-level import `time-sign` → 根模組的 time_sign()

// ParsedEntry / ParseError 因 world 的 use 已由 generate! 匯出到根模組
use local::log_process::pipeline_process::LogLevel;
use serde_json::Value;
use wasi::clocks::monotonic_clock;

// ── 全域狀態（WASM 單執行緒） ────────────────────────────────────────────

thread_local! {
    static LAST_EXEC_NS: Cell<u64> = Cell::new(0);
}

// ── 輔助函數 ────────────────────────────────────────────────────────────

/// 非字串值以 compact JSON 表示（同 cJSON_PrintUnformatted）
fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// 映射字串 Log Level 到系統定義的數值（同 C plugin 的 map_level）
fn map_level_str(s: &str) -> LogLevel {
    if s.eq_ignore_ascii_case("debug") {
        LogLevel::Debug
    } else if s.eq_ignore_ascii_case("info") {
        LogLevel::Info
    } else if s.eq_ignore_ascii_case("warn") {
        LogLevel::Warn
    } else if s.eq_ignore_ascii_case("error") {
        LogLevel::Error
    } else {
        LogLevel::Info
    }
}

fn map_level_num(n: u64) -> LogLevel {
    match n {
        0 => LogLevel::Debug,
        1 => LogLevel::Info,
        2 => LogLevel::Warn,
        3 => LogLevel::Error,
        4 => LogLevel::Crit,
        5 => LogLevel::Alert,
        6 => LogLevel::Emerg,
        _ => LogLevel::Info,
    }
}

/// 根據 log level 回傳 route 標籤（同 C plugin 的 route_tag_c）
fn route_tag(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Error | LogLevel::Crit | LogLevel::Alert | LogLevel::Emerg => "A",
        LogLevel::Warn => "A",
        _ => "A",
    }
}

/// 收集標籤：1. lang=Rust 2. 根層非保留欄位 3. "att" 物件內的所有欄位
fn collect_all_tags(root: &Value) -> Vec<(String, String)> {
    let mut tags = vec![("lang".to_string(), "Rust".to_string())];

    if let Some(map) = root.as_object() {
        for (key, val) in map {
            if key == "ts" || key == "level" || key == "msg" || key == "att" {
                continue;
            }
            tags.push((key.clone(), value_to_string(val)));
        }

        if let Some(att) = map.get("att").and_then(Value::as_object) {
            for (key, val) in att {
                tags.push((key.clone(), value_to_string(val)));
            }
        }
    }

    tags
}

/// 實際解析邏輯，由 parse 包一層以便在單一出口前後呼叫 time-sign
fn do_parse(raw_data: &[String]) -> Result<Vec<ParsedEntry>, ParseError> {
    let mut entries = Vec::with_capacity(raw_data.len());

    for raw in raw_data {
        let root: Value = match serde_json::from_str(raw) {
            Ok(v) => v,
            Err(_) => {
                return Err(ParseError::InvalidFormat("JSON Parse Error".to_string()));
            }
        };

        // 1. 解析 ts (timestamp)
        let timestamp = root
            .get("ts")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        // 2. 解析 level（數值直接映射，字串走 map_level）
        let level = match root.get("level") {
            Some(Value::Number(n)) => map_level_num(n.as_u64().unwrap_or(1)),
            Some(Value::String(s)) => map_level_str(s),
            _ => LogLevel::Info,
        };

        // 3. 解析 msg (message)
        let message = root
            .get("msg")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        // 4. 自動收集所有標籤（含根層非保留欄位與 att 物件）
        let tags = collect_all_tags(&root);

        // 5. 設定 route 標籤
        let targettag = route_tag(level).to_string();

        entries.push(ParsedEntry {
            timestamp,
            level,
            message,
            tags,
            targettag,
        });
    }

    Ok(entries)
}

// ── world 匯出實作 ──────────────────────────────────────────────────────

struct Component;

impl Guest for Component {
    fn parse(raw_data: Vec<String>) -> Result<Vec<ParsedEntry>, ParseError> {
        // 主函數開頭呼叫 host 匯入的 time-sign
        time_sign();

        let start_ns = monotonic_clock::now();
        let result = do_parse(&raw_data);
        LAST_EXEC_NS.set(monotonic_clock::now().saturating_sub(start_ns));

        // 主函數結尾（含錯誤路徑）再呼叫一次 time-sign
        time_sign();
        result
    }

    fn report_usage() -> u64 {
        LAST_EXEC_NS.get()
    }
}

export!(Component);
