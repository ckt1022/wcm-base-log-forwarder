//! Core Wasm JSON ABI guests 與 host 共用的資料模型和業務邏輯。
//!
//! 本檔案定義穩定的 JSON request/response schema，實作 parse、filter、format 參考邏輯，
//! 並提供計時標記與跨 Core Wasm ABI 傳回 buffer 所需的記憶體管理工具。

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
    Crit,
    Alert,
    Emerg,
}

impl LogLevel {
    /// 將日誌層級轉成可比較嚴重程度的數值排名。
    pub fn rank(self) -> u8 {
        match self {
            Self::Debug => 0,
            Self::Info => 1,
            Self::Warn => 2,
            Self::Error => 3,
            Self::Crit => 4,
            Self::Alert => 5,
            Self::Emerg => 6,
        }
    }

    /// 回傳 ABI 與格式化輸出使用的小寫日誌層級名稱。
    pub fn name(self) -> &'static str {
        const NAMES: [&str; 7] = ["debug", "info", "warn", "error", "crit", "alert", "emerg"];
        NAMES[usize::from(self.rank())]
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ParsedEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub message: String,
    pub tags: Vec<(String, String)>,
    pub targettag: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct LogEntry {
    pub id: u64,
    pub timestamp: String,
    pub level: LogLevel,
    pub message: String,
    pub tags: Vec<(String, String)>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FilterResult {
    pub id: u64,
    pub keep: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ParseRequest {
    pub records: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct EntriesRequest {
    pub entries: Vec<LogEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ParseOk {
    pub entries: Vec<ParsedEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FilterOk {
    pub results: Vec<FilterResult>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct FormatOk {
    pub framed_output_base64: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AbiError {
    pub kind: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(untagged)]
pub enum AbiResponse<T> {
    Ok { ok: T },
    Err { err: AbiError },
}

/// 將任意 JSON 值轉為 tag 字串；字串保留原值，其餘使用 JSON 表示。
pub fn value_to_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        other => other.to_string(),
    }
}

/// 將 JSON 的字串或數字層級映射成 `LogLevel`，未知值預設為 `Info`。
pub fn map_level(value: Option<&Value>) -> LogLevel {
    match value {
        Some(Value::Number(number)) => match number.as_u64().unwrap_or(1) {
            0 => LogLevel::Debug,
            1 => LogLevel::Info,
            2 => LogLevel::Warn,
            3 => LogLevel::Error,
            4 => LogLevel::Crit,
            5 => LogLevel::Alert,
            6 => LogLevel::Emerg,
            _ => LogLevel::Info,
        },
        Some(Value::String(level)) if level.eq_ignore_ascii_case("debug") => LogLevel::Debug,
        Some(Value::String(level)) if level.eq_ignore_ascii_case("warn") => LogLevel::Warn,
        Some(Value::String(level)) if level.eq_ignore_ascii_case("error") => LogLevel::Error,
        Some(Value::String(level)) if level.eq_ignore_ascii_case("crit") => LogLevel::Crit,
        Some(Value::String(level)) if level.eq_ignore_ascii_case("alert") => LogLevel::Alert,
        Some(Value::String(level)) if level.eq_ignore_ascii_case("emerg") => LogLevel::Emerg,
        _ => LogLevel::Info,
    }
}

/// 從原始 JSON 物件收集固定的語言 tag、額外欄位及 `att` 內的屬性。
pub fn collect_tags(root: &Value) -> Vec<(String, String)> {
    let mut tags = vec![("lang".to_string(), "Rust".to_string())];
    if let Value::Object(fields) = root {
        for (key, value) in fields {
            if !matches!(key.as_str(), "ts" | "level" | "msg" | "att") {
                tags.push((key.clone(), value_to_string(value)));
            }
        }
        if let Some(Value::Object(attributes)) = fields.get("att") {
            for (key, value) in attributes {
                tags.push((key.clone(), value_to_string(value)));
            }
        }
    }
    tags
}

/// 解析每筆原始 JSON log，產生與 WCM parse component 相同的結構化結果。
pub fn parse_records(raw_data: &[String]) -> Result<Vec<ParsedEntry>, AbiError> {
    let mut entries = Vec::with_capacity(raw_data.len());
    for raw in raw_data {
        let root: Value = serde_json::from_str(raw).map_err(|_| AbiError {
            kind: "invalid-format".to_string(),
            message: "JSON Parse Error".to_string(),
        })?;
        let timestamp = root
            .get("ts")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let level = map_level(root.get("level"));
        let message = root
            .get("msg")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let tags = collect_tags(&root);
        entries.push(ParsedEntry {
            timestamp,
            level,
            message,
            tags,
            targettag: "A".to_string(),
        });
    }
    Ok(entries)
}

/// 依日誌層級是否至少為 `Warn`，為每筆 entry 產生保留判定。
pub fn filter_entries(entries: &[LogEntry]) -> Vec<FilterResult> {
    entries
        .iter()
        .map(|entry| FilterResult {
            id: entry.id,
            keep: entry.level.rank() >= LogLevel::Warn.rank(),
        })
        .collect()
}

/// 將字串以 JSON 規則跳脫後附加到輸出 buffer。
fn json_escape(out: &mut Vec<u8>, value: &str) {
    out.push(b'"');
    for &byte in value.as_bytes() {
        match byte {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            byte if byte < 0x20 => {
                out.extend_from_slice(format!("\\u{byte:04x}").as_bytes());
            }
            byte => out.push(byte),
        }
    }
    out.push(b'"');
}

/// 將一筆資料包裝成「4-byte little-endian 長度 + body」frame。
fn write_frame(out: &mut Vec<u8>, entry: &[u8]) {
    out.extend_from_slice(&(entry.len() as u32).to_le_bytes());
    out.extend_from_slice(entry);
}

/// 將 entries 格式化成扁平 JSON，並以 length-prefixed frames 串接輸出。
pub fn format_entries(entries: &[LogEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    for log in entries {
        let mut entry = Vec::new();
        entry.extend_from_slice(b"{\"id\":");
        entry.extend_from_slice(log.id.to_string().as_bytes());
        entry.extend_from_slice(b",\"timestamp\":");
        json_escape(&mut entry, &log.timestamp);
        entry.extend_from_slice(b",\"level\":\"");
        entry.extend_from_slice(log.level.name().as_bytes());
        entry.extend_from_slice(b"\",\"message\":");
        json_escape(&mut entry, &log.message);
        for (key, value) in &log.tags {
            entry.push(b',');
            json_escape(&mut entry, key);
            entry.push(b':');
            json_escape(&mut entry, value);
        }
        entry.push(b'}');
        write_frame(&mut out, &entry);
    }
    out
}

/// 將 parse 結果依順序配置 ID，轉成 filter 與 format 使用的 `LogEntry`。
pub fn log_entries_from_parsed(entries: &[ParsedEntry]) -> Vec<LogEntry> {
    entries
        .iter()
        .enumerate()
        .map(|(index, entry)| LogEntry {
            id: index as u64,
            timestamp: entry.timestamp.clone(),
            level: entry.level,
            message: entry.message.clone(),
            tags: entry.tags.clone(),
        })
        .collect()
}

#[cfg(target_arch = "wasm32")]
#[link(wasm_import_module = "bench")]
unsafe extern "C" {
    /// 呼叫 host 提供的 `bench::time-sign`，記錄 H1 或 H2 邊界。
    #[link_name = "time-sign"]
    fn imported_time_sign();
}

#[cfg(target_arch = "wasm32")]
/// 通知 host 記錄目前的 benchmark 時間邊界。
pub fn time_sign() {
    unsafe { imported_time_sign() };
}

/// 將輸出 buffer 的所有權交給 host，並把指標與長度封裝在一個 `u64` 中。
pub fn leak_bytes(bytes: Vec<u8>) -> u64 {
    let output = bytes.into_boxed_slice();
    let len = output.len() as u32;
    let ptr = Box::into_raw(output) as *mut u8 as u32;
    (u64::from(ptr) << 32) | u64::from(len)
}

/// 依 guest 指標與長度重建配置並釋放 ABI buffer。
///
/// # Safety
///
/// `ptr` 與 `len` 必須來自尚未釋放的 guest buffer，且只能釋放一次。
pub unsafe fn dealloc_bytes(ptr: u32, len: u32) {
    if len == 0 {
        return;
    }
    let slice = std::ptr::slice_from_raw_parts_mut(ptr as *mut u8, len as usize);
    unsafe { drop(Box::from_raw(slice)) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建立供 JSON schema 與 frame golden tests 共用的固定 entry。
    fn sample_entry() -> LogEntry {
        LogEntry {
            id: 7,
            timestamp: "2026-08-14T00:00:00Z".to_string(),
            level: LogLevel::Warn,
            message: "hello".to_string(),
            tags: vec![("lang".to_string(), "Rust".to_string())],
        }
    }

    #[test]
    /// 確認 parse 與 entries request 的 JSON 欄位及序列化格式維持穩定。
    fn request_json_schema_is_stable() {
        let parse = serde_json::to_string(&ParseRequest {
            records: vec!["{}".to_string()],
        })
        .unwrap();
        assert_eq!(parse, r#"{"records":["{}"]}"#);

        let entries = serde_json::to_string(&EntriesRequest {
            entries: vec![sample_entry()],
        })
        .unwrap();
        assert_eq!(
            entries,
            r#"{"entries":[{"id":7,"timestamp":"2026-08-14T00:00:00Z","level":"warn","message":"hello","tags":[["lang","Rust"]]}]}"#
        );
    }

    #[test]
    /// 確認 ABI 成功回應的 JSON envelope 與 filter result schema 維持穩定。
    fn response_json_schema_is_stable() {
        let response = AbiResponse::Ok {
            ok: FilterOk {
                results: vec![FilterResult { id: 7, keep: true }],
            },
        };
        assert_eq!(
            serde_json::to_string(&response).unwrap(),
            r#"{"ok":{"results":[{"id":7,"keep":true}]}}"#
        );
    }

    #[test]
    /// 確認 format 的扁平 JSON body 與 little-endian frame header 維持穩定。
    fn flat_json_frame_is_stable() {
        let output = format_entries(&[sample_entry()]);
        let body = br#"{"id":7,"timestamp":"2026-08-14T00:00:00Z","level":"warn","message":"hello","lang":"Rust"}"#;
        assert_eq!(&output[..4], &(body.len() as u32).to_le_bytes());
        assert_eq!(&output[4..], body);
    }
}
