//! Direct Memory ABI benchmark 的固定測試資料產生器。
//!
//! 本檔案建立可重現的 1024-byte JSON logs、衍生的 parsed/entry 結構，並以和 JSON ABI
//! control group 相同的邏輯 request 計算 fixture hash，確保兩組收到相同資料。

use core_wasm_binary_common::{LogEntry, ParsedEntry, log_entries_from_parsed, parse_records};

const JSON_BYTES: usize = 1024;

/// 建立恰好 1024 bytes、具有指定層級且格式有效的原始 JSON log。
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

/// 建立指定筆數、交錯 info/warn 的固定 logs 及各階段所需衍生資料。
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
    // Hash the same logical ParseRequest representation used by the JSON ABI
    // control group, so equal hashes prove that both ABIs received equal logs.
    let logical_request = serde_json::to_vec(&serde_json::json!({ "records": &raw }))
        .expect("serialize logical fixture hash input");
    Fixture {
        logical_input_bytes: raw.iter().map(String::len).sum(),
        hash: fnv1a(&logical_request),
        raw,
        parsed,
        entries,
    }
}

/// 計算資料的 64-bit FNV-1a hash，供跨 ABI fixture 與輸出一致性檢查。
pub fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
