// ── 內建原生 stage（no-WASM baseline）──────────────────────────────────────────
//
// forwarder.yaml 的 plugins.<stage> 留空或填 `native` 時，host 改用本檔的
// Rust 原生函式處理該 stage，不經過任何 WASM 邊界。邏輯逐行對照
// test-plugins/rust-plugin/{parse,filter,format,transport}：
//   - parse ：serde_json 解析 ts／level／msg，其餘欄位與 att 內欄位攤平成 tags，
//             第一個 tag 固定為 lang=Rust，targettag 一律 "A"
//   - filter：保留 level >= warn
//   - format：json-flat，輸出 [4-byte LE 長度][資料] frame（與 plugin 相同）
//   - transport：NDJSON body，以 HTTP/1.1 POST 到 endpoint，每個 POST 新建一條
//                TCP 連線（對齊 wasmtime-wasi-http 預設 send_request 的行為）
//
// 用途有二：
//   1. 系統層 baseline：同一個 binary、同一套 harness，只把 stage 從 WCM 換成原生，
//      量測「沙箱整體成本」。
//   2. 微基準 baseline：test/ours/baselines/ 以 #[path] 直接引用本檔，確保微基準
//      與系統層用的是同一份邏輯。
//
// 注意：原生 stage 沒有任何隔離——panic 會讓 host 執行緒崩潰、無記憶體上限、
// 無 CPU 時間上限、擁有 host process 的全部能力。這正是 baseline 要對照的差異。
//
// 與 WCM parser 不同，原生 parse 天生是「逐筆結果」：一筆 malformed 只會讓
// 該筆失敗，不需要 host 二分重試（對應 isolation.rs 提到的長期 WIT 設計）。

use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use serde_json::Value;

use crate::app::local::log_process::pipeline_process::{FilterResult, LogEntry, LogLevel, ParsedEntry};

// ── Parse ─────────────────────────────────────────────────────────────────────

/// 將 JSON 值轉為字串：字串直接取值，其餘型別輸出 compact JSON。
fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// 映射字串 Log Level（與 Rust plugin 相同：未知一律 info）。
fn map_level(level_str: Option<&str>) -> LogLevel {
    match level_str {
        Some(s) if s.eq_ignore_ascii_case("debug") => LogLevel::Debug,
        Some(s) if s.eq_ignore_ascii_case("info") => LogLevel::Info,
        Some(s) if s.eq_ignore_ascii_case("warn") => LogLevel::Warn,
        Some(s) if s.eq_ignore_ascii_case("error") => LogLevel::Error,
        _ => LogLevel::Info,
    }
}

/// 數值型 level 欄位的映射（syslog 七級）。
fn level_from_number(n: u64) -> LogLevel {
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

/// 根目錄非保留欄位 + att 內欄位攤平成 tags；第一個 tag 為 lang=Rust。
fn collect_all_tags(root: &Value) -> Vec<(String, String)> {
    let mut tags = vec![("lang".to_string(), "Rust".to_string())];
    if let Value::Object(map) = root {
        for (key, val) in map {
            if matches!(key.as_str(), "ts" | "level" | "msg" | "att") {
                continue;
            }
            tags.push((key.clone(), value_to_string(val)));
        }
        if let Some(Value::Object(att)) = map.get("att") {
            for (key, val) in att {
                tags.push((key.clone(), value_to_string(val)));
            }
        }
    }
    tags
}

/// 解析單筆原始 JSON 行。
pub fn parse_line(raw: &str) -> Result<ParsedEntry, String> {
    let root: Value = serde_json::from_str(raw).map_err(|e| format!("JSON Parse Error: {e}"))?;
    let timestamp = root.get("ts").and_then(Value::as_str).unwrap_or("").to_string();
    let level = match root.get("level") {
        Some(Value::Number(n)) => level_from_number(n.as_u64().unwrap_or(1)),
        lv => map_level(lv.and_then(Value::as_str)),
    };
    let message = root.get("msg").and_then(Value::as_str).unwrap_or("").to_string();
    let tags = collect_all_tags(&root);
    Ok(ParsedEntry {
        timestamp,
        level,
        message,
        tags,
        targettag: "A".to_string(),
    })
}

/// 逐筆解析：每筆各自成功或失敗（原生 stage 的 host 路徑使用）。
pub fn parse_batch(lines: &[String]) -> Vec<Result<ParsedEntry, String>> {
    lines.iter().map(|l| parse_line(l)).collect()
}

/// 與 WCM plugin 相同的「全有或全無」語意（微基準用，與 plugin 一對一比較）。
pub fn parse_batch_strict(lines: &[String]) -> Result<Vec<ParsedEntry>, String> {
    lines.iter().map(|l| parse_line(l)).collect()
}

// ── Filter ────────────────────────────────────────────────────────────────────

/// 保留 level >= warn（與 Rust filter plugin 的 MIN_KEEP_LEVEL 相同）。
pub fn filter_batch(entries: &[LogEntry]) -> Vec<FilterResult> {
    entries
        .iter()
        .map(|e| FilterResult {
            id: e.id,
            keep: e.level as u8 >= LogLevel::Warn as u8,
        })
        .collect()
}

// ── Format ────────────────────────────────────────────────────────────────────

fn level_name(level: LogLevel) -> &'static str {
    const NAMES: [&str; 7] = ["debug", "info", "warn", "error", "crit", "alert", "emerg"];
    NAMES.get(level as u8 as usize).copied().unwrap_or("unknown")
}

fn json_escape(out: &mut Vec<u8>, s: &str) {
    out.push(b'"');
    for &c in s.as_bytes() {
        match c {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            c if c < 0x20 => out.extend_from_slice(format!("\\u{c:04x}").as_bytes()),
            c => out.push(c),
        }
    }
    out.push(b'"');
}

fn write_frame(out: &mut Vec<u8>, entry: &[u8]) {
    out.extend_from_slice(&(entry.len() as u32).to_le_bytes());
    out.extend_from_slice(entry);
}

/// 單筆 json-flat 編碼（與 Rust format plugin 的 encode_entry 相同）。
fn encode_entry(out: &mut Vec<u8>, e: &LogEntry) {
    let mut entry = Vec::new();
    entry.extend_from_slice(b"{\"id\":");
    entry.extend_from_slice(e.id.to_string().as_bytes());
    entry.extend_from_slice(b",\"timestamp\":");
    json_escape(&mut entry, &e.timestamp);
    entry.extend_from_slice(b",\"level\":\"");
    entry.extend_from_slice(level_name(e.level).as_bytes());
    entry.extend_from_slice(b"\",\"message\":");
    json_escape(&mut entry, &e.message);
    for (key, val) in &e.tags {
        entry.push(b',');
        json_escape(&mut entry, key);
        entry.push(b':');
        json_escape(&mut entry, val);
    }
    entry.push(b'}');
    write_frame(out, &entry);
}

/// 整批 format：輸出 length-prefixed frame 串（與 plugin 的 list<u8> 相同）。
pub fn format_batch(entries: &[LogEntry]) -> Vec<u8> {
    let mut out = Vec::new();
    for e in entries {
        encode_entry(&mut out, e);
    }
    out
}

/// 解析 [4-byte LE 長度][資料] frame 串為輸出行（與 runtime.rs format_loop 相同規則）。
pub fn decode_frames(bytes: &[u8]) -> Result<Vec<Vec<u8>>, String> {
    let mut lines = Vec::new();
    let mut pos = 0;
    while pos + 4 <= bytes.len() {
        let len = u32::from_le_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        pos += 4;
        if pos + len > bytes.len() {
            return Err(format!("declared {} bytes but only {} remain", len, bytes.len() - pos));
        }
        lines.push(bytes[pos..pos + len].to_vec());
        pos += len;
    }
    Ok(lines)
}

// ── Transport ─────────────────────────────────────────────────────────────────

/// 解析後的 HTTP endpoint。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpEndpoint {
    pub host: String,
    pub port: u16,
    pub path: String,
}

/// 只支援 http://（原生 baseline 不含 TLS；實驗 sink 皆為 http）。
pub fn parse_http_endpoint(url: &str) -> Result<HttpEndpoint, String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| format!("native transport 只支援 http://：{url}"))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if authority.is_empty() {
        return Err(format!("endpoint 缺少 host：{url}"));
    }
    let (host, port) = match authority.rfind(':') {
        Some(i) => (
            &authority[..i],
            authority[i + 1..].parse::<u16>().map_err(|e| format!("port 錯誤：{e}"))?,
        ),
        None => (authority, 80),
    };
    Ok(HttpEndpoint { host: host.to_string(), port, path: path.to_string() })
}

/// 依 max_batch_bytes 切成數個 NDJSON body（與 transport plugin 的 build_batches 相同）。
pub fn build_batches(lines: &[Vec<u8>], max_batch_bytes: usize) -> Vec<Vec<u8>> {
    let mut batches = Vec::new();
    let mut cur: Vec<u8> = Vec::new();
    for entry in lines {
        if max_batch_bytes > 0 && !cur.is_empty() && cur.len() + entry.len() + 1 > max_batch_bytes {
            batches.push(std::mem::take(&mut cur));
        }
        cur.extend_from_slice(entry);
        if entry.last() != Some(&b'\n') {
            cur.push(b'\n');
        }
    }
    if !cur.is_empty() {
        batches.push(cur);
    }
    batches
}

/// 送出一個 HTTP/1.1 POST，回應 2xx 視為成功。每次新建連線並以 Connection: close 結束。
pub fn http_post(ep: &HttpEndpoint, body: &[u8], connect_timeout: Duration, io_timeout: Duration) -> Result<(), String> {
    let addr = (ep.host.as_str(), ep.port)
        .to_socket_addrs()
        .map_err(|e| format!("resolve {}:{}: {e}", ep.host, ep.port))?
        .next()
        .ok_or_else(|| format!("resolve {}:{}: no address", ep.host, ep.port))?;
    let mut stream = TcpStream::connect_timeout(&addr, connect_timeout).map_err(|e| format!("connect {addr}: {e}"))?;
    stream.set_read_timeout(Some(io_timeout)).ok();
    stream.set_write_timeout(Some(io_timeout)).ok();
    stream.set_nodelay(true).ok();

    let authority = if ep.port == 80 { ep.host.clone() } else { format!("{}:{}", ep.host, ep.port) };
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: {}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        ep.path,
        authority,
        body.len()
    );
    stream.write_all(head.as_bytes()).map_err(|e| format!("write header: {e}"))?;
    stream.write_all(body).map_err(|e| format!("write body: {e}"))?;

    let mut resp = Vec::with_capacity(256);
    stream.read_to_end(&mut resp).map_err(|e| format!("read response: {e}"))?;
    let status = parse_status_code(&resp).ok_or_else(|| "malformed HTTP response".to_string())?;
    if (200..300).contains(&status) {
        Ok(())
    } else {
        Err(format!("server returned HTTP {status}"))
    }
}

/// 從 "HTTP/1.1 200 OK\r\n..." 取出狀態碼。
fn parse_status_code(resp: &[u8]) -> Option<u16> {
    let line_end = resp.iter().position(|&b| b == b'\n')?;
    let line = std::str::from_utf8(&resp[..line_end]).ok()?;
    let mut parts = line.split_whitespace();
    let proto = parts.next()?;
    if !proto.starts_with("HTTP/") {
        return None;
    }
    parts.next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, level: LogLevel) -> LogEntry {
        LogEntry {
            id,
            timestamp: "2026-08-14T00:00:00Z".into(),
            level,
            message: "m\"q\\\n".into(),
            tags: vec![("lang".into(), "Rust".into()), ("k".into(), "v".into())],
        }
    }

    #[test]
    fn parse_matches_rust_plugin_semantics() {
        let raw = r#"{"ts":"2026-08-14T00:00:00Z","level":"warn","msg":"hello","host":"h1","n":3,"att":{"source":"s","b":true}}"#;
        let p = parse_line(raw).unwrap();
        assert_eq!(p.timestamp, "2026-08-14T00:00:00Z");
        assert_eq!(p.level, LogLevel::Warn);
        assert_eq!(p.message, "hello");
        assert_eq!(p.targettag, "A");
        assert_eq!(p.tags[0], ("lang".to_string(), "Rust".to_string()));
        assert!(p.tags.contains(&("host".to_string(), "h1".to_string())));
        assert!(p.tags.contains(&("n".to_string(), "3".to_string())));
        assert!(p.tags.contains(&("source".to_string(), "s".to_string())));
        assert!(p.tags.contains(&("b".to_string(), "true".to_string())));
        // 數值 level 與未知 level
        assert_eq!(parse_line(r#"{"level":3}"#).unwrap().level, LogLevel::Error);
        assert_eq!(parse_line(r#"{"level":"bogus"}"#).unwrap().level, LogLevel::Info);
    }

    #[test]
    fn parse_batch_is_per_record_but_strict_is_all_or_nothing() {
        let lines = vec![r#"{"level":"warn"}"#.to_string(), "{not json".to_string(), r#"{"level":"info"}"#.to_string()];
        let per = parse_batch(&lines);
        assert!(per[0].is_ok() && per[1].is_err() && per[2].is_ok());
        assert!(parse_batch_strict(&lines).is_err());
        assert_eq!(parse_batch_strict(&lines[..1]).unwrap().len(), 1);
    }

    #[test]
    fn filter_keeps_warn_and_above() {
        let es = vec![entry(1, LogLevel::Debug), entry(2, LogLevel::Info), entry(3, LogLevel::Warn), entry(4, LogLevel::Emerg)];
        let keep: Vec<(u64, bool)> = filter_batch(&es).into_iter().map(|r| (r.id, r.keep)).collect();
        assert_eq!(keep, vec![(1, false), (2, false), (3, true), (4, true)]);
    }

    #[test]
    fn format_roundtrips_frames_and_escapes() {
        let es = vec![entry(7, LogLevel::Error), entry(8, LogLevel::Warn)];
        let bytes = format_batch(&es);
        let lines = decode_frames(&bytes).unwrap();
        assert_eq!(lines.len(), 2);
        let v: Value = serde_json::from_slice(&lines[0]).unwrap();
        assert_eq!(v["id"], 7);
        assert_eq!(v["level"], "error");
        assert_eq!(v["message"], "m\"q\\\n");
        assert_eq!(v["k"], "v");
        // 截斷的 frame 必須回報錯誤
        assert!(decode_frames(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn endpoint_and_batches() {
        let ep = parse_http_endpoint("http://127.0.0.1:8080/ingest").unwrap();
        assert_eq!(ep, HttpEndpoint { host: "127.0.0.1".into(), port: 8080, path: "/ingest".into() });
        assert_eq!(parse_http_endpoint("http://example").unwrap().port, 80);
        assert!(parse_http_endpoint("https://x/").is_err());
        let lines = vec![vec![b'a'; 10], vec![b'b'; 10], vec![b'c'; 10]];
        let b = build_batches(&lines, 22);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].len(), 22);
        assert_eq!(build_batches(&lines, 0).len(), 1);
    }

    #[test]
    fn http_post_against_local_server() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 4096];
            let mut got = Vec::new();
            // 讀到 header 結束與 body（測試資料很小）
            while !got.windows(5).any(|w| w == b"\r\n\r\nx") {
                let n = s.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                got.extend_from_slice(&buf[..n]);
            }
            s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
            String::from_utf8_lossy(&got).to_string()
        });
        let ep = HttpEndpoint { host: "127.0.0.1".into(), port, path: "/ingest".into() };
        http_post(&ep, b"x\n", Duration::from_secs(2), Duration::from_secs(2)).unwrap();
        let req = server.join().unwrap();
        assert!(req.starts_with("POST /ingest HTTP/1.1\r\n"));
        assert!(req.contains("Content-Length: 2\r\n"));
        assert_eq!(parse_status_code(b"HTTP/1.1 503 Busy\r\n"), Some(503));
        assert_eq!(parse_status_code(b"garbage"), None);
    }
}
