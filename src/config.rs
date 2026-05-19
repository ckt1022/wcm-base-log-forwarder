use std::mem::size_of;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct BatchConfig {
    pub mem_limit_mb: usize,
    pub safe_data_ratio: f64,
    pub max_wait: Duration,
    pub max_batch_lines: usize,
    pub channel_capacity: usize,
    /// 每次呼叫 format plugin 的最大 entry 數。
    /// TinyGo GC 在大批次時無法及時回收中間字串緩衝區，需分批呼叫。
    pub max_format_chunk: usize,
    /// Transport plugin 目標端點 URL。
    pub transport_endpoint: String,
    /// 每次呼叫 transport send() 的最大累積 byte 數（一次對應一個 HTTP POST）。
    /// WASI blocking-write-and-flush 單次寫入上限為 4096 B（sync/async 皆適用）；
    /// plugin 必須在內部分批寫入 HTTP body（每次 ≤ 4096 B）。
    /// 此值控制每次 send() 傳入的資料總量，預設 32 KB，確保 plugin 寫入安全。
    /// 若 plugin 已正確實作分批寫入，可大幅調高（例如 256 KB）以減少 POST 次數。
    pub max_transport_bytes: usize,
    /// 平行 transport worker 數量。每個 worker 持有獨立 WASM store，
    /// 共用同一個 FormattedBatch channel，允許同時進行多個 HTTP POST。
    pub transport_workers: usize,
}

impl Default for BatchConfig {
    fn default() -> Self {
        Self {
            mem_limit_mb: 256,
            safe_data_ratio: 0.5,
            max_wait: Duration::from_millis(5000),
            max_batch_lines: 50_000,
            channel_capacity: 150_000,
            max_format_chunk: 50_000,
            transport_endpoint: String::from("http://127.0.0.1:8080/ingest"),
            max_transport_bytes: 128 * 1024,
            transport_workers: 5,
        }
    }
}

impl BatchConfig {
    /// 驗證所有參數是否合理，並說明每個欄位實際影響哪條程式碼路徑。
    ///
    /// 回傳 `Err(說明)` 若有明顯錯誤設定，否則印出每個參數的使用狀況後回傳 `Ok(())`。
    pub fn validate_and_describe(&self) -> Result<(), String> {
        // ── 範圍驗證 ──────────────────────────────────────────────────────
        // 每個斷言後面的 "(used at ...)" 說明這個值真正被消費的地方，
        // 方便日後確認某個欄位是否還有在用。

        // mem_limit_mb
        // used at: runtime.rs run_pipeline → mem_limit_bytes = mem_limit_mb * 1024 * 1024
        //          → MyLimiter::new(mem_limit_bytes) 限制每個 WASM store 的線性記憶體上限
        // BUG: main.rs:44 用了 cfg.mem_limit_mb 而非 * 1024*1024，safe_data_budget 計算錯誤
        if self.mem_limit_mb < 64 {
            return Err(format!(
                "mem_limit_mb={} 過小（建議 >= 64 MB）",
                self.mem_limit_mb
            ));
        }

        // safe_data_ratio
        // used at: runtime.rs parse_loop → safe_data_budget = mem_limit_bytes * ratio
        //          → 當 batch 累積 bytes > safe_data_budget 時觸發提前 flush（size trigger）
        // 注意: main.rs 也計算了 safe_data_budget 但因 mem_limit_bytes bug 導致實際為 179 bytes，
        //       該值只用於 print_startup，不影響 parse_loop 的判斷。
        if !(0.0 < self.safe_data_ratio && self.safe_data_ratio < 1.0) {
            return Err(format!(
                "safe_data_ratio={} 必須在 (0.0, 1.0) 之間",
                self.safe_data_ratio
            ));
        }

        // max_wait
        // used at: runtime.rs parse_loop → rx.recv_timeout(max_wait) 的 timeout 值
        //          → 觸發 time-based flush（即使 batch 未滿也會送出）
        if self.max_wait.is_zero() {
            return Err("max_wait 不能為 0（parse loop 會無限 spin）".to_string());
        }

        // max_batch_lines
        // used at: runtime.rs parse_loop → batch.len() >= max_batch_lines 時觸發 line-count flush
        //          → 限制單次傳入 WASM parser 的最大行數，防止 WASM 堆積太多資料
        if self.max_batch_lines == 0 {
            return Err("max_batch_lines 不能為 0".to_string());
        }

        // channel_capacity
        // used at: main.rs → sync_channel::<LineItem>(channel_capacity)
        //          → 只控制 stdin→parse 的 LineItem channel 容量
        // 警告: ParsedBatch channel (parse→format) 和 FormattedBatch channel (format→transport)
        //       目前在 runtime.rs:56-57 硬編碼為 20_000，不受此參數控制。
        if self.channel_capacity < self.max_batch_lines {
            eprintln!(
                "[config-warn] channel_capacity={} < max_batch_lines={}，\
                 stdin channel 可能在單個 batch 積滿前就阻塞",
                self.channel_capacity, self.max_batch_lines
            );
        }

        // max_format_chunk
        // used at: runtime.rs format_loop → pb.entries.chunks(max_format_chunk)
        //          → 將大批次拆成小塊，每塊各自建立新 WASM store 呼叫 format plugin
        //          → 目的：避免 TinyGo GC 在大批次時無法回收中間字串，導致 WASM OOM
        if self.max_format_chunk == 0 {
            return Err("max_format_chunk 不能為 0".to_string());
        }
        if self.max_format_chunk > self.max_batch_lines {
            eprintln!(
                "[config-warn] max_format_chunk={} > max_batch_lines={}，\
                 format 拆塊無實際效果（每批只有一塊）",
                self.max_format_chunk, self.max_batch_lines
            );
        }

        // transport_endpoint
        // used at: runtime.rs transport_worker → make_transport_config_val(endpoint)
        //          → 傳入 transport WASM plugin 的 init() 作為目標 URL
        if !self.transport_endpoint.starts_with("http://")
            && !self.transport_endpoint.starts_with("https://")
        {
            return Err(format!(
                "transport_endpoint='{}' 不是合法的 HTTP/HTTPS URL",
                self.transport_endpoint
            ));
        }

        // max_transport_bytes
        // used at: runtime.rs transport_worker → while acc_bytes + line_len <= max_transport_bytes
        //          → 控制每次呼叫 transport plugin send() 的最大 byte 數
        //          → 每次 send() 對應一個 HTTP POST
        // 注意: transport WASM plugin 內部仍須以 ≤ 4096 B 分批呼叫 blocking_write_and_flush
        if self.max_transport_bytes < 4096 {
            return Err(format!(
                "max_transport_bytes={} 過小（至少需 4096 B，等於一次 WASI write）",
                self.max_transport_bytes
            ));
        }

        // transport_workers
        // used at: runtime.rs run_pipeline → for i in 0..transport_workers { spawn worker }
        //          → 每個 worker 持有獨立 WASM store，共用 rx_formatted channel（Mutex 保護）
        //          → 允許同時進行多個 HTTP POST
        if self.transport_workers == 0 {
            return Err("transport_workers 不能為 0".to_string());
        }

        Ok(())
    }

    /// 以表格形式印出每個參數的值、用途、以及是否為預設值。
    pub fn print_config_table(&self) {
        let default = BatchConfig::default();
        let rows: &[(&str, String, &str, bool)] = &[
            (
                "mem_limit_mb",
                format!("{} MB", self.mem_limit_mb),
                "每個 WASM store 線性記憶體上限 (runtime.rs MyLimiter)",
                self.mem_limit_mb == default.mem_limit_mb,
            ),
            (
                "safe_data_ratio",
                format!("{:.0}%", self.safe_data_ratio * 100.0),
                "parse batch size 觸發提前 flush 的閾值 (runtime.rs parse_loop)",
                self.safe_data_ratio == default.safe_data_ratio,
            ),
            (
                "max_wait",
                format!("{} ms", self.max_wait.as_millis()),
                "parse 時間觸發 flush 的等待上限 (runtime.rs parse_loop recv_timeout)",
                self.max_wait == default.max_wait,
            ),
            (
                "max_batch_lines",
                format!("{}", self.max_batch_lines),
                "parse 行數觸發 flush 的上限 (runtime.rs parse_loop)",
                self.max_batch_lines == default.max_batch_lines,
            ),
            (
                "channel_capacity",
                format!("{}", self.channel_capacity),
                "stdin→parse LineItem channel 容量 [注意: parsed/formatted channel 硬編碼 20000]",
                self.channel_capacity == default.channel_capacity,
            ),
            (
                "max_format_chunk",
                format!("{}", self.max_format_chunk),
                "format plugin 每次呼叫的最大 entry 數 (runtime.rs format_loop chunks)",
                self.max_format_chunk == default.max_format_chunk,
            ),
            (
                "transport_endpoint",
                self.transport_endpoint.clone(),
                "transport plugin init() 目標 URL (runtime.rs make_transport_config_val)",
                self.transport_endpoint == default.transport_endpoint,
            ),
            (
                "max_transport_bytes",
                format!("{} KB", self.max_transport_bytes / 1024),
                "每次 transport send() 的最大 byte 數 = 一個 HTTP POST 大小",
                self.max_transport_bytes == default.max_transport_bytes,
            ),
            (
                "transport_workers",
                format!("{}", self.transport_workers),
                "並行 transport worker 數量，每個持有獨立 WASM store",
                self.transport_workers == default.transport_workers,
            ),
        ];

        eprintln!("┌{:─<28}┬{:─<14}┬{:─<7}┬{:─<58}┐", "", "", "", "");
        eprintln!(
            "│ {:26} │ {:12} │ {:5} │ {:56} │",
            "參數", "值", "預設", "用途"
        );
        eprintln!("├{:─<28}┼{:─<14}┼{:─<7}┼{:─<58}┤", "", "", "", "");
        for (name, val, desc, is_default) in rows {
            eprintln!(
                "│ {:26} │ {:12} │ {:5} │ {:56} │",
                name,
                val,
                if *is_default { "✓" } else { "●" },
                desc
            );
        }
        eprintln!("└{:─<28}┴{:─<14}┴{:─<7}┴{:─<58}┘", "", "", "", "");
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
    pub lines: Vec<String>,
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
    pub fn push(&mut self, line: String) {
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

#[derive(Clone, Copy, Default)]
pub struct ParseDiffTiming {
    pub noop_elapsed_ns: u64,
    pub noop_component_ns: u64,
    pub copy_in_ns: u64,
    pub guest_ns: u64,
    pub copy_out_ns: u64,
}

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
    /// 累計 memory.grow 觸發次數（各 batch 加總）
    pub total_grow_count: u64,
    /// 累計 memory.grow 增量 bytes（各 batch 加總）
    pub total_grow_delta_bytes: u64,
    /// 累計 component 內部回報的執行時間 ns（各 batch 加總）
    pub total_component_ns: u64,
    /// 差分量測成功的 batch 數。
    pub total_diff_batches: u64,
    /// no-op parser 的整段 call_parse() 時間，用於估算 host→WCM copy-in。
    pub total_noop_elapsed_ns: u64,
    /// no-op parser 回報的 guest 端空邏輯時間。
    pub total_noop_component_ns: u64,
    /// 差分估算的 host→WCM copy-in 時間。
    pub total_copy_in_ns: u64,
    /// 差分估算的 WCM→host copy-out 時間。
    pub total_copy_out_ns: u64,
}

#[derive(Default)]
pub struct FormatStats {
    pub total_batches: u64,
    pub total_input_entries: u64,
    pub total_output_lines: u64,
    pub total_elapsed: Duration,
    /// WASM 線性記憶體峰值（各 batch 最大值）
    pub wasm_mem_peak_max: usize,
    /// 累計 component 內部回報的編碼邏輯時間 ns（各 batch 加總）
    pub total_component_ns: u64,
}

#[derive(Default)]
pub struct TransportStats {
    pub total_batches: u64,
    /// 傳送的格式化行數
    pub total_input_lines: u64,
    /// 傳送的總 byte 數（格式化輸出大小）
    pub total_input_bytes: u64,
    /// transport plugin report-usage() 回報的累計 byte 數
    pub total_bytes_reported: u64,
    pub total_elapsed: Duration,
    /// WASM 線性記憶體峰值
    pub wasm_mem_peak_max: usize,
}

#[derive(Default)]
pub struct FilterStats {
    pub total_batches: u64,
    pub total_input_entries: u64,
    pub total_kept_entries: u64,
    pub total_dropped_entries: u64,
    pub total_elapsed: Duration,
    pub wasm_mem_peak_max: usize,
    pub total_component_ns: u64,
}
