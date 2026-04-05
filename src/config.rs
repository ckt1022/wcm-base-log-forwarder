use std::mem::size_of;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct BatchConfig {
    pub mem_limit_mb: usize,
    pub safe_data_ratio: f64,
    pub max_wait: Duration,
    pub max_batch_lines: usize,
    pub channel_capacity: usize,
    /// 每次呼叫 format plugin 的最大 entry 數。
    /// TinyGo GC 在大批次時無法及時回收中間字串緩衝區，需分批呼叫。
    pub max_format_chunk: usize,
}



impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            mem_limit_mb: 256,
            safe_data_ratio: 0.3,
            max_wait: Duration::from_millis(250),
            max_batch_lines: 20_000,
            channel_capacity: 5_000,
            max_format_chunk: 100,
        }
    }
}

pub struct LineItem {
    pub bytes: Vec<u8>,
}

impl LineItem {
    pub fn total_size_bytes(&self) -> usize {
        size_of::<LineItem>() + self.bytes.capacity() * size_of::<u8>()
    }
}

/// 追蹤 channel 目前積壓的條數與 byte 數。
/// Clone 後共享同一組 Atomic，可分別交給 sender thread 和 receiver thread。
#[derive(Clone)]
pub struct ChannelStats {
    count: Arc<AtomicUsize>,
    bytes: Arc<AtomicUsize>,
}

impl ChannelStats {
    pub fn new() -> Self {
        Self {
            count: Arc::new(AtomicUsize::new(0)),
            bytes: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// 送出一條後呼叫（send() 成功回傳後才呼叫，以反映真實 buffer 狀態）
    pub fn on_send(&self, byte_len: usize) {
        self.count.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(byte_len, Ordering::Relaxed);
    }

    /// 收到一條後立刻呼叫（取出時就減，反映已離開 buffer）
    pub fn on_recv(&self, byte_len: usize) {
        self.count.fetch_sub(1, Ordering::Relaxed);
        self.bytes.fetch_sub(byte_len, Ordering::Relaxed);
    }

    /// 回傳 (條數, byte 數) 快照
    pub fn snapshot(&self) -> (usize, usize) {
        (
            self.count.load(Ordering::Relaxed),
            self.bytes.load(Ordering::Relaxed),
        )
    }
}

pub struct Batch {
    pub lines: Vec<Vec<u8>>,
    pub bytes: usize,
    pub created_at: Instant,
}

impl Batch {
    pub fn new() -> Self {
        Self { lines: Vec::new(), bytes: 0, created_at: Instant::now() }
    }
    pub fn is_empty(&self) -> bool { self.lines.is_empty() }
    pub fn len(&self) -> usize { self.lines.len() }
    pub fn elapsed(&self) -> Duration { self.created_at.elapsed() }
    pub fn push(&mut self, line: Vec<u8>) {
        self.bytes += line.len();
        self.lines.push(line);
    }
    pub fn clear(&mut self) {
        self.lines.clear();
        self.bytes = 0;
        self.created_at = Instant::now();
    }
}

pub struct FlushReason {
    pub size: bool,
    pub time: bool,
    pub line_count: bool,
    pub eof: bool,
}

// ── 統計結構 ──────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct ParseStats {
    pub total_batches: u64,
    pub total_input_lines: u64,
    pub total_input_bytes: u64,
    pub total_output_entries: u64,
    pub total_elapsed: Duration,
    /// Go heap 峰值（各 batch 最大值）
    pub go_heap_peak_max: u64,
    /// WASM 線性記憶體峰值（各 batch 最大值）
    pub wasm_mem_peak_max: usize,
}

#[derive(Default)]
pub struct FormatStats {
    pub total_batches: u64,
    pub total_input_entries: u64,
    pub total_output_lines: u64,
    pub total_elapsed: Duration,
    /// WASM 線性記憶體峰值（各 batch 最大值）
    pub wasm_mem_peak_max: usize,
}
