//! WCM benchmark pipeline 的多階段執行與工作排程。
//!
//! 本檔案管理 fresh parse instance pool、批次分派與 worker，依序執行 parse、filter、
//! format、transport components，收集 H0–H3 與記憶體統計，並處理重試、熱更新及錯誤輸出。

use std::collections::VecDeque;
use std::fs::File;
use std::future::Future;
use std::io::Write;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
// [latency-bench] AtomicBool 只有已註解的採樣 thread 使用。
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, OnceLock, RwLock};
use std::thread;
// [latency-bench] SystemTime / UNIX_EPOCH 只有 throughput 採樣 thread 使用，已隨採樣一併註解。
use std::time::{Duration, Instant};

use wasmtime::Store;
use wasmtime_wasi::sockets::SocketAddrUse;
use wasmtime_wasi::{ResourceTable, WasiCtxBuilder};
use wasmtime_wasi_http::WasiHttpCtx;

use crate::app::{
    MyLimiter, MyState, ParserPlugin, SharedPlugin,
    format_bindings::FormatPlugin,
    local::log_process::pipeline_process::{LogEntry, ParsedEntry},
    reduction_bindings::ReductionPlugin,
    reset_copy_markers, take_copy_markers,
    transport_bindings::{
        TransportPlugin,
        local::log_process::transport_types::{AuthMethod, TransportConfig},
    },
};
// [latency-bench] ParseDiffTiming 在 measure_noop_parse 註解後已不再使用，import 一併移除。
use crate::config::{
    AppConfig, Batch, BatchConfig, FilterStats, FlushReason, FormatStats, ParseStats,
    TransportStats,
};
// [perf] print_parse_batch / print_filter_batch / print_format_batch 為每批次的
//        stderr 輸出：每批產生 format! 配置與同步 write syscall，影響 p99。
//        呼叫端已全部註解，import 一併移除；需要除錯時再打開。
use crate::output::{
    // [latency-bench] print_flush_header 的呼叫端已註解，import 一併移除。
    print_parse_aggregate,
    print_pipeline_summary,
};

// ── 階段間傳遞的批次結構 ────────────────────────────────────────────────────

/// parse 階段的輸出：解析後的 LogEntry 與各筆對應的 targettag。
struct ParsedBatch {
    entries: Vec<LogEntry>,
    targettags: Vec<String>,
    seq: u64,
}

/// format 階段的輸出：已格式化的輸出行（bytes）與各行對應的 targettag。
struct FormattedBatch {
    lines: Vec<Vec<u8>>,
    targettags: Vec<String>,
    seq: u64,
    total_bytes: usize,
}

/// transport router 分發給 transport worker 的傳送任務。
struct SendTask {
    url: String,
    lines: Vec<Vec<u8>>,
}

// ── Parse 平行化常數與 WorkBatch ──────────────────────────────────────────────

const PARSE_WORKERS: usize = 1;

/// WASM 實例化（_initialize）期間使用的 epoch deadline。
/// 實例化通常在毫秒內完成；此值約有 28 小時的餘裕，
/// 確保 pool 補充／filter 重建不會誤觸 epoch 逾時保護。
const INSTANTIATE_EPOCH_DEADLINE: u64 = 10_000_000;

/// dispatcher 分發給 parse worker 的工作單元。
struct WorkBatch {
    lines: Vec<String>,
    bytes: usize,
    seq: u64,
    reason: FlushReason,
    created_at: Instant,
}

// ── Parse Store Pool ──────────────────────────────────────────────────────────
//
// 每個 parse worker 持有一個獨立的 ParsePool（POOL_TARGET_SIZE 個備援 instance）。
// 正常流程：acquire → parse() → discard_and_replenish
// OOM 流程：acquire → parse() OOM → discard_and_replenish → acquire 備援 → retry

const POOL_TARGET_SIZE: usize = 3;

struct ParseInstance {
    id: u8,
    store: Store<MyState>,
    plugin: ParserPlugin,
}

struct ParsePool {
    ready: VecDeque<ParseInstance>,
    shared: SharedPlugin,
    mem_limit_bytes: usize,
    target_size: usize,
    current_count: u8,
    current_version: u64,
}

impl ParsePool {
    /// 建立新的 parse pool 並預熱實例。
    /// 使用時機：
    /// 1. 系統啟動初始化
    /// 2. plugin 熱更換
    fn new(
        shared: SharedPlugin,
        mem_limit_bytes: usize,
        target_size: usize,
    ) -> wasmtime::Result<Self> {
        let version = shared.read().unwrap().version;
        let mut pool = Self {
            ready: VecDeque::with_capacity(target_size),
            shared,
            mem_limit_bytes,
            target_size,
            current_count: 0,
            current_version: version,
        };
        // 不足3個inst時，補充inst
        pool.replenish();
        if pool.ready.is_empty() {
            return Err(wasmtime::Error::msg(
                "parse pool: failed to pre-warm any instance",
            ));
        }
        eprintln!(
            "[pool] pre-warmed {}/{} parse instances (plugin v{})",
            pool.ready.len(),
            target_size,
            version,
        );
        Ok(pool)
    }

    /// 建立一個新的 WASM parse 實例（含 Store、記憶體限制與 epoch deadline 設定）。
    fn create_one(&mut self) -> wasmtime::Result<ParseInstance> {
        let slot = self.shared.read().unwrap();
        let state = MyState {
            ctx: WasiCtxBuilder::new().inherit_stdio().build(),
            table: ResourceTable::new(),
            limiter: MyLimiter::new(self.mem_limit_bytes),
            http: WasiHttpCtx::new(),
        };
        let mut store = Store::new(&slot.engine, state);
        store.limiter(|s| &mut s.limiter);
        store.set_epoch_deadline(INSTANTIATE_EPOCH_DEADLINE);
        let plugin = ParserPlugin::instantiate(&mut store, &slot.component, &slot.linker)?;
        drop(slot);
        self.current_count += 1;
        let id = self.current_count;
        Ok(ParseInstance { id, store, plugin })
    }

    /// 偵測 config watcher 觸發的插件版本更新；版本改變時丟棄所有實例並重建整個 pool。
    fn check_reload(&mut self) {
        let new_version = self.shared.read().unwrap().version;
        if new_version != self.current_version {
            eprintln!(
                "[parse-pool] plugin v{} → v{}: discarding {} instances, rebuilding pool",
                self.current_version,
                new_version,
                self.ready.len()
            );
            self.ready.clear();
            self.current_count = 0;
            self.current_version = new_version;
            self.replenish();
            eprintln!("[parse-pool] pool ready ({} instances)", self.ready.len());
        }
    }
    /// 實例數量少於目標數量時補充實例；
    /// 若 plugin import 了未授權的介面（socket／filesystem 等）則拒絕載入並記錄安全訊息。
    fn replenish(&mut self) {
        while self.ready.len() < self.target_size {
            match self.create_one() {
                Ok(inst) => self.ready.push_back(inst),
                Err(e) => {
                    let msg = e.to_string();
                    // 從錯誤訊息中提取缺少的 import 名稱，格式通常為
                    // "unknown import: `<interface>::<func>`"
                    let import_hint = msg
                        .find("unknown import")
                        .and_then(|_| msg.find('`'))
                        .map(|s| {
                            let rest = &msg[s + 1..];
                            let end = rest.find('`').unwrap_or(rest.len());
                            &rest[..end]
                        })
                        .unwrap_or("(unknown)");

                    if msg.contains("wasi:sockets")
                        || msg.contains("tcp-create-socket")
                        || msg.contains("udp-create-socket")
                        || msg.contains("ip-name-lookup")
                    {
                        eprintln!(
                            "[SECURITY][pool] 拒絕載入：plugin import 了未授權的 socket 介面 `{}` ({}/{})",
                            import_hint,
                            self.ready.len(),
                            self.target_size
                        );
                    } else if msg.contains("wasi:filesystem") {
                        eprintln!(
                            "[SECURITY][pool] 拒絕載入：plugin import 了未授權的 filesystem 介面 `{}` ({}/{})",
                            import_hint,
                            self.ready.len(),
                            self.target_size
                        );
                    } else if msg.contains("unknown import") {
                        eprintln!(
                            "[SECURITY][pool] 拒絕載入：plugin import 了 linker 中不存在的介面 `{}` ({}/{})",
                            import_hint,
                            self.ready.len(),
                            self.target_size
                        );
                    } else {
                        eprintln!(
                            "[pool] replenish failed ({}/{}): {}",
                            self.ready.len(),
                            self.target_size,
                            e
                        );
                    }
                    break;
                }
            }
        }
    }

    /// 從 pool 取出一個實例供 parse 使用；pool 耗盡時緊急建立新實例。
    fn acquire(&mut self) -> wasmtime::Result<ParseInstance> {
        if let Some(inst) = self.ready.pop_front() {
            Ok(inst)
        } else {
            eprintln!("[pool] pool exhausted, creating emergency instance");
            self.create_one()
        }
    }

    // 選擇每批重建，不使用同一個
    //fn release(&mut self, mut inst: ParseInstance) {
    //    self.ready.push_back(inst);
    //}

    //fn release(&mut self, inst: ParseInstance) {
    //    self.ready.push_back(inst);
    //    self.replenish();
    //}

    //fn release_without_reset(&mut self, inst: ParseInstance) {
    //    self.ready.push_back(inst);
    //    self.replenish();
    //}

    /// 用完實例後直接丟棄並補充新實例（每批次皆重建，不重複使用）。
    fn discard_and_replenish(&mut self, inst: ParseInstance) {
        drop(inst);
        self.replenish();
    }
}

// ── 對外入口 ──────────────────────────────────────────────────────────────────

/// 啟動背景執行緒，每 10 ms 遞增 engine 的 epoch。
/// 當 Store 的 epoch deadline 到期時，執行中的 WASM 呼叫會以 trap 中止，
/// 藉此解除無窮迴圈或卡死 plugin 的阻塞。
/// 此執行緒每次都從 SharedPlugin 讀取當前 engine，
/// 因此熱更換後會自動跟隨新 engine，無需重啟。
///
// 這是一個公用計時器，讓每個stage的plugin對齊的時間
fn spawn_epoch_ticker(shared: &SharedPlugin) {
    let sh = Arc::clone(shared);
    thread::spawn(move || {
        loop {
            thread::sleep(Duration::from_millis(10));
            sh.read().unwrap().engine.increment_epoch();
        }
    });
}

/// Pipeline 入口：input → [parse thread] → [filter thread?] → [format thread] → [transport thread]
/// 建立各階段執行緒與 channel、啟動 epoch ticker，最後彙總各階段統計並輸出總結。
/// `cfg` 與 config-watcher 執行緒共享；parse_dispatcher 每次迭代都會從中讀取
/// 批次參數，因此 max_wait、buffer_size_mb 支援熱重載。
pub fn run_pipeline(
    rx_raw: Receiver<String>,
    parse: Option<SharedPlugin>,
    parse_noop: Option<SharedPlugin>,
    filter: Option<SharedPlugin>,
    format: Option<SharedPlugin>,
    transport: Option<SharedPlugin>,
    cfg: Arc<RwLock<AppConfig>>,
) -> wasmtime::Result<()> {
    // 從初始 config 快照中取出啟動後即固定的參數。
    let (
        mem_limit_bytes,
        max_format_chunk,
        max_transport_bytes,
        transport_workers,
        pipeline_channel_capacity,
        stage_timeout,
        instance_reuse,
    ) = {
        let c = cfg.read().unwrap();
        let b = BatchConfig::from(c.batch.clone());
        (
            b.mem_limit_mb * 1024 * 1024,
            b.max_format_chunk,
            b.max_transport_bytes,
            b.transport_workers,
            b.pipeline_channel_capacity,
            Duration::from_millis(b.stage_timeout_ms),
            b.plugin_instance_reuse,
        )
    };
    // 每個 epoch tick = 10 ms；deadline = 卡住的 plugin 被中止前可經過的 tick 數。
    let deadline_ticks = stage_timeout.as_millis() as u64 / 10;

    // parse → filter channel（或直接 parse → format 若 filter 停用）
    let (tx_parsed, rx_parsed) =
        std::sync::mpsc::sync_channel::<ParsedBatch>(pipeline_channel_capacity);
    // filter → format channel
    let (tx_filtered, rx_filtered) =
        std::sync::mpsc::sync_channel::<ParsedBatch>(pipeline_channel_capacity);
    let (tx_formatted, rx_formatted) =
        std::sync::mpsc::sync_channel::<FormattedBatch>(pipeline_channel_capacity);

    let cfg_for_parse = Arc::clone(&cfg);

    // [latency-bench] wall-clock 時間擷取已註解，避免干擾延遲量測。
    // let wall_start = Instant::now();

    // ── Parse thread（dispatcher + N workers）────────────────────────────────
    let parse_handle = if let Some(shared) = parse {
        // 開一個自己的記數thread
        spawn_epoch_ticker(&shared);
        if let Some(ref noop) = parse_noop {
            spawn_epoch_ticker(noop);
        }
        Some(thread::spawn(move || {
            parse_loop(
                rx_raw,
                tx_parsed,
                shared,
                parse_noop,
                cfg_for_parse,
                mem_limit_bytes,
                deadline_ticks,
            )
        }))
    } else {
        drop(tx_parsed);
        thread::spawn(move || while rx_raw.recv().is_ok() {});
        None
    };

    // ── Filter thread (sync) ──────────────────────────────────────────────────
    let filter_handle = if let Some(shared) = filter {
        // 開一個自己的記數thread
        spawn_epoch_ticker(&shared);
        Some(thread::spawn(move || {
            filter_loop(
                rx_parsed,
                tx_filtered,
                shared,
                mem_limit_bytes,
                stage_timeout,
            )
        }))
    } else {
        // filter 停用：把 rx_parsed 直接橋接到 tx_filtered
        thread::spawn(move || {
            while let Ok(pb) = rx_parsed.recv() {
                if tx_filtered.send(pb).is_err() {
                    break;
                }
            }
        });
        None
    };

    // ── Format thread (sync) ──────────────────────────────────────────────────
    let format_handle = if let Some(shared) = format {
        // 開一個自己的記數thread
        spawn_epoch_ticker(&shared);
        Some(thread::spawn(move || {
            format_loop(
                rx_filtered,
                tx_formatted,
                shared,
                mem_limit_bytes,
                max_format_chunk,
                stage_timeout,
                instance_reuse,
            )
        }))
    } else {
        drop(tx_formatted);
        thread::spawn(move || while rx_filtered.recv().is_ok() {});
        None
    };

    // ── Transport thread (router + N async workers) ───────────────────────────
    let transport_handle = if let Some(shared) = transport {
        spawn_epoch_ticker(&shared);
        Some(thread::spawn(move || {
            // router → workers 的工作佇列
            let (tx_work, rx_work) = std::sync::mpsc::sync_channel::<SendTask>(1024);
            let rx_work_shared = Arc::new(Mutex::new(rx_work));

            // router thread：接 FormattedBatch，依 targettag 分流到 per-endpoint buffer，
            // buffer 滿了就推 SendTask 進工作佇列。
            let cfg_for_router = Arc::clone(&cfg);
            let router_handle = thread::spawn(move || {
                transport_router(rx_formatted, tx_work, cfg_for_router, max_transport_bytes);
            });

            // N transport workers：搶 SendTask，每 task 建立新 WASM 實例。
            let mut worker_handles = Vec::new();
            for i in 0..transport_workers {
                let rx = Arc::clone(&rx_work_shared);
                let sh = Arc::clone(&shared);
                let cfg_w = Arc::clone(&cfg);
                worker_handles.push(thread::spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap()
                        .block_on(async move {
                            transport_worker(
                                rx,
                                sh,
                                mem_limit_bytes,
                                max_transport_bytes,
                                i,
                                stage_timeout,
                                cfg_w,
                                instance_reuse,
                            )
                            .await
                        })
                }));
            }

            let _ = router_handle.join();

            let mut combined = TransportStats::default();
            for h in worker_handles {
                match h.join() {
                    Ok(Ok(stats)) => {
                        combined.total_batches += stats.total_batches;
                        combined.total_input_lines += stats.total_input_lines;
                        combined.total_input_bytes += stats.total_input_bytes;
                        combined.total_bytes_reported += stats.total_bytes_reported;
                        combined.total_elapsed += stats.total_elapsed;
                        if stats.wasm_mem_peak_max > combined.wasm_mem_peak_max {
                            combined.wasm_mem_peak_max = stats.wasm_mem_peak_max;
                        }
                    }
                    Ok(Err(e)) => return Err(e),
                    Err(_) => return Err(wasmtime::Error::msg("transport worker panicked")),
                }
            }
            Ok(combined)
        }))
    } else {
        thread::spawn(move || while rx_formatted.recv().is_ok() {});
        None
    };

    // ── Collect results ───────────────────────────────────────────────────────
    let parse_stats = parse_handle
        .map(|h| h.join().expect("parse thread panicked"))
        .transpose()?;
    let filter_stats = filter_handle
        .map(|h| h.join().expect("filter thread panicked"))
        .transpose()?;
    let format_stats = format_handle
        .map(|h| h.join().expect("format thread panicked"))
        .transpose()?;
    let transport_stats = transport_handle
        .map(|h| h.join().expect("transport thread panicked"))
        .transpose()?;

    // [latency-bench] wall-clock 時間擷取已註解，summary 不再輸出時間相關欄位。
    // let wall_elapsed = wall_start.elapsed();

    if let Some(ps) = &parse_stats {
        print_pipeline_summary(
            ps,
            filter_stats.as_ref(),
            format_stats.as_ref(),
            transport_stats.as_ref(),
            // [latency-bench] wall_elapsed 參數已移除
            mem_limit_bytes,
        );
    }

    Ok(())
}

// ── Parse：orchestrator ───────────────────────────────────────────────────────
//
// 架構：
//   parse_loop（主 thread）
//     ├─ parse_dispatcher（跑在主 thread 上，累積 batch 並送 WorkBatch）
//     └─ parse_worker × PARSE_WORKERS（各自持有 ParsePool，搶 rx_work 處理）
//
// WorkBatch channel 關閉（dispatcher 結束後 tx_work drop）→ workers 排完後退出。
// 主 thread 等所有 workers 結束，彙總統計並印出 wall-clock 總吞吐。

/// Parse 階段主迴圈（orchestrator）：啟動 PARSE_WORKERS 個 worker 執行緒，
/// 並在當前執行緒跑 dispatcher；dispatcher 結束後等待所有 worker 完成，
/// 彙總各 worker 統計並輸出 aggregate 結果。
fn parse_loop(
    rx: Receiver<String>,
    tx: SyncSender<ParsedBatch>,
    shared: SharedPlugin,
    parse_noop: Option<SharedPlugin>,
    cfg: Arc<RwLock<AppConfig>>,
    mem_limit_bytes: usize,
    deadline_ticks: u64,
) -> wasmtime::Result<ParseStats> {
    // [latency-bench] wall-clock 時間擷取已註解，避免干擾延遲量測。
    // let wall_start = Instant::now();

    let (tx_work, rx_work) = std::sync::mpsc::sync_channel::<WorkBatch>(64);
    let rx_work = Arc::new(Mutex::new(rx_work));

    // 共用計數器：parse workers 累積已處理的 input lines
    let total_parsed_lines = Arc::new(AtomicU64::new(0));

    // [latency-bench] 每秒採樣 thread（SystemTime::now + csv 檔案寫入）屬於時間擷取與
    //                 週期性 I/O，會干擾延遲量測，整段註解。需要吞吐量統計時再打開。
    // let sampler_stop = Arc::new(AtomicBool::new(false));
    //
    // // 每秒採樣 thread：寫入 parse_throughput.csv
    // let sampler_total = Arc::clone(&total_parsed_lines);
    // let sampler_flag = Arc::clone(&sampler_stop);
    // let sampler_handle = thread::spawn(move || {
    //     let mut csv = match File::create("parse_throughput.csv") {
    //         Ok(f) => f,
    //         Err(e) => {
    //             eprintln!("[sampler] 無法建立 parse_throughput.csv: {}", e);
    //             return;
    //         }
    //     };
    //     let _ = writeln!(csv, "ts,total_line");
    //     loop {
    //         thread::sleep(Duration::from_secs(1));
    //         if sampler_flag.load(Ordering::Relaxed) {
    //             break;
    //         }
    //         let ts = SystemTime::now()
    //             .duration_since(UNIX_EPOCH)
    //             .unwrap_or_default()
    //             .as_secs();
    //         let total = sampler_total.load(Ordering::Relaxed);
    //         let _ = writeln!(csv, "{},{}", ts, total);
    //     }
    // });

    // 啟動 PARSE_WORKERS 個 worker thread
    let mut worker_handles = Vec::new();
    for worker_id in 0..PARSE_WORKERS {
        let rx_w = Arc::clone(&rx_work);
        let tx_p = tx.clone();
        let sh = Arc::clone(&shared);
        let noop = parse_noop.as_ref().map(Arc::clone);
        let counter = Arc::clone(&total_parsed_lines);
        worker_handles.push(thread::spawn(move || {
            parse_worker(
                worker_id,
                rx_w,
                tx_p,
                sh,
                noop,
                mem_limit_bytes,
                deadline_ticks,
                counter,
            )
        }));
    }

    // dispatcher 跑在目前 thread，結束後 tx_work 自動 drop
    parse_dispatcher(rx, tx_work, cfg);

    // 彙總 workers 的統計
    // [latency-bench] wall-clock 時間擷取已註解。
    // let wall_elapsed = wall_start.elapsed();
    let mut combined = ParseStats::default();
    let mut total_errors: u32 = 0;

    for (i, h) in worker_handles.into_iter().enumerate() {
        match h.join() {
            Ok(Ok((stats, errs))) => {
                combined.total_batches += stats.total_batches;
                combined.total_input_lines += stats.total_input_lines;
                combined.total_input_bytes += stats.total_input_bytes;
                combined.total_output_entries += stats.total_output_entries;
                combined.total_elapsed += stats.total_elapsed;
                combined.total_component_ns += stats.total_component_ns;
                combined.total_diff_batches += stats.total_diff_batches;
                combined.total_noop_elapsed_ns += stats.total_noop_elapsed_ns;
                combined.total_noop_component_ns += stats.total_noop_component_ns;
                combined.total_copy_in_ns += stats.total_copy_in_ns;
                combined.total_copy_out_ns += stats.total_copy_out_ns;
                combined.total_grow_count += stats.total_grow_count;
                combined.total_grow_delta_bytes += stats.total_grow_delta_bytes;
                if stats.go_heap_peak_max > combined.go_heap_peak_max {
                    combined.go_heap_peak_max = stats.go_heap_peak_max;
                }
                if stats.wasm_mem_peak_max > combined.wasm_mem_peak_max {
                    combined.wasm_mem_peak_max = stats.wasm_mem_peak_max;
                }
                total_errors += errs;
            }
            Ok(Err(e)) => return Err(e),
            Err(_) => return Err(wasmtime::Error::msg(format!("parse worker {} panicked", i))),
        }
    }

    // [latency-bench] 採樣 thread 已註解，停止/join 一併註解。
    // sampler_stop.store(true, Ordering::Relaxed);
    // let _ = sampler_handle.join();

    // [latency-bench] wall_elapsed 參數已移除，aggregate 不再輸出時間相關欄位。
    print_parse_aggregate(&combined, PARSE_WORKERS, total_errors);
    Ok(combined)
}

// ── Parse：dispatcher ─────────────────────────────────────────────────────────
//
// 負責從 rx_raw 累積行、觸發 flush 條件後把 WorkBatch 送進 tx_work。
// 不碰 WASM，只做 batching 邏輯。

/// Parse 階段的 dispatcher：從 rx 累積輸入行，依 size（超過 buffer 上限）、
/// time（批次窗到期）、eof（輸入來源關閉）三種條件觸發 flush，
/// 把 WorkBatch 送進工作佇列。不碰 WASM，只做 batching 邏輯。
fn parse_dispatcher(rx: Receiver<String>, tx: SyncSender<WorkBatch>, cfg: Arc<RwLock<AppConfig>>) {
    let mut batch = Batch::new();
    let mut seq: u64 = 0;
    let benchmark_batch_records = std::env::var("WCM_BENCH_BATCH_RECORDS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0);

    loop {
        // 每個 recv 週期讀取一次可熱重載的參數（持鎖時間極短）。
        let (max_wait, buffer_budget) = {
            let c = cfg.read().unwrap();
            (
                Duration::from_millis(c.batch.max_wait_ms),
                c.batch.buffer_size_mb * 1024 * 1024,
            )
        };

        let timeout = if batch.is_empty() {
            max_wait
        } else {
            max_wait.saturating_sub(batch.elapsed())
        };

        match rx.recv_timeout(timeout) {
            Ok(item) => {
                let line_len = item.len();
                let size_trigger = !batch.is_empty() && batch.bytes + line_len > buffer_budget;

                if size_trigger {
                    let reason = FlushReason {
                        size: true,
                        time: false,
                        eof: false,
                    };
                    if send_work_batch(&mut batch, seq, reason, &tx).is_err() {
                        break;
                    }
                    seq += 1;
                }
                batch.push(item);
                if benchmark_batch_records.is_some_and(|limit| batch.len() >= limit) {
                    let reason = FlushReason {
                        size: true,
                        time: false,
                        eof: false,
                    };
                    if send_work_batch(&mut batch, seq, reason, &tx).is_err() {
                        break;
                    }
                    seq += 1;
                }
            }

            Err(RecvTimeoutError::Timeout) => {
                if !batch.is_empty() {
                    let reason = FlushReason {
                        size: false,
                        time: true,
                        eof: false,
                    };
                    if send_work_batch(&mut batch, seq, reason, &tx).is_err() {
                        break;
                    }
                    seq += 1;
                }
            }

            Err(RecvTimeoutError::Disconnected) => {
                if !batch.is_empty() {
                    let reason = FlushReason {
                        size: false,
                        time: false,
                        eof: true,
                    };
                    let _ = send_work_batch(&mut batch, seq, reason, &tx);
                }
                break;
            }
        }
    }
}

/// 把累積的 Batch 打包成 WorkBatch 送進工作佇列，並印出本次 flush 的觸發原因。
/// 回傳 Err 表示工作佇列已關閉（下游結束）。
fn send_work_batch(
    batch: &mut Batch,
    seq: u64,
    reason: FlushReason,
    tx: &SyncSender<WorkBatch>,
) -> Result<(), ()> {
    // [perf] 每次 flush 的 eprintln 是熱路徑上的同步 stderr syscall，影響 p99，故註解。
    //        原輸出記錄觸發原因：size=buffer 到達 buffer_size_mb、
    //        time=批次窗 max_wait_ms 到期、eof=輸入來源關閉。除錯時再打開：
    // let why = if reason.eof { "eof " } else if reason.size { "size" } else { "time" };
    // eprintln!(
    //     "[flush   #{:>4}]  reason={}  In={:>6}/{:>7.0}KB",
    //     seq, why,
    //     batch.len(),
    //     batch.bytes as f64 / 1024.0,
    // );
    let created_at = batch.created_at;
    let lines = std::mem::take(&mut batch.lines);
    let bytes = batch.bytes;
    batch.bytes = 0;
    batch.created_at = Instant::now();
    tx.send(WorkBatch {
        lines,
        bytes,
        seq,
        reason,
        created_at,
    })
    .map_err(|_| ())
}

// ── Parse：worker ─────────────────────────────────────────────────────────────
//
// 每個 worker 持有獨立 ParsePool，搶 rx_work 中的 WorkBatch，
// 完成 WASM parse 後把 ParsedBatch 推給下游 tx。

/// Parse worker：持有獨立的 ParsePool，從工作佇列搶 WorkBatch，
/// 完成 WASM parse 後把 ParsedBatch 推給下游，並累計統計與已處理行數。
fn parse_worker(
    worker_id: usize,
    rx: Arc<Mutex<Receiver<WorkBatch>>>,
    tx: SyncSender<ParsedBatch>,
    shared: SharedPlugin,
    parse_noop: Option<SharedPlugin>,
    mem_limit_bytes: usize,
    deadline_ticks: u64,
    total_lines: Arc<AtomicU64>,
) -> wasmtime::Result<(ParseStats, u32)> {
    let mut pool = ParsePool::new(shared, mem_limit_bytes, POOL_TARGET_SIZE)?;
    let mut noop_pool = parse_noop
        .map(|sp| ParsePool::new(sp, mem_limit_bytes, POOL_TARGET_SIZE))
        .transpose()?;
    let mut stats = ParseStats::default();
    let mut error_count: u32 = 0;

    loop {
        let wb = match rx.lock().unwrap().recv() {
            Ok(wb) => wb,
            Err(_) => break,
        };

        let seq = wb.seq;
        let reason = wb.reason;
        let mut batch = Batch {
            lines: wb.lines,
            bytes: wb.bytes,
            created_at: wb.created_at,
        };

        pool.check_reload();
        if let Some(np) = &mut noop_pool {
            np.check_reload();
        }

        let lines_before = stats.total_input_lines;
        if !worker_flush_batch(
            worker_id,
            &mut pool,
            noop_pool.as_mut(),
            &mut batch,
            seq,
            &reason,
            &mut stats,
            &tx,
            &mut error_count,
            1,
            deadline_ticks,
        ) {
            total_lines.fetch_add(stats.total_input_lines - lines_before, Ordering::Relaxed);
            break;
        }
        total_lines.fetch_add(stats.total_input_lines - lines_before, Ordering::Relaxed);
    }

    eprintln!(
        "[parse-worker-{}] done  batches={}  entries={}  errors={}",
        worker_id, stats.total_batches, stats.total_output_entries, error_count,
    );
    Ok((stats, error_count))
}

/// 取 instance、執行 parse、處理 OOM 重試，最終送出 ParsedBatch。
/// 回傳 false 表示下游 channel 已關閉，worker 應停止。
fn worker_flush_batch(
    worker_id: usize,
    pool: &mut ParsePool,
    mut noop_pool: Option<&mut ParsePool>,
    batch: &mut Batch,
    seq: u64,
    reason: &FlushReason,
    stats: &mut ParseStats,
    tx: &SyncSender<ParsedBatch>,
    error_count: &mut u32,
    max_retries: u32,
    deadline_ticks: u64,
) -> bool {
    for attempt in 0..=max_retries {
        let mut inst = match pool.acquire() {
            Ok(i) => i,
            Err(e) => {
                eprintln!(
                    "[worker{}][pool] cannot acquire (batch={}): {}",
                    worker_id, seq, e
                );
                batch.clear();
                return true;
            }
        };
        inst.store.data_mut().limiter.reset_batch_stats();

        match do_parse_batch(
            worker_id,
            &mut inst,
            noop_pool.as_deref_mut(),
            batch,
            seq,
            pool.mem_limit_bytes,
            reason,
            stats,
            deadline_ticks,
        ) {
            Ok(Some(pb)) => {
                //pool.release(inst);
                pool.discard_and_replenish(inst);
                return tx.send(pb).is_ok();
            }
            Ok(None) => {
                //pool.release(inst);
                pool.discard_and_replenish(inst);
                return true;
            }
            Err(e) => {
                eprintln!(
                    "[worker{}][trap {}/{}] batch={}: {}",
                    worker_id,
                    attempt + 1,
                    max_retries + 1,
                    seq,
                    e
                );
                pool.discard_and_replenish(inst);
                if attempt == max_retries {
                    eprintln!(
                        "[worker{}][trap] exceeded retries, skip batch {}",
                        worker_id, seq
                    );
                    *error_count += 1;
                    write_error_file("以下這批發生trap/timeout", &batch.lines);
                    batch.clear();
                }
            }
        }
    }
    true
}

/// 在單一實例上執行一次 WASM parse：呼叫 plugin 的 parse()，
/// 把解析結果轉為 LogEntry／targettag 並更新統計。
/// 回傳 Ok(None) 表示 plugin 回報 parse 錯誤（略過該批）；Err 表示 trap／timeout。
fn do_parse_batch(
    worker_id: usize,
    inst: &mut ParseInstance,
    // [latency-bench] noop 差分量測已註解，noop_pool 暫不使用。
    _noop_pool: Option<&mut ParsePool>,
    batch: &mut Batch,
    seq: u64,
    // [latency-bench] mem_limit_bytes 原本只供 print_parse_batch 的 Mem% 使用，已不需要。
    _mem_limit_bytes: usize,
    // [latency-bench] reason 僅供 print_flush_header 使用，該呼叫註解後已不需要。
    _reason: &FlushReason,
    stats: &mut ParseStats,
    deadline_ticks: u64,
) -> wasmtime::Result<Option<ParsedBatch>> {
    let input_lines = batch.len();
    let input_bytes = batch.bytes;
    // [latency-bench] host 端時間擷取已註解，避免干擾延遲量測。
    // let started = Instant::now();

    inst.store.set_epoch_deadline(deadline_ticks);

    // ======================================== //
    // 這裡請插入時間擷取，並將當下的時間寫入檔案   //
    // 時間使用奈秒，並記錄當下的輪次是第幾個，避免
    // 錯亂。
    // 範例輸出:
    // [parse-copy-in-1] time
    // [paser-copy-in-2] time
    let result = match inst.plugin.call_parse(&mut inst.store, &batch.lines) {
        Ok(Ok(parsed)) => {
            // [latency-bench] host 端時間擷取（elapsed）已註解。
            // let elapsed = started.elapsed();
            // [latency-bench] call_report_usage 會多產生一次 WASM 呼叫而污染延遲量測，
            //                 量測延遲分佈期間以 0 代替，排除額外開銷。
            // let component_ns = inst.plugin.call_report_usage(&mut inst.store).unwrap_or(0);
            // let component_ns: u64 = 0;
            // [latency-bench] noop 差分量測（copy-in/out 估算）本身就是時間擷取，整段註解。
            // let diff = match noop_pool {
            //     Some(pool) => match measure_noop_parse(pool, &batch.lines, component_ns, elapsed, deadline_ticks) {
            //         Ok(diff) => Some(diff),
            //         Err(e) => {
            //             eprintln!("[worker{}][diff-warn] batch={} {}", worker_id, seq, e);
            //             None
            //         }
            //     },
            //     None => None,
            // };
            let wasm_mem_peak = inst.store.data().limiter.wasm_mem_peak;
            let grow_count = inst.store.data().limiter.grow_count;
            let grow_delta_bytes = inst.store.data().limiter.grow_total_delta_bytes;
            // [perf] inst_id 只有下方已註解的測試輸出使用，一併註解。
            // let inst_id = inst.id;

            let (entries, targettags): (Vec<LogEntry>, Vec<String>) = parsed
                .into_iter()
                .enumerate()
                .map(|(i, p): (usize, ParsedEntry)| {
                    let e = LogEntry {
                        id: seq * 100_000 + i as u64,
                        timestamp: p.timestamp,
                        level: p.level,
                        message: p.message,
                        tags: p.tags,
                    };
                    (e, p.targettag)
                })
                .unzip();

            // [latency-bench] tag 除錯輸出：每批次都會產生 Vec 配置 + eprintln，影響 p99，故註解。
            // if let Some(first) = entries.first() {
            //     if !first.tags.is_empty() {
            //         let tag_str: Vec<String> = first
            //             .tags
            //             .iter()
            //             .map(|(k, v)| format!("{}={}", k, v))
            //             .collect();
            //         eprintln!("[parse][batch={}] tags: {}", seq, tag_str.join(" | "));
            //     }
            // }
            // [latency-bench] print_flush_header：每次 flush 的 eprintln 對延遲測試不必要，故註解。
            // print_flush_header(seq, batch, reason);
            // [latency-bench] 時間相關參數（component_ns/elapsed/diff）與資源參數已自
            //                 print_parse_batch 簽名移除，僅保留批次數量資訊。
            // [perf] 每批次的 stderr 輸出影響 p99，連同簡化版一併註解；除錯時再打開。
            // print_parse_batch(
            //     worker_id,
            //     seq,
            //     input_lines,
            //     input_bytes,
            //     entries.len(),
            // );
            /*
            if let Some(sample) = entries.get(0) {
                let tag_preview = sample
                    .tags
                    .first()
                    .map(|tag| format!("{}={}", tag.0, tag.1))
                    .unwrap_or_else(|| "-".to_string());
                println!(
                    "[w{worker_id}-inst{}][測試log解析結果] time:{} / tag:{}",
                    inst_id, sample.timestamp, tag_preview
                );
            }

            println!(
                "[w{worker_id}-inst{}][測試log解析結果] time:{} msg:{}",
                inst_id, entries[4].timestamp,entries[2].message
            );
            */
            stats.total_batches += 1;
            stats.total_input_lines += input_lines as u64;
            stats.total_input_bytes += input_bytes as u64;
            stats.total_output_entries += entries.len() as u64;
            // [latency-bench] 時間相關統計（elapsed/component_ns/diff）已隨時間擷取一併註解。
            // stats.total_elapsed += elapsed;
            // stats.total_component_ns += component_ns;
            // if let Some(d) = diff {
            //     stats.total_diff_batches += 1;
            //     stats.total_noop_elapsed_ns += d.noop_elapsed_ns;
            //     stats.total_noop_component_ns += d.noop_component_ns;
            //     stats.total_copy_in_ns += d.copy_in_ns;
            //     stats.total_copy_out_ns += d.copy_out_ns;
            // }
            stats.total_grow_count += grow_count;
            stats.total_grow_delta_bytes += grow_delta_bytes;
            // if component_ns > stats.go_heap_peak_max {
            //     stats.go_heap_peak_max = component_ns;
            // }
            if wasm_mem_peak > stats.wasm_mem_peak_max {
                stats.wasm_mem_peak_max = wasm_mem_peak;
            }

            Some(ParsedBatch {
                entries,
                targettags,
                seq,
            })
        }
        Ok(Err(e)) => {
            eprintln!("[worker{}][parse-error] batch={} {:?}", worker_id, seq, e);
            None
        }
        Err(e) => return Err(e),
    };

    // [latency-bench] print_max 雖為 no-op，但呼叫本身仍有開銷，故註解。
    // inst.store.data().limiter.print_max("parse");
    batch.clear();
    Ok(result)
}

// [latency-bench] measure_noop_parse / duration_ns 整段為時間擷取（noop 差分量測），
//                 回傳 ParseDiffTiming 的路徑已全部註解，避免干擾延遲量測。
// fn measure_noop_parse(
//     pool: &mut ParsePool,
//     lines: &[String],
//     guest_ns: u64,
//     real_elapsed: Duration,
//     deadline_ticks: u64,
// ) -> wasmtime::Result<ParseDiffTiming> {
//     let mut inst = pool.acquire()?;
//     inst.store.data_mut().limiter.reset_batch_stats();
//
//     let started = Instant::now();
//     inst.store.set_epoch_deadline(deadline_ticks);
//     let call_result = inst.plugin.call_parse(&mut inst.store, lines);
//     let noop_elapsed = started.elapsed();
//
//     match call_result {
//         Ok(Ok(_)) => {
//             // [latency-bench] noop 側的 report_usage 也是額外的 WASM 呼叫，故註解。
//             //                 parse_noop 通常為 None（forwarder.yaml: parse_noop: ~）不會執行到，僅為保險。
//             // let noop_component_ns = inst.plugin.call_report_usage(&mut inst.store).unwrap_or(0);
//             let noop_component_ns: u64 = 0;
//             pool.release_without_reset(inst);
//
//             let noop_elapsed_ns = duration_ns(noop_elapsed);
//             let real_elapsed_ns = duration_ns(real_elapsed);
//             let copy_in_ns = noop_elapsed_ns.saturating_sub(noop_component_ns);
//             let copy_out_ns = real_elapsed_ns
//                 .saturating_sub(guest_ns)
//                 .saturating_sub(copy_in_ns);
//
//             Ok(ParseDiffTiming {
//                 noop_elapsed_ns,
//                 noop_component_ns,
//                 copy_in_ns,
//                 guest_ns,
//                 copy_out_ns,
//             })
//         }
//         Ok(Err(e)) => {
//             pool.release_without_reset(inst);
//             Err(wasmtime::Error::msg(format!(
//                 "noop parse returned error: {:?}",
//                 e
//             )))
//         }
//         Err(e) => {
//             pool.discard_and_replenish(inst);
//             Err(e)
//         }
//     }
// }
//
// fn duration_ns(duration: Duration) -> u64 {
//     duration.as_nanos().min(u64::MAX as u128) as u64
// }

/// 把發生錯誤的原始輸入行連同說明標頭寫入 error.txt。
fn write_error_file(header: &str, lines: &[String]) {
    match File::create("error.txt") {
        Ok(mut file) => {
            let _ = writeln!(file, "{}", header);
            for line in lines {
                let _ = writeln!(file, "{}", line);
            }
        }
        Err(e) => eprintln!("無法寫入 error.txt: {}", e),
    }
}

/// 把發生錯誤的 LogEntry 批次連同說明標頭寫入 error.txt。
fn write_entry_error_file(header: &str, entries: &[LogEntry]) {
    match File::create("error.txt") {
        Ok(mut file) => {
            let _ = writeln!(file, "{}", header);
            for e in entries {
                let tags: Vec<String> =
                    e.tags.iter().map(|(k, v)| format!("{}={}", k, v)).collect();
                let _ = writeln!(
                    file,
                    "id={} ts={} level={:?} msg={} tags=[{}]",
                    e.id,
                    e.timestamp,
                    e.level,
                    e.message,
                    tags.join(",")
                );
            }
        }
        Err(e) => eprintln!("無法寫入 error.txt: {}", e),
    }
}

/// 把發生錯誤的已格式化輸出行（bytes）連同說明標頭寫入 error.txt。
fn write_bytes_error_file(header: &str, lines: &[Vec<u8>]) {
    match File::create("error.txt") {
        Ok(mut file) => {
            let _ = writeln!(file, "{}", header);
            for line in lines {
                let _ = file.write_all(line);
                let _ = writeln!(file);
            }
        }
        Err(e) => eprintln!("無法寫入 error.txt: {}", e),
    }
}

// ── Filter Loop (sync) ───────────────────────────────────────────────────────
//
// 每批建立 fresh store/instance，接收 ParsedBatch 並呼叫 filter()，
// 依 filter-result 中的 keep 欄位決定保留哪些 LogEntry，送出精簡後的 ParsedBatch。

/// Filter 階段主迴圈：每批建立只使用一次的實例並呼叫 filter()，
/// 依 filter-result 的 keep 欄位保留 LogEntry 後送往下游；
/// 呼叫完成即丟棄實例；出錯／trap 時以另一個 fresh instance 重試。
fn filter_loop(
    rx: Receiver<ParsedBatch>,
    tx: SyncSender<ParsedBatch>,
    shared: SharedPlugin,
    mem_limit_bytes: usize,
    stage_timeout: Duration,
) -> wasmtime::Result<FilterStats> {
    static BENCHMARK_HEADER: OnceLock<()> = OnceLock::new();
    static BENCHMARK_CALL: AtomicU64 = AtomicU64::new(0);
    // 建立 filter 用的 store 與 plugin 實例，並回傳建立當下的插件版本。
    let make_filter =
        |shared: &SharedPlugin| -> wasmtime::Result<(Store<MyState>, ReductionPlugin, u64)> {
            let slot = shared.read().unwrap();
            let state = MyState {
                ctx: WasiCtxBuilder::new().inherit_stdio().build(),
                table: ResourceTable::new(),
                limiter: MyLimiter::new(mem_limit_bytes),
                http: WasiHttpCtx::new(),
            };
            let mut store = Store::new(&slot.engine, state);
            store.limiter(|s| &mut s.limiter);
            store.set_epoch_deadline(INSTANTIATE_EPOCH_DEADLINE);
            let plugin = ReductionPlugin::instantiate(&mut store, &slot.component, &slot.linker)?;
            let version = slot.version;
            Ok((store, plugin, version))
        };

    let mut current_version = shared.read().unwrap().version;
    let deadline_ticks = stage_timeout.as_millis() as u64 / 10;
    let mut stats = FilterStats::default();

    loop {
        let new_version = shared.read().unwrap().version;
        if new_version != current_version {
            current_version = new_version;
            eprintln!("[filter] plugin hot-swapped → v{}", current_version);
        }

        match rx.recv() {
            Ok(pb) => {
                let seq = pb.seq;
                let input_count = pb.entries.len();
                let mut entries = pb.entries;
                let mut targettags = pb.targettags;
                const MAX_RETRIES: u32 = 2;
                let mut channel_closed = false;

                'retry: for attempt in 0..=MAX_RETRIES {
                    // 每次嘗試都建立 fresh instance；instantiate 在 H0 之前，
                    // 因此不會被誤算成 component copy-in。
                    let (mut store, plugin, version) = match make_filter(&shared) {
                        Ok(instance) => instance,
                        Err(e) => {
                            eprintln!(
                                "[filter] 建立 fresh instance 失敗 batch={} attempt={}: {}",
                                seq, attempt, e
                            );
                            if attempt == MAX_RETRIES {
                                write_entry_error_file(
                                    &format!("filter建立實例失敗 batch={}", seq),
                                    &entries,
                                );
                            }
                            continue 'retry;
                        }
                    };
                    current_version = version;

                    // [latency-bench] host 端時間擷取（started/elapsed）已註解。
                    //                 elapsed > stage_timeout 的軟性超時檢查隨之停用；
                    //                 卡死的 plugin 仍由 epoch deadline（set_epoch_deadline）以 trap 攔截。
                    // let started = Instant::now();
                    store.set_epoch_deadline(deadline_ticks);
                    store.data_mut().limiter.reset_batch_stats();
                    reset_copy_markers();
                    let host_before = Instant::now();
                    let filter_call = plugin.call_filter(&mut store, &entries);
                    let host_after = Instant::now();
                    let (guest_enter, guest_exit, marker_count) = take_copy_markers();
                    match filter_call {
                        Ok(Ok(filter_results)) => {
                            let call_index = BENCHMARK_CALL.fetch_add(1, Ordering::Relaxed);
                            let warmup = std::env::var("WCM_BENCH_WARMUP")
                                .ok()
                                .and_then(|value| value.parse::<u64>().ok())
                                .unwrap_or(0);
                            if marker_count == 2 && call_index >= warmup {
                                let guest_enter =
                                    guest_enter.expect("first filter time-sign missing");
                                let guest_exit =
                                    guest_exit.expect("second filter time-sign missing");
                                BENCHMARK_HEADER.get_or_init(|| {
                                    println!("FILTER_BENCH,batch_count,round,copy_in_ns,guest_ns,copy_out_ns,total_ns,output_entries,wasm_grow_count,wasm_grow_delta_bytes,wasm_mem_peak_bytes,instance_mode,marker_count");
                                });
                                println!(
                                    "FILTER_BENCH,{},{},{},{},{},{},{},{},{},{},src-fresh,{}",
                                    input_count,
                                    call_index - warmup,
                                    guest_enter.duration_since(host_before).as_nanos(),
                                    guest_exit.duration_since(guest_enter).as_nanos(),
                                    host_after.duration_since(guest_exit).as_nanos(),
                                    host_after.duration_since(host_before).as_nanos(),
                                    filter_results.len(),
                                    store.data().limiter.grow_count,
                                    store.data().limiter.grow_total_delta_bytes,
                                    store.data().limiter.wasm_mem_peak,
                                    marker_count,
                                );
                            }
                            // let elapsed = started.elapsed();
                            //
                            // if elapsed > stage_timeout {
                            //     eprintln!(
                            //         "[filter] batch={} 超時 {}ms > {}ms 嘗試 {}/{}",
                            //         seq, elapsed.as_millis(), stage_timeout.as_millis(),
                            //         attempt + 1, MAX_RETRIES + 1
                            //     );
                            //     if attempt < MAX_RETRIES {
                            //         continue 'retry;
                            //     }
                            //     write_entry_error_file(
                            //         &format!("filter超時失敗 batch={}", seq),
                            //         &entries,
                            //     );
                            //     break 'retry;
                            // }

                            // [latency-bench] filter 的 report_usage 是額外的 WASM 呼叫，故註解。
                            // let component_ns = plugin.call_report_usage(&mut store).unwrap_or(0);
                            // let component_ns: u64 = 0;
                            let wasm_mem_peak = store.data().limiter.wasm_mem_peak;

                            let keep_ids: std::collections::HashSet<u64> = filter_results
                                .iter()
                                .filter(|r| r.keep)
                                .map(|r| r.id)
                                .collect();

                            // std::mem::take 移出資料（留空 Vec），避免在 retry loop 中 move
                            let owned_entries = std::mem::take(&mut entries);
                            let owned_tags = std::mem::take(&mut targettags);

                            let (filtered, kept_tags): (Vec<LogEntry>, Vec<String>) = owned_entries
                                .into_iter()
                                .zip(owned_tags.into_iter())
                                .filter(|(e, _)| keep_ids.contains(&e.id))
                                .unzip();

                            let kept = filtered.len();
                            let dropped = input_count - kept;

                            // [latency-bench] 時間相關參數已自 print_filter_batch 簽名移除。
                            // [perf] 每批次的 stderr 輸出影響 p99，連同簡化版一併註解；除錯時再打開。
                            // print_filter_batch(
                            //     seq,
                            //     input_count,
                            //     kept,
                            //     dropped,
                            // );

                            stats.total_batches += 1;
                            stats.total_input_entries += input_count as u64;
                            stats.total_kept_entries += kept as u64;
                            stats.total_dropped_entries += dropped as u64;
                            // [latency-bench] 時間相關統計已隨時間擷取一併註解。
                            // stats.total_elapsed += elapsed;
                            // stats.total_component_ns += component_ns;
                            if wasm_mem_peak > stats.wasm_mem_peak_max {
                                stats.wasm_mem_peak_max = wasm_mem_peak;
                            }

                            if !filtered.is_empty() {
                                let out = ParsedBatch {
                                    entries: filtered,
                                    targettags: kept_tags,
                                    seq,
                                };
                                if tx.send(out).is_err() {
                                    channel_closed = true;
                                }
                            }
                            break 'retry;
                        }
                        Ok(Err(e)) => {
                            eprintln!(
                                "[filter-error] batch={} 嘗試 {}/{}: {:?}",
                                seq,
                                attempt + 1,
                                MAX_RETRIES + 1,
                                e
                            );
                            if attempt == MAX_RETRIES {
                                write_entry_error_file(
                                    &format!("filter錯誤 batch={}", seq),
                                    &entries,
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!(
                                "[filter-trap] batch={} 嘗試 {}/{}: {}",
                                seq,
                                attempt + 1,
                                MAX_RETRIES + 1,
                                e
                            );
                            if attempt == MAX_RETRIES {
                                write_entry_error_file(
                                    &format!("filter trap batch={}", seq),
                                    &entries,
                                );
                            }
                        }
                    }
                }

                if channel_closed {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    Ok(stats)
}

// ── Format Loop (sync) ────────────────────────────────────────────────────────

/// Format 階段主迴圈：接收（已過濾的）ParsedBatch，依 max_chunk 分塊呼叫 format()，
/// 解析回傳的 length-prefixed frame 成輸出行，組成 FormattedBatch 送往 transport；
/// 支援插件熱更換與實例重用開關，出錯／trap 時丟棄實例並重試。
fn format_loop(
    rx: Receiver<ParsedBatch>,
    tx: SyncSender<FormattedBatch>,
    shared: SharedPlugin,
    mem_limit_bytes: usize,
    max_chunk: usize,
    stage_timeout: Duration,
    instance_reuse: bool,
) -> wasmtime::Result<FormatStats> {
    static BENCHMARK_HEADER: OnceLock<()> = OnceLock::new();
    static BENCHMARK_CALL: AtomicU64 = AtomicU64::new(0);
    let mut current_version = shared.read().unwrap().version;
    let deadline_ticks = stage_timeout.as_millis() as u64 / 10;
    let mut stats = FormatStats::default();

    // instance_reuse=true 時跨 batch 快取實例；出錯、trap、熱抽換時丟棄重建。
    // instance_reuse=false（GC 語言防 OOM 模式）時每 chunk 用完即丟。
    let mut cached: Option<(Store<MyState>, FormatPlugin)> = None;

    loop {
        match rx.recv() {
            Ok(mut pb) => {
                let seq = pb.seq;
                let entry_count = pb.entries.len();
                const MAX_RETRIES: u32 = 2;
                let mut final_batch: Option<FormattedBatch> = None;
                let mut channel_closed = false;

                'retry: for attempt in 0..=MAX_RETRIES {
                    // 每次嘗試重新快照 engine/component/linker（同時偵測熱抽換）
                    let (engine, component, linker) = {
                        let slot = shared.read().unwrap();
                        if slot.version != current_version {
                            eprintln!("[format] plugin hot-swapped → v{}", slot.version);
                            current_version = slot.version;
                            cached = None; // 舊版實例作廢
                        }
                        (
                            slot.engine.clone(),
                            slot.component.clone(),
                            slot.linker.clone(),
                        )
                    };

                    // [latency-bench] host 端時間擷取（batch_started/elapsed/logic_ns）已註解。
                    // let batch_started = Instant::now();
                    let mut all_lines: Vec<Vec<u8>> = Vec::new();
                    let mut batch_wasm_peak: usize = 0;
                    // let mut batch_logic_ns: u64 = 0;
                    let mut batch_ok = true;

                    for (chunk_idx, chunk) in pb.entries.chunks(max_chunk).enumerate() {
                        // 取得實例：快取為空就新建（instance_reuse=false 時每 chunk 都會是空）
                        if cached.is_none() {
                            let state = MyState {
                                ctx: WasiCtxBuilder::new().inherit_stdio().build(),
                                table: ResourceTable::new(),
                                limiter: MyLimiter::new(mem_limit_bytes),
                                http: WasiHttpCtx::new(),
                            };
                            let mut store = Store::new(&engine, state);
                            store.limiter(|s| &mut s.limiter);
                            store.set_epoch_deadline(INSTANTIATE_EPOCH_DEADLINE);

                            match FormatPlugin::instantiate(&mut store, &component, &linker) {
                                Ok(plugin) => cached = Some((store, plugin)),
                                Err(e) => {
                                    eprintln!(
                                        "[format] 實例化失敗 batch={} chunk={} 嘗試 {}/{}: {}",
                                        seq,
                                        chunk_idx,
                                        attempt + 1,
                                        MAX_RETRIES + 1,
                                        e
                                    );
                                    batch_ok = false;
                                    break;
                                }
                            }
                        }
                        let (store, plugin) = cached.as_mut().unwrap();
                        let mut instance_poisoned = false;

                        store.set_epoch_deadline(deadline_ticks);
                        store.data_mut().limiter.reset_batch_stats();
                        reset_copy_markers();
                        let host_before = Instant::now();
                        let format_call = plugin.call_format(&mut *store, chunk);
                        let host_after = Instant::now();
                        let (guest_enter, guest_exit, marker_count) = take_copy_markers();
                        match format_call {
                            Ok(Ok(bytes)) => {
                                let call_index = BENCHMARK_CALL.fetch_add(1, Ordering::Relaxed);
                                let warmup = std::env::var("WCM_BENCH_WARMUP")
                                    .ok()
                                    .and_then(|value| value.parse::<u64>().ok())
                                    .unwrap_or(0);
                                if marker_count == 2 && call_index >= warmup {
                                    let guest_enter =
                                        guest_enter.expect("first format time-sign missing");
                                    let guest_exit =
                                        guest_exit.expect("second format time-sign missing");
                                    BENCHMARK_HEADER.get_or_init(|| {
                                        println!("FORMAT_BENCH,batch_count,round,copy_in_ns,guest_ns,copy_out_ns,total_ns,output_bytes,wasm_grow_count,wasm_grow_delta_bytes,wasm_mem_peak_bytes,instance_mode,marker_count");
                                    });
                                    println!(
                                        "FORMAT_BENCH,{},{},{},{},{},{},{},{},{},{},{},{}",
                                        chunk.len(),
                                        call_index - warmup,
                                        guest_enter.duration_since(host_before).as_nanos(),
                                        guest_exit.duration_since(guest_enter).as_nanos(),
                                        host_after.duration_since(guest_exit).as_nanos(),
                                        host_after.duration_since(host_before).as_nanos(),
                                        bytes.len(),
                                        store.data().limiter.grow_count,
                                        store.data().limiter.grow_total_delta_bytes,
                                        store.data().limiter.wasm_mem_peak,
                                        if instance_reuse {
                                            "src-reuse"
                                        } else {
                                            "src-fresh"
                                        },
                                        marker_count,
                                    );
                                }
                                // [latency-bench] format 的 report_usage 是額外的 WASM 呼叫，故註解。
                                // let logic_ns = plugin.call_report_usage(&mut store).unwrap_or(0);
                                // let logic_ns: u64 = 0;
                                // batch_logic_ns += logic_ns;
                                let wasm_mem_peak = store.data().limiter.wasm_mem_peak;
                                if wasm_mem_peak > batch_wasm_peak {
                                    batch_wasm_peak = wasm_mem_peak;
                                }
                                // 解析 [4-byte LE 長度][資料] 格式的 frame
                                let mut pos = 0;
                                while pos + 4 <= bytes.len() {
                                    let len = u32::from_le_bytes([
                                        bytes[pos],
                                        bytes[pos + 1],
                                        bytes[pos + 2],
                                        bytes[pos + 3],
                                    ]) as usize;
                                    pos += 4;
                                    if pos + len <= bytes.len() {
                                        all_lines.push(bytes[pos..pos + len].to_vec());
                                        pos += len;
                                    } else {
                                        eprintln!(
                                            "[format] malformed frame batch={} chunk={}: \
                                             declared {} bytes but only {} remain",
                                            seq,
                                            chunk_idx,
                                            len,
                                            bytes.len() - pos
                                        );
                                        batch_ok = false;
                                        break;
                                    }
                                }
                            }
                            Ok(Err(_)) => {
                                eprintln!(
                                    "[format-error] batch={} chunk={} 嘗試 {}/{}",
                                    seq,
                                    chunk_idx,
                                    attempt + 1,
                                    MAX_RETRIES + 1
                                );
                                batch_ok = false;
                                instance_poisoned = true;
                            }
                            Err(e) => {
                                eprintln!(
                                    "[format-oom] batch={} chunk={} 嘗試 {}/{}: {:?}",
                                    seq,
                                    chunk_idx,
                                    attempt + 1,
                                    MAX_RETRIES + 1,
                                    e
                                );
                                batch_ok = false;
                                instance_poisoned = true;
                            }
                        }
                        // [latency-bench] print_max 雖為 no-op，但呼叫仍有開銷，故註解。
                        // store.data().limiter.print_max("format");

                        // 出錯（實例可能已污染）或停用重用時丟棄，下個 chunk 重建
                        if instance_poisoned || !instance_reuse {
                            cached = None;
                        }

                        if !batch_ok {
                            break;
                        }
                    }

                    // [latency-bench] host 端時間擷取（elapsed）已註解。
                    //                 elapsed > stage_timeout 的軟性超時檢查隨之停用；
                    //                 卡死的 plugin 仍由 epoch deadline 以 trap 攔截。
                    // let elapsed = batch_started.elapsed();

                    if batch_ok && !all_lines.is_empty() {
                        // if elapsed > stage_timeout {
                        //     eprintln!(
                        //         "[format] batch={} 超時 {}ms > {}ms 嘗試 {}/{}",
                        //         seq, elapsed.as_millis(), stage_timeout.as_millis(),
                        //         attempt + 1, MAX_RETRIES + 1
                        //     );
                        //     if attempt < MAX_RETRIES {
                        //         continue 'retry;
                        //     }
                        //     write_entry_error_file(
                        //         &format!("format超時失敗 batch={}", seq),
                        //         &pb.entries,
                        //     );
                        //     break 'retry;
                        // }

                        let output_lines = all_lines.len();
                        let total_bytes: usize = all_lines.iter().map(|l| l.len()).sum();

                        // [latency-bench] 時間相關參數已自 print_format_batch 簽名移除。
                        // [perf] 每批次的 stderr 輸出影響 p99，連同簡化版一併註解；除錯時再打開。
                        // print_format_batch(
                        //     seq,
                        //     entry_count,
                        //     output_lines,
                        // );

                        stats.total_batches += 1;
                        stats.total_input_entries += entry_count as u64;
                        stats.total_output_lines += output_lines as u64;
                        // [latency-bench] 時間相關統計已隨時間擷取一併註解。
                        // stats.total_elapsed += elapsed;
                        // stats.total_component_ns += batch_logic_ns;
                        if batch_wasm_peak > stats.wasm_mem_peak_max {
                            stats.wasm_mem_peak_max = batch_wasm_peak;
                        }

                        debug_assert_eq!(
                            all_lines.len(),
                            pb.targettags.len(),
                            "format: lines/targettags length mismatch (batch={})",
                            seq
                        );
                        final_batch = Some(FormattedBatch {
                            lines: all_lines,
                            // [perf] 原為 pb.targettags.clone()（每批深拷貝整個 Vec<String>）。
                            //        成功時才會走到這裡且隨即 break，直接移出即可，省一次配置。
                            targettags: std::mem::take(&mut pb.targettags),
                            seq,
                            total_bytes,
                        });
                        break 'retry;
                    } else {
                        eprintln!(
                            "[format] batch={} 失敗 嘗試 {}/{}",
                            seq,
                            attempt + 1,
                            MAX_RETRIES + 1
                        );
                        if attempt == MAX_RETRIES {
                            write_entry_error_file(
                                &format!("format失敗 batch={}", seq),
                                &pb.entries,
                            );
                        }
                    }
                }

                if let Some(fb) = final_batch {
                    if tx.send(fb).is_err() {
                        channel_closed = true;
                    }
                }
                if channel_closed {
                    break;
                }
            }
            Err(_) => break,
        }
    }

    Ok(stats)
}

// ── Transport Router (sync) ───────────────────────────────────────────────────
//
// 從 FormattedBatch channel 收線，依 targettag 分流到各 endpoint 的 buffer。
// fan-out："AB" → 同一條 line clone 後推入 buffer_A 和 buffer_B。
// buffer 加入下一行會超過 max_transport_bytes（含 NDJSON 換行）時先送出；
// 否則 drain 到 channel 空（同一波到齊）時把剩餘 buffer 組成 SendTask
// 推給 N workers（無計時器，零額外等待）。

/// Transport router：接收 FormattedBatch，依 targettag 分流到各 endpoint 的 buffer；
/// buffer 加入下一行會超過 max_transport_bytes 時先送出，
/// channel 撈空（同一波到齊）時把剩餘 buffer 組成 SendTask 推給 workers。
fn transport_router(
    rx: Receiver<FormattedBatch>,
    tx_work: std::sync::mpsc::SyncSender<SendTask>,
    cfg: Arc<RwLock<AppConfig>>,
    max_transport_bytes: usize,
) {
    use std::collections::HashMap;

    // key → (buffer, buffer_bytes)
    // Router 不設自己的計時器（drain-then-flush）：批次節奏由入口批次窗
    // （max_wait_ms）決定，router 只負責把「同一波」pipeline 輸出收攏成
    // 一個 POST——channel 撈到空即視為這一波到齊，立刻全部送出，零額外等待。
    // 不能用計時器：跟 max_wait_ms 同值會疊成兩層批次窗（e2e ~2x max_wait_ms），
    // 且與上游節奏的相位漂移會造成週期性延遲尖峰（閒置計時器永不到期）。
    let mut buffers: HashMap<String, (Vec<Vec<u8>>, usize)> = HashMap::new();

    // 把指定 endpoint 的 buffer 內容組成 SendTask 送進工作佇列；找不到 endpoint 時丟棄並警告。
    let flush_buffer = |key: &str,
                        buf: &mut Vec<Vec<u8>>,
                        endpoint_map: &HashMap<String, String>,
                        tx: &std::sync::mpsc::SyncSender<SendTask>| {
        if buf.is_empty() {
            return;
        }
        let url = match endpoint_map.get(key) {
            Some(u) => u.clone(),
            None => {
                eprintln!(
                    "[router] unknown endpoint key '{}', dropping {} lines",
                    key,
                    buf.len()
                );
                buf.clear();
                return;
            }
        };
        let lines = std::mem::take(buf);
        if tx.send(SendTask { url, lines }).is_err() {
            eprintln!("[router] worker channel closed");
        }
    };

    loop {
        // 阻塞等待這一波的第一個訊息
        let first = match rx.recv() {
            Ok(b) => b,
            Err(_) => break, // 上游關閉；buffer 於迴圈外已空（每波結束都清空）
        };

        let endpoint_map = cfg.read().unwrap().endpoint.0.clone();
        let mut disconnected = false;

        // drain：把 channel 裡已排隊的訊息全部撈完（含第一個）
        let mut pending = Some(first);
        loop {
            let batch = match pending.take() {
                Some(b) => b,
                None => match rx.try_recv() {
                    Ok(b) => b,
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                },
            };
            for (line, tag) in batch.lines.into_iter().zip(batch.targettags.iter()) {
                for key in tag.chars().map(|c| c.to_string()) {
                    // 與 transport plugin 的 build_batches 同一套規則：以含 '\n' 的
                    // NDJSON body 計算，且「加入會超過上限」時先 flush。
                    // 這樣每個 task 都 ≤ max_transport_bytes，worker/plugin 不會
                    // 再二次切分（否則會切出 3 筆、1 筆的碎包 POST）。
                    let framed_len = line.len() + 1;
                    let (buf, buf_bytes) = buffers.entry(key.clone()).or_default();
                    if !buf.is_empty() && *buf_bytes + framed_len > max_transport_bytes {
                        flush_buffer(&key, buf, &endpoint_map, &tx_work);
                        *buf_bytes = 0;
                    }
                    *buf_bytes += framed_len;
                    buf.push(line.clone());
                }
            }
        }

        // channel 已空 → 這一波到齊，剩餘 buffer 全部送出
        for (key, (buf, buf_bytes)) in &mut buffers {
            flush_buffer(key, buf, &endpoint_map, &tx_work);
            *buf_bytes = 0;
        }

        if disconnected {
            break;
        }
    }
}

// ── Transport Worker (async) ──────────────────────────────────────────────────

/// 從 `endpoint:` map 裡的 URL 字串（例如 "http://127.0.0.1:8080/ingest"）
/// 解析出 (host, port)，僅供 socket 白名單比對使用。
fn parse_endpoint_host_port(url: &str) -> Option<(String, u16)> {
    let (default_port, rest) = if let Some(r) = url.strip_prefix("https://") {
        (443u16, r)
    } else if let Some(r) = url.strip_prefix("http://") {
        (80u16, r)
    } else {
        return None;
    };

    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() {
        return None;
    }

    if let Some(idx) = authority.rfind(':') {
        let host = &authority[..idx];
        let port: u16 = authority[idx + 1..].parse().ok()?;
        Some((host.to_string(), port))
    } else {
        Some((authority.to_string(), default_port))
    }
}

/// 建立 transport plugin 專用的 socket 白名單檢查：只允許連到當下
/// `forwarder.yaml` 的 `endpoint:` map 中列出的 host:port（TCP connect）。
/// 其餘用途（bind / UDP）一律拒絕。plugin 可能用 wasi:sockets 直連（如 C
/// 版），也可能透過 wasi:http（如 Rust 版），兩者最終都會經過這層檢查。
///
/// **安全考量**：因為 host:port 是從 config 動態讀取（支援熱重載），檢查在
/// 每次 connect 時即時比對，而非在 store 建立時固定快照。
fn build_endpoint_socket_check(
    cfg: Arc<RwLock<AppConfig>>,
) -> impl Fn(SocketAddr, SocketAddrUse) -> Pin<Box<dyn Future<Output = bool> + Send + Sync>>
+ Send
+ Sync
+ 'static {
    move |addr: SocketAddr, use_: SocketAddrUse| {
        let cfg = Arc::clone(&cfg);
        Box::pin(async move {
            if !matches!(use_, SocketAddrUse::TcpConnect) {
                return false;
            }
            let endpoint_map = cfg.read().unwrap().endpoint.0.clone();
            for url in endpoint_map.values() {
                let Some((host, port)) = parse_endpoint_host_port(url) else {
                    continue;
                };
                if port != addr.port() {
                    continue;
                }
                if let Ok(ip) = host.parse::<IpAddr>() {
                    if ip == addr.ip() {
                        return true;
                    }
                } else if let Ok(mut resolved) =
                    tokio::net::lookup_host((host.as_str(), port)).await
                {
                    if resolved.any(|a| a.ip() == addr.ip()) {
                        return true;
                    }
                }
            }
            eprintln!(
                "[SECURITY][transport] 拒絕連線：{} 不在 forwarder.yaml 的 endpoint map 中",
                addr
            );
            false
        })
    }
}

/// 建立 transport 用的 Store 與 plugin 實例：
/// WasiCtx 開啟 DNS 查詢並附掛 endpoint socket 白名單檢查，再以 async 方式實例化 plugin。
async fn build_transport_store(
    shared: &SharedPlugin,
    mem_limit_bytes: usize,
    cfg: &Arc<RwLock<AppConfig>>,
) -> wasmtime::Result<(Store<MyState>, TransportPlugin)> {
    let (engine, component, linker) = {
        let slot = shared.read().unwrap();
        (
            slot.engine.clone(),
            slot.component.clone(),
            slot.linker.clone(),
        )
    };
    let state = MyState {
        ctx: WasiCtxBuilder::new()
            .inherit_stdio()
            .allow_ip_name_lookup(true)
            .socket_addr_check(build_endpoint_socket_check(Arc::clone(cfg)))
            .build(),
        table: ResourceTable::new(),
        limiter: MyLimiter::new(mem_limit_bytes),
        http: WasiHttpCtx::new(),
    };
    let mut store = Store::new(&engine, state);
    store.limiter(|s| &mut s.limiter);
    store.set_epoch_deadline(INSTANTIATE_EPOCH_DEADLINE);
    let plugin = TransportPlugin::instantiate_async(&mut store, &component, &linker).await?;
    Ok((store, plugin))
}

/// Transport worker：從工作佇列搶 SendTask，必要時建立／重用 WASM 實例並對 endpoint 執行 init，
/// 把任務切成 ≤ max_chunk_bytes 的 chunk 逐一送出；失敗時重試並寫入 error.txt，
/// 出錯或停用實例重用時於下個 task 重建實例。
async fn transport_worker(
    rx: Arc<Mutex<std::sync::mpsc::Receiver<SendTask>>>,
    shared: SharedPlugin,
    mem_limit_bytes: usize,
    max_chunk_bytes: usize,
    id: usize,
    stage_timeout: Duration,
    cfg: Arc<RwLock<AppConfig>>,
    instance_reuse: bool,
) -> wasmtime::Result<TransportStats> {
    // 依指定 endpoint 產生 transport plugin 的初始化設定。
    let make_config = |ep: &str| TransportConfig {
        endpoint: ep.to_string(),
        auth: AuthMethod::None,
        connect_timeout_ms: 5_000,
        request_timeout_ms: 30_000,
        retry: None,
        tls: None,
        extra_headers: vec![],
        // 傳入 yaml 的 max_transport_bytes 作為 plugin 每個 POST 的上限。
        // 注意：plugin 必須「先 handle() 再串流寫 body」（transport_http_stream），
        // 舊版 transport_http 是寫完 body 才 handle()，body >4096B 會在
        // wasmtime 的 outgoing-body 緩衝上 deadlock（epoch timeout 攔不住）。
        max_batch_bytes: max_chunk_bytes as u32,
    };

    let deadline_ticks = stage_timeout.as_millis() as u64 / 10;
    let mut stats = TransportStats::default();

    // instance_reuse=true 時跨 task 快取實例（連同已 init 的 endpoint 與插件版本）；
    // 出錯、trap、熱抽換時丟棄重建。false（GC 語言防 OOM 模式）時每 task 用完即丟。
    // 注意：重用時 WasiCtx 的 socket 白名單是建立當下的快照，endpoint map 改動後
    // 由熱抽換／出錯重建路徑更新。
    let mut cached: Option<(Store<MyState>, TransportPlugin, String, u64)> = None;

    loop {
        let task = match rx.lock().unwrap().recv() {
            Ok(t) => t,
            Err(_) => break,
        };

        // 熱抽換偵測：插件版本變更時作廢快取
        let slot_version = shared.read().unwrap().version;
        if cached
            .as_ref()
            .is_some_and(|(_, _, _, v)| *v != slot_version)
        {
            eprintln!("[transport-{}] plugin hot-swapped → v{}", id, slot_version);
            cached = None;
        }

        if cached.is_none() {
            // 建立新實例並 init
            let (mut store, plugin) =
                match build_transport_store(&shared, mem_limit_bytes, &cfg).await {
                    Ok(v) => v,
                    Err(e) => {
                        eprintln!("[transport-{}] build store failed: {}", id, e);
                        continue;
                    }
                };

            store.set_epoch_deadline(deadline_ticks);
            match plugin.call_init(&mut store, &make_config(&task.url)).await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    eprintln!("[transport-{}] init failed url={}: {:?}", id, task.url, e);
                    continue;
                }
                Err(e) => {
                    eprintln!("[transport-{}] init trap url={}: {}", id, task.url, e);
                    continue;
                }
            }
            cached = Some((store, plugin, task.url.clone(), slot_version));
        } else if cached.as_ref().unwrap().2 != task.url {
            // endpoint 改變：同一實例重新 init（plugin 將重複 init 視為重新設定）
            let mut reinit_ok = false;
            {
                let (store, plugin, url, _) = cached.as_mut().unwrap();
                store.set_epoch_deadline(deadline_ticks);
                match plugin.call_init(&mut *store, &make_config(&task.url)).await {
                    Ok(Ok(())) => {
                        *url = task.url.clone();
                        reinit_ok = true;
                    }
                    Ok(Err(e)) => {
                        eprintln!(
                            "[transport-{}] re-init failed url={}: {:?}",
                            id, task.url, e
                        );
                    }
                    Err(e) => {
                        eprintln!("[transport-{}] re-init trap url={}: {}", id, task.url, e);
                    }
                }
            }
            if !reinit_ok {
                cached = None;
                continue;
            }
        }
        let (store, plugin, _, _) = cached.as_mut().unwrap();

        // [latency-bench] host 端時間擷取（batch_started/elapsed）已註解。
        // let batch_started = Instant::now();
        let mut batch_ok = true;
        let mut lines_sent: usize = 0;
        let mut bytes_sent: usize = 0;
        let mut wasm_peak: usize = 0;

        let mut chunk_start = 0;
        while chunk_start < task.lines.len() {
            let mut chunk_end = chunk_start;
            let mut acc_bytes = 0usize;
            while chunk_end < task.lines.len() {
                // 含 '\n' 計算，與 router / plugin build_batches 一致；
                // router 已保證 task ≤ max_transport_bytes，這裡是防禦性上限。
                let line_len = task.lines[chunk_end].len() + 1;
                if acc_bytes > 0 && acc_bytes + line_len > max_chunk_bytes {
                    break;
                }
                acc_bytes += line_len;
                chunk_end += 1;
            }
            let chunk = &task.lines[chunk_start..chunk_end];
            let chunk_bytes = acc_bytes;

            const MAX_RETRIES: u32 = 2;
            let mut chunk_ok = false;

            for attempt in 0..=MAX_RETRIES {
                // [latency-bench] host 端時間擷取（chunk_started/elapsed）已註解。
                //                 elapsed > stage_timeout 的軟性超時檢查隨之停用；
                //                 卡死的 plugin 仍由 epoch deadline 以 trap 攔截。
                // let chunk_started = Instant::now();
                store.set_epoch_deadline(deadline_ticks);
                match plugin.call_transport(&mut *store, chunk).await {
                    Ok(Ok(())) => {
                        // let elapsed = chunk_started.elapsed();
                        // if elapsed > stage_timeout {
                        //     eprintln!(
                        //         "[transport-{}] send 超時 {}ms url={} 嘗試 {}/{}",
                        //         id, elapsed.as_millis(), task.url,
                        //         attempt + 1, MAX_RETRIES + 1
                        //     );
                        //     if attempt < MAX_RETRIES {
                        //         continue;
                        //     }
                        //     write_bytes_error_file(
                        //         &format!("transport超時 url={}", task.url),
                        //         chunk,
                        //     );
                        //     break;
                        // }
                        let peak = store.data().limiter.wasm_mem_peak;
                        if peak > wasm_peak {
                            wasm_peak = peak;
                        }
                        lines_sent += chunk.len();
                        bytes_sent += chunk_bytes;
                        chunk_ok = true;
                        break;
                    }
                    Ok(Err(e)) => {
                        eprintln!(
                            "[transport-{}] send error url={} 嘗試 {}/{}: {:?}",
                            id,
                            task.url,
                            attempt + 1,
                            MAX_RETRIES + 1,
                            e
                        );
                        if attempt == MAX_RETRIES {
                            write_bytes_error_file(
                                &format!("transport send error url={}", task.url),
                                chunk,
                            );
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "[transport-{}] send trap url={} 嘗試 {}/{}: {}",
                            id,
                            task.url,
                            attempt + 1,
                            MAX_RETRIES + 1,
                            e
                        );
                        if attempt == MAX_RETRIES {
                            write_bytes_error_file(
                                &format!("transport trap url={}", task.url),
                                chunk,
                            );
                        }
                    }
                }
            }

            if !chunk_ok {
                batch_ok = false;
                break;
            }
            chunk_start = chunk_end;
        }

        if batch_ok && lines_sent > 0 {
            // [latency-bench] 時間擷取與時間相關統計已註解。
            // let elapsed = batch_started.elapsed();
            //eprintln!(
            //    "[transport-{}] url={} lines={} bytes={} elapsed={:.1}ms",
            //    id, task.url, lines_sent, bytes_sent, elapsed.as_secs_f64() * 1000.0
            //);
            stats.total_batches += 1;
            stats.total_input_lines += lines_sent as u64;
            stats.total_input_bytes += bytes_sent as u64;
            // stats.total_elapsed += elapsed;
            if wasm_peak > stats.wasm_mem_peak_max {
                stats.wasm_mem_peak_max = wasm_peak;
            }
        }

        // 送出失敗（實例可能已污染）或停用重用時丟棄，下個 task 重建
        if !batch_ok || !instance_reuse {
            cached = None;
        }
    }

    Ok(stats)
}
