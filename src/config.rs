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
            mem_limit_mb: 64,
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
        Self {
            lines: Vec::new(),
            bytes: 0,
            created_at: Instant::now(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn elapsed(&self) -> Duration {
        self.created_at.elapsed()
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.bytes = 0;
        self.created_at = Instant::now();
    }

    pub fn push(&mut self, line: Vec<u8>) {
        self.bytes += line.len();
        self.lines.push(line);
    }
}

pub struct FlushReason {
    pub size: bool,
    pub time: bool,
    pub line_count: bool,
    pub eof: bool,
}

pub struct BatchReport {
    pub batch_seq: u64,
    pub input_lines: usize,
    pub input_bytes: usize,
    pub output_lines: usize,
    /// Go runtime heap 峰值（HeapInuse），由 plugin 內 ReportUsage() 回傳。
    /// 範圍：僅 Go heap，不含 goroutine stacks 等，< 實際 WASM 線性記憶體。
    pub go_heap_peak: u64,
    /// WASM 線性記憶體峰值，由 host 端 MyLimiter 在 memory.grow 時追蹤。
    /// 範圍：完整 WASM 線性記憶體（含 Go runtime 所有開銷），最準確的記憶體指標。
    pub wasm_linear_mem_peak: usize,
    pub mem_limit_bytes: usize,
    pub elapsed: Duration,
}