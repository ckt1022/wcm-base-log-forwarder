// 微基準共用：輸入 fixture、跨 process 時鐘、subprocess 的 length-prefixed 二進位 framing。
//
// ── Framing 格式（全部 little-endian）──────────────────────────────────────────
// str        = u32 長度 + UTF-8 bytes
// request    = u8 op | u32 payload_len | payload
//              op: 1=parse 2=filter 3=format 0x7F=ping 0=quit
//   parse  payload = u32 n | n × str(line)
//   filter/format payload = u32 n | n × entry
//   entry  = u64 id | u8 level | str timestamp | str message | u32 ntags | ntags × (str key, str value)
// response   = u64 h1_ns | u64 h2_ns | u8 status(0=ok,1=error) | u32 payload_len | payload
//   parse  ok payload = u32 n | n × parsed
//   parsed = str timestamp | u8 level | str message | u32 ntags | tags | str targettag
//   filter ok payload = u32 n | n × (u64 id | u8 keep)
//   format ok payload = frame 串（[u32 長度][資料] 重複，與 WCM format plugin 輸出相同）
//   error  payload = str 錯誤訊息
// h1／h2 是 child 端 CLOCK_MONOTONIC 奈秒（Linux 上跨 process 共用同一時間軸）：
//   h1 = child 讀完並解碼成 owned 結構之後，h2 = 業務邏輯完成、開始編碼回應之前。

use std::io::{self, Read, Write};

use crate::app::local::log_process::pipeline_process::{FilterResult, LogEntry, LogLevel, ParsedEntry};

pub const OP_QUIT: u8 = 0;
pub const OP_PARSE: u8 = 1;
pub const OP_FILTER: u8 = 2;
pub const OP_FORMAT: u8 = 3;
pub const OP_PING: u8 = 0x7F;

/// CLOCK_MONOTONIC（奈秒），host 與 child 共用時間軸。
pub fn now_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    unsafe {
        libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts);
    }
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

/// 與 copy_overhead/src_pipeline_benchmark/generate_input.js 相同的 1024 B fixture。
pub fn fixture(level: &str) -> String {
    let prefix = format!("{{\"ts\":\"2026-08-14T00:00:00Z\",\"level\":\"{level}\",\"msg\":\"");
    let suffix = "\",\"att\":{\"source\":\"parse-benchmark\",\"sequence\":\"fixed\"}}";
    let pad = 1024 - prefix.len() - suffix.len();
    let s = format!("{prefix}{}{suffix}", "x".repeat(pad));
    assert_eq!(s.len(), 1024);
    s
}

/// n 筆輸入：偶數 info、奇數 warn（與 generate_input.js 相同）。
pub fn input_lines(n: usize) -> Vec<String> {
    let info = fixture("info");
    let warn = fixture("warn");
    (0..n).map(|i| if i % 2 == 0 { info.clone() } else { warn.clone() }).collect()
}

/// filter／format 的輸入：以原生 parse 產生 LogEntry（id = index），對應 WCM 實驗中
/// parse 輸出 → host 分配 id 的路徑。
pub fn input_entries(n: usize) -> Vec<LogEntry> {
    input_lines(n)
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let p = crate::native::parse_line(l).expect("fixture must parse");
            LogEntry { id: i as u64, timestamp: p.timestamp, level: p.level, message: p.message, tags: p.tags }
        })
        .collect()
}

// ── 編碼 ─────────────────────────────────────────────────────────────────────

pub fn put_u32(b: &mut Vec<u8>, v: u32) {
    b.extend_from_slice(&v.to_le_bytes());
}
pub fn put_u64(b: &mut Vec<u8>, v: u64) {
    b.extend_from_slice(&v.to_le_bytes());
}
pub fn put_str(b: &mut Vec<u8>, s: &str) {
    put_u32(b, s.len() as u32);
    b.extend_from_slice(s.as_bytes());
}
fn put_tags(b: &mut Vec<u8>, tags: &[(String, String)]) {
    put_u32(b, tags.len() as u32);
    for (k, v) in tags {
        put_str(b, k);
        put_str(b, v);
    }
}

pub fn encode_lines(lines: &[String]) -> Vec<u8> {
    let cap: usize = 4 + lines.iter().map(|l| 4 + l.len()).sum::<usize>();
    let mut b = Vec::with_capacity(cap);
    put_u32(&mut b, lines.len() as u32);
    for l in lines {
        put_str(&mut b, l);
    }
    b
}

pub fn encode_entries(entries: &[LogEntry]) -> Vec<u8> {
    let mut b = Vec::new();
    put_u32(&mut b, entries.len() as u32);
    for e in entries {
        put_u64(&mut b, e.id);
        b.push(e.level as u8);
        put_str(&mut b, &e.timestamp);
        put_str(&mut b, &e.message);
        put_tags(&mut b, &e.tags);
    }
    b
}

pub fn encode_parsed(ps: &[ParsedEntry]) -> Vec<u8> {
    let mut b = Vec::new();
    put_u32(&mut b, ps.len() as u32);
    for p in ps {
        put_str(&mut b, &p.timestamp);
        b.push(p.level as u8);
        put_str(&mut b, &p.message);
        put_tags(&mut b, &p.tags);
        put_str(&mut b, &p.targettag);
    }
    b
}

pub fn encode_filter_results(rs: &[FilterResult]) -> Vec<u8> {
    let mut b = Vec::with_capacity(4 + rs.len() * 9);
    put_u32(&mut b, rs.len() as u32);
    for r in rs {
        put_u64(&mut b, r.id);
        b.push(r.keep as u8);
    }
    b
}

// ── 解碼 ─────────────────────────────────────────────────────────────────────

pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> &'a [u8] {
        let s = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        s
    }
    pub fn u8(&mut self) -> u8 {
        self.take(1)[0]
    }
    pub fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take(4).try_into().unwrap())
    }
    pub fn u64(&mut self) -> u64 {
        u64::from_le_bytes(self.take(8).try_into().unwrap())
    }
    pub fn string(&mut self) -> String {
        let n = self.u32() as usize;
        String::from_utf8(self.take(n).to_vec()).expect("utf8")
    }
    fn tags(&mut self) -> Vec<(String, String)> {
        let n = self.u32() as usize;
        (0..n).map(|_| (self.string(), self.string())).collect()
    }
}

pub fn decode_lines(buf: &[u8]) -> Vec<String> {
    let mut r = Reader::new(buf);
    let n = r.u32() as usize;
    (0..n).map(|_| r.string()).collect()
}

pub fn decode_entries(buf: &[u8]) -> Vec<LogEntry> {
    let mut r = Reader::new(buf);
    let n = r.u32() as usize;
    (0..n)
        .map(|_| {
            let id = r.u64();
            let level = LogLevel::from_u8(r.u8());
            let timestamp = r.string();
            let message = r.string();
            let tags = r.tags();
            LogEntry { id, timestamp, level, message, tags }
        })
        .collect()
}

pub fn decode_parsed(buf: &[u8]) -> Vec<ParsedEntry> {
    let mut r = Reader::new(buf);
    let n = r.u32() as usize;
    (0..n)
        .map(|_| {
            let timestamp = r.string();
            let level = LogLevel::from_u8(r.u8());
            let message = r.string();
            let tags = r.tags();
            let targettag = r.string();
            ParsedEntry { timestamp, level, message, tags, targettag }
        })
        .collect()
}

pub fn decode_filter_results(buf: &[u8]) -> Vec<FilterResult> {
    let mut r = Reader::new(buf);
    let n = r.u32() as usize;
    (0..n).map(|_| FilterResult { id: r.u64(), keep: r.u8() != 0 }).collect()
}

// ── 訊息 I/O ─────────────────────────────────────────────────────────────────

pub fn write_request<W: Write>(w: &mut W, op: u8, payload: &[u8]) -> io::Result<()> {
    let mut head = [0u8; 5];
    head[0] = op;
    head[1..5].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    w.write_all(&head)?;
    w.write_all(payload)?;
    w.flush()
}

/// 讀一個 request；EOF 時回傳 None。
pub fn read_request<R: Read>(r: &mut R) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut head = [0u8; 5];
    match r.read_exact(&mut head) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_le_bytes(head[1..5].try_into().unwrap()) as usize;
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    Ok(Some((head[0], payload)))
}

pub fn write_response<W: Write>(w: &mut W, h1: u64, h2: u64, status: u8, payload: &[u8]) -> io::Result<()> {
    let mut head = [0u8; 21];
    head[0..8].copy_from_slice(&h1.to_le_bytes());
    head[8..16].copy_from_slice(&h2.to_le_bytes());
    head[16] = status;
    head[17..21].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    w.write_all(&head)?;
    w.write_all(payload)?;
    w.flush()
}

pub struct Response {
    pub h1: u64,
    pub h2: u64,
    pub status: u8,
    pub payload: Vec<u8>,
}

pub fn read_response<R: Read>(r: &mut R) -> io::Result<Response> {
    let mut head = [0u8; 21];
    r.read_exact(&mut head)?;
    let len = u32::from_le_bytes(head[17..21].try_into().unwrap()) as usize;
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    Ok(Response {
        h1: u64::from_le_bytes(head[0..8].try_into().unwrap()),
        h2: u64::from_le_bytes(head[8..16].try_into().unwrap()),
        status: head[16],
        payload,
    })
}
