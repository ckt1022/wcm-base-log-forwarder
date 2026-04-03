use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy)]
pub struct BatchConfig {
    pub mem_limit_mb: usize,
    pub safe_data_ratio: f64,
    pub max_wait: Duration,
    pub max_batch_lines: usize,
    pub channel_capacity: usize,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            mem_limit_mb: 256,
            safe_data_ratio: 0.3,
            max_wait: Duration::from_millis(250),
            max_batch_lines: 50_000,
            channel_capacity: 25_000,
        }
    }
}

pub struct LineItem {
    pub bytes: Vec<u8>,
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
