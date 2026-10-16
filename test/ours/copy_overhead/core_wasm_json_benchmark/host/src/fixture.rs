//! Core Wasm JSON ABI 基準測試的固定測試資料產生器。
//!
//! 本檔案建立長度固定且內容可重現的 JSON 記錄，衍生各測試階段需要的資料結構，
//! 並提供 FNV-1a 雜湊以確認不同執行批次使用相同輸入。

use core_wasm_json_common::{
    LogEntry, ParseRequest, ParsedEntry, log_entries_from_parsed, parse_records,
};

const JSON_BYTES: usize = 1024;

/// 建立一筆恰好 1024 位元組、具有指定日誌層級且可被正確解析的 JSON 字串。
fn make_json_1024(level: &str) -> String {
    let prefix = format!("{{\"ts\":\"2026-08-14T00:00:00Z\",\"level\":\"{level}\",\"msg\":\"");
    const SUFFIX: &str = "\",\"att\":{\"source\":\"parse-benchmark\",\"sequence\":\"fixed\"}}";
    let padding = JSON_BYTES
        .checked_sub(prefix.len() + SUFFIX.len())
        .expect("JSON framing exceeds 1024 bytes");
    let json = format!("{prefix}{}{SUFFIX}", "x".repeat(padding));
    assert_eq!(json.len(), JSON_BYTES);
    serde_json::from_str::<serde_json::Value>(&json).expect("fixture must be valid JSON");
    json
}

pub struct Fixture {
    pub raw: Vec<String>,
    pub parsed: Vec<ParsedEntry>,
    pub entries: Vec<LogEntry>,
    pub logical_input_bytes: usize,
    pub hash: u64,
}

/// 依批次大小建立交錯的 info/warn 記錄，以及解析、過濾和格式化階段所需的衍生資料。
pub fn make_fixture(batch: usize) -> Fixture {
    let info = make_json_1024("info");
    let warn = make_json_1024("warn");
    let raw: Vec<String> = (0..batch)
        .map(|index| {
            if index % 2 == 0 {
                info.clone()
            } else {
                warn.clone()
            }
        })
        .collect();
    let parsed = parse_records(&raw).expect("deterministic fixture must parse");
    let entries = log_entries_from_parsed(&parsed);
    let request = serde_json::to_vec(&ParseRequest {
        records: raw.clone(),
    })
    .expect("serialize fixture hash input");
    Fixture {
        logical_input_bytes: raw.iter().map(String::len).sum(),
        hash: fnv1a(&request),
        raw,
        parsed,
        entries,
    }
}

/// 計算位元組資料的 64 位元 FNV-1a 雜湊，供輸入與輸出一致性檢查使用。
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
