use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex, RwLock};
use std::thread;
use std::time::{Duration, Instant};

use wasmtime::Store;
use wasmtime_wasi::{ResourceTable, WasiCtxBuilder};
use wasmtime_wasi_http::WasiHttpCtx;

use crate::app::{
    MyLimiter, MyState, ParserPlugin, SharedPlugin,
    format_bindings::FormatPlugin,
    local::log_process::pipeline_process::{LogEntry, ParsedEntry},
    reduction_bindings::ReductionPlugin,
    transport_bindings::{
        TransportPlugin,
        local::log_process::transport_types::{AuthMethod, TransportConfig},
    },
};
use crate::config::{
    AppConfig, Batch, BatchConfig, FilterStats, FlushReason, FormatStats, ParseDiffTiming,
    ParseStats, TransportStats,
};
use crate::output::{
    print_filter_batch, print_flush_header, print_format_batch, print_parse_aggregate,
    print_parse_batch, print_pipeline_summary,
};

// ── 階段間傳遞的批次結構 ────────────────────────────────────────────────────

struct ParsedBatch {
    entries: Vec<LogEntry>,
    targettags: Vec<String>,
    seq: u64,
}

struct FormattedBatch {
    lines: Vec<Vec<u8>>,
    targettags: Vec<String>,
    seq: u64,
    total_bytes: usize,
}

struct SendTask {
    url: String,
    lines: Vec<Vec<u8>>,
}

// ── Parse 平行化常數與 WorkBatch ──────────────────────────────────────────────

const PARSE_WORKERS: usize = 1;

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

    fn create_one(&mut self) -> wasmtime::Result<ParseInstance> {
        let slot = self.shared.read().unwrap();
        let state = MyState {
            ctx: WasiCtxBuilder::new().inherit_stdio().inherit_env().build(),
            table: ResourceTable::new(),
            limiter: MyLimiter::new(self.mem_limit_bytes),
            http: WasiHttpCtx::new(),
        };
        let mut store = Store::new(&slot.engine, state);
        store.limiter(|s| &mut s.limiter);
        let plugin = ParserPlugin::instantiate(&mut store, &slot.component, &slot.linker)?;
        drop(slot);
        self.current_count += 1;
        let id = self.current_count;
        Ok(ParseInstance { id, store, plugin })
    }

    /// Detect version bump from config watcher; discard all instances and rebuild.
    fn check_reload(&mut self) {
        let new_version = self.shared.read().unwrap().version;
        if new_version != self.current_version {
            eprintln!(
                "[parse-pool] plugin v{} → v{}: discarding {} instances, rebuilding pool",
                self.current_version, new_version, self.ready.len()
            );
            self.ready.clear();
            self.current_count = 0;
            self.current_version = new_version;
            self.replenish();
            eprintln!("[parse-pool] pool ready ({} instances)", self.ready.len());
        }
    }

    fn replenish(&mut self) {
        while self.ready.len() < self.target_size {
            match self.create_one() {
                Ok(inst) => self.ready.push_back(inst),
                Err(e) => {
                    eprintln!(
                        "[pool] replenish failed ({}/{}): {}",
                        self.ready.len(),
                        self.target_size,
                        e
                    );
                    break;
                }
            }
        }
    }

    fn acquire(&mut self) -> wasmtime::Result<ParseInstance> {
        if let Some(inst) = self.ready.pop_front() {
            Ok(inst)
        } else {
            eprintln!("[pool] pool exhausted, creating emergency instance");
            self.create_one()
        }
    }

    //fn release(&mut self, mut inst: ParseInstance) {
    //    self.ready.push_back(inst);
    //}

    fn release(&mut self, inst: ParseInstance) {
        self.ready.push_back(inst);
        self.replenish();
    }

    fn release_without_reset(&mut self, inst: ParseInstance) {
        self.ready.push_back(inst);
        self.replenish();
    }

    fn discard_and_replenish(&mut self, inst: ParseInstance) {
        drop(inst);
        self.replenish();
    }
}

// ── 對外入口 ──────────────────────────────────────────────────────────────────

/// Pipeline 入口：input → [parse thread] → [filter thread?] → [format thread] → [transport thread]
/// `cfg` is shared with the config-watcher thread; parse_dispatcher reads batch params from it
/// on each iteration, enabling hot-reload of max_wait, max_batch_lines, safe_data_ratio.
pub fn run_pipeline(
    rx_raw: Receiver<String>,
    parse: Option<SharedPlugin>,
    parse_noop: Option<SharedPlugin>,
    filter: Option<SharedPlugin>,
    format: Option<SharedPlugin>,
    transport: Option<SharedPlugin>,
    cfg: Arc<RwLock<AppConfig>>,
) -> wasmtime::Result<()> {
    // Extract fixed startup values from the initial config snapshot.
    let (mem_limit_bytes, max_format_chunk, max_transport_bytes, transport_workers,
         pipeline_channel_capacity, stage_timeout) = {
        let c = cfg.read().unwrap();
        let b = BatchConfig::from(c.batch.clone());
        (
            b.mem_limit_mb * 1024 * 1024,
            b.max_format_chunk,
            b.max_transport_bytes,
            b.transport_workers,
            b.pipeline_channel_capacity,
            Duration::from_millis(b.stage_timeout_ms),
        )
    };

    // parse → filter channel（或直接 parse → format 若 filter 停用）
    let (tx_parsed, rx_parsed) = std::sync::mpsc::sync_channel::<ParsedBatch>(pipeline_channel_capacity);
    // filter → format channel
    let (tx_filtered, rx_filtered) = std::sync::mpsc::sync_channel::<ParsedBatch>(pipeline_channel_capacity);
    let (tx_formatted, rx_formatted) = std::sync::mpsc::sync_channel::<FormattedBatch>(pipeline_channel_capacity);

    let cfg_for_parse = Arc::clone(&cfg);

    let wall_start = Instant::now();

    // ── Parse thread（dispatcher + N workers）────────────────────────────────
    let parse_handle = if let Some(shared) = parse {
        Some(thread::spawn(move || {
            parse_loop(
                rx_raw,
                tx_parsed,
                shared,
                parse_noop,
                cfg_for_parse,
                mem_limit_bytes,
            )
        }))
    } else {
        drop(tx_parsed);
        thread::spawn(move || while rx_raw.recv().is_ok() {});
        None
    };

    // ── Filter thread (sync) ──────────────────────────────────────────────────
    let filter_handle = if let Some(shared) = filter {
        Some(thread::spawn(move || {
            filter_loop(rx_parsed, tx_filtered, shared, mem_limit_bytes, stage_timeout)
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
        Some(thread::spawn(move || {
            format_loop(rx_filtered, tx_formatted, shared, mem_limit_bytes, max_format_chunk, stage_timeout)
        }))
    } else {
        drop(tx_formatted);
        thread::spawn(move || while rx_filtered.recv().is_ok() {});
        None
    };

    // ── Transport thread (router + N async workers) ───────────────────────────
    let transport_handle = if let Some(shared) = transport {
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

            // N transport workers：搶 SendTask，每次建立新 WASM 實例發送。
            let mut worker_handles = Vec::new();
            for i in 0..transport_workers {
                let rx = Arc::clone(&rx_work_shared);
                let sh = Arc::clone(&shared);
                worker_handles.push(thread::spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap()
                        .block_on(transport_worker(rx, sh, mem_limit_bytes, max_transport_bytes, i, stage_timeout))
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

    let wall_elapsed = wall_start.elapsed();

    if let Some(ps) = &parse_stats {
        print_pipeline_summary(
            ps,
            filter_stats.as_ref(),
            format_stats.as_ref(),
            transport_stats.as_ref(),
            wall_elapsed,
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

fn parse_loop(
    rx: Receiver<String>,
    tx: SyncSender<ParsedBatch>,
    shared: SharedPlugin,
    parse_noop: Option<SharedPlugin>,
    cfg: Arc<RwLock<AppConfig>>,
    mem_limit_bytes: usize,
) -> wasmtime::Result<ParseStats> {
    let wall_start = Instant::now();

    let (tx_work, rx_work) = std::sync::mpsc::sync_channel::<WorkBatch>(64);
    let rx_work = Arc::new(Mutex::new(rx_work));

    // 啟動 PARSE_WORKERS 個 worker thread
    let mut worker_handles = Vec::new();
    for worker_id in 0..PARSE_WORKERS {
        let rx_w = Arc::clone(&rx_work);
        let tx_p = tx.clone();
        let sh = Arc::clone(&shared);
        let noop = parse_noop.as_ref().map(Arc::clone);
        worker_handles.push(thread::spawn(move || {
            parse_worker(worker_id, rx_w, tx_p, sh, noop, mem_limit_bytes)
        }));
    }

    // dispatcher 跑在目前 thread，結束後 tx_work 自動 drop
    parse_dispatcher(rx, tx_work, cfg);

    // 彙總 workers 的統計
    let wall_elapsed = wall_start.elapsed();
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

    print_parse_aggregate(&combined, PARSE_WORKERS, wall_elapsed, total_errors);
    Ok(combined)
}

// ── Parse：dispatcher ─────────────────────────────────────────────────────────
//
// 負責從 rx_raw 累積行、觸發 flush 條件後把 WorkBatch 送進 tx_work。
// 不碰 WASM，只做 batching 邏輯。

fn parse_dispatcher(
    rx: Receiver<String>,
    tx: SyncSender<WorkBatch>,
    cfg: Arc<RwLock<AppConfig>>,
) {
    let mut batch = Batch::new();
    let mut seq: u64 = 0;

    loop {
        // Read hot-reloadable params once per recv cycle (lock is extremely brief).
        let (max_wait, safe_data_budget, max_batch_lines) = {
            let c = cfg.read().unwrap();
            let mem = c.batch.mem_limit_mb * 1024 * 1024;
            (
                Duration::from_millis(c.batch.max_wait_ms),
                (mem as f64 * c.batch.safe_data_ratio) as usize,
                c.batch.max_batch_lines,
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
                let size_trigger = !batch.is_empty() && batch.bytes + line_len > safe_data_budget;
                let line_trigger = !batch.is_empty() && batch.len() >= max_batch_lines;

                if size_trigger || line_trigger {
                    let reason = FlushReason {
                        size: size_trigger,
                        time: false,
                        line_count: line_trigger,
                        eof: false,
                    };
                    if send_work_batch(&mut batch, seq, reason, &tx).is_err() {
                        break;
                    }
                    seq += 1;
                }
                batch.push(item);
            }

            Err(RecvTimeoutError::Timeout) => {
                if !batch.is_empty() {
                    let reason = FlushReason {
                        size: false,
                        time: true,
                        line_count: false,
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
                        line_count: false,
                        eof: true,
                    };
                    let _ = send_work_batch(&mut batch, seq, reason, &tx);
                }
                break;
            }
        }
    }
}

fn send_work_batch(
    batch: &mut Batch,
    seq: u64,
    reason: FlushReason,
    tx: &SyncSender<WorkBatch>,
) -> Result<(), ()> {
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

fn parse_worker(
    worker_id: usize,
    rx: Arc<Mutex<Receiver<WorkBatch>>>,
    tx: SyncSender<ParsedBatch>,
    shared: SharedPlugin,
    parse_noop: Option<SharedPlugin>,
    mem_limit_bytes: usize,
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
        if let Some(np) = &mut noop_pool { np.check_reload(); }

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
            3,
        ) {
            break;
        }
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
                    "[worker{}][OOM {}/{}] batch={}: {}",
                    worker_id,
                    attempt + 1,
                    max_retries + 1,
                    seq,
                    e
                );
                pool.discard_and_replenish(inst);
                if attempt == max_retries {
                    eprintln!(
                        "[worker{}][OOM] exceeded retries, skip batch {}",
                        worker_id, seq
                    );
                    *error_count += 1;
                    write_error_file("以下這批是OOM", &batch.lines);
                    batch.clear();
                }
            }
        }
    }
    true
}

fn do_parse_batch(
    worker_id: usize,
    inst: &mut ParseInstance,
    noop_pool: Option<&mut ParsePool>,
    batch: &mut Batch,
    seq: u64,
    mem_limit_bytes: usize,
    reason: &FlushReason,
    stats: &mut ParseStats,
) -> wasmtime::Result<Option<ParsedBatch>> {
    let input_lines = batch.len();
    let input_bytes = batch.bytes;
    let started = Instant::now();

    let result = match inst.plugin.call_parse(&mut inst.store, &batch.lines) {
        Ok(Ok(parsed)) => {
            let elapsed = started.elapsed();
            let component_ns = inst.plugin.call_report_usage(&mut inst.store).unwrap_or(0);
            let diff = match noop_pool {
                Some(pool) => match measure_noop_parse(pool, &batch.lines, component_ns, elapsed) {
                    Ok(diff) => Some(diff),
                    Err(e) => {
                        eprintln!("[worker{}][diff-warn] batch={} {}", worker_id, seq, e);
                        None
                    }
                },
                None => None,
            };
            let wasm_mem_peak = inst.store.data().limiter.wasm_mem_peak;
            let grow_count = inst.store.data().limiter.grow_count;
            let grow_delta_bytes = inst.store.data().limiter.grow_total_delta_bytes;
            let inst_id = inst.id;

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

            if let Some(first) = entries.first() {
                if let Some(tag) = first.tags.first() {
                    eprintln!("[parse][batch={}] first-tag: {}={}", seq, tag.0, tag.1);
                }
            }
            print_flush_header(seq, batch, reason);
            print_parse_batch(
                worker_id,
                seq,
                input_lines,
                input_bytes,
                entries.len(),
                component_ns,
                wasm_mem_peak,
                mem_limit_bytes,
                elapsed,
                grow_count,
                grow_delta_bytes,
                diff,
            );
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
            stats.total_elapsed += elapsed;
            stats.total_component_ns += component_ns;
            if let Some(d) = diff {
                stats.total_diff_batches += 1;
                stats.total_noop_elapsed_ns += d.noop_elapsed_ns;
                stats.total_noop_component_ns += d.noop_component_ns;
                stats.total_copy_in_ns += d.copy_in_ns;
                stats.total_copy_out_ns += d.copy_out_ns;
            }
            stats.total_grow_count += grow_count;
            stats.total_grow_delta_bytes += grow_delta_bytes;
            if component_ns > stats.go_heap_peak_max {
                stats.go_heap_peak_max = component_ns;
            }
            if wasm_mem_peak > stats.wasm_mem_peak_max {
                stats.wasm_mem_peak_max = wasm_mem_peak;
            }

            Some(ParsedBatch { entries, targettags, seq })
        }
        Ok(Err(e)) => {
            eprintln!("[worker{}][parse-error] batch={} {:?}", worker_id, seq, e);
            None
        }
        Err(e) => return Err(e),
    };

    inst.store.data().limiter.print_max("parse");
    batch.clear();
    Ok(result)
}

fn measure_noop_parse(
    pool: &mut ParsePool,
    lines: &[String],
    guest_ns: u64,
    real_elapsed: Duration,
) -> wasmtime::Result<ParseDiffTiming> {
    let mut inst = pool.acquire()?;
    inst.store.data_mut().limiter.reset_batch_stats();

    let started = Instant::now();
    let call_result = inst.plugin.call_parse(&mut inst.store, lines);
    let noop_elapsed = started.elapsed();

    match call_result {
        Ok(Ok(_)) => {
            let noop_component_ns = inst.plugin.call_report_usage(&mut inst.store).unwrap_or(0);
            pool.release_without_reset(inst);

            let noop_elapsed_ns = duration_ns(noop_elapsed);
            let real_elapsed_ns = duration_ns(real_elapsed);
            let copy_in_ns = noop_elapsed_ns.saturating_sub(noop_component_ns);
            let copy_out_ns = real_elapsed_ns
                .saturating_sub(guest_ns)
                .saturating_sub(copy_in_ns);

            Ok(ParseDiffTiming {
                noop_elapsed_ns,
                noop_component_ns,
                copy_in_ns,
                guest_ns,
                copy_out_ns,
            })
        }
        Ok(Err(e)) => {
            pool.release_without_reset(inst);
            Err(wasmtime::Error::msg(format!(
                "noop parse returned error: {:?}",
                e
            )))
        }
        Err(e) => {
            pool.discard_and_replenish(inst);
            Err(e)
        }
    }
}

fn duration_ns(duration: Duration) -> u64 {
    duration.as_nanos().min(u64::MAX as u128) as u64
}

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

fn write_entry_error_file(header: &str, entries: &[LogEntry]) {
    match File::create("error.txt") {
        Ok(mut file) => {
            let _ = writeln!(file, "{}", header);
            for e in entries {
                let tags: Vec<String> = e.tags.iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect();
                let _ = writeln!(
                    file,
                    "id={} ts={} level={:?} msg={} tags=[{}]",
                    e.id, e.timestamp, e.level, e.message, tags.join(",")
                );
            }
        }
        Err(e) => eprintln!("無法寫入 error.txt: {}", e),
    }
}

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
// 一個長壽 store，迴圈接收 ParsedBatch，呼叫 filter()，
// 依 filter-result 中的 keep 欄位決定保留哪些 LogEntry，送出精簡後的 ParsedBatch。

fn filter_loop(
    rx: Receiver<ParsedBatch>,
    tx: SyncSender<ParsedBatch>,
    shared: SharedPlugin,
    mem_limit_bytes: usize,
    stage_timeout: Duration,
) -> wasmtime::Result<FilterStats> {
    let make_filter = |shared: &SharedPlugin| -> wasmtime::Result<(Store<MyState>, ReductionPlugin, u64)> {
        let slot = shared.read().unwrap();
        let state = MyState {
            ctx: WasiCtxBuilder::new().inherit_stdio().inherit_env().build(),
            table: ResourceTable::new(),
            limiter: MyLimiter::new(mem_limit_bytes),
            http: WasiHttpCtx::new(),
        };
        let mut store = Store::new(&slot.engine, state);
        store.limiter(|s| &mut s.limiter);
        let plugin = ReductionPlugin::instantiate(&mut store, &slot.component, &slot.linker)?;
        let version = slot.version;
        Ok((store, plugin, version))
    };

    let (mut store, mut plugin, mut current_version) = make_filter(&shared)?;
    let mut stats = FilterStats::default();

    loop {
        let new_version = shared.read().unwrap().version;
        if new_version != current_version {
            match make_filter(&shared) {
                Ok((s, p, v)) => {
                    store = s; plugin = p; current_version = v;
                    eprintln!("[filter] plugin hot-swapped → v{}", current_version);
                }
                Err(e) => eprintln!("[filter] plugin reload failed (keeping old): {}", e),
            }
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
                    if attempt > 0 {
                        match make_filter(&shared) {
                            Ok((s, p, v)) => {
                                store = s; plugin = p; current_version = v;
                            }
                            Err(e) => {
                                eprintln!("[filter] 重建失敗 batch={} attempt={}: {}", seq, attempt, e);
                                if attempt == MAX_RETRIES {
                                    write_entry_error_file(
                                        &format!("filter重建失敗 batch={}", seq),
                                        &entries,
                                    );
                                }
                                continue 'retry;
                            }
                        }
                    }

                    let started = Instant::now();
                    match plugin.call_filter(&mut store, &entries) {
                        Ok(Ok(filter_results)) => {
                            let elapsed = started.elapsed();

                            if elapsed > stage_timeout {
                                eprintln!(
                                    "[filter] batch={} 超時 {}ms > {}ms 嘗試 {}/{}",
                                    seq, elapsed.as_millis(), stage_timeout.as_millis(),
                                    attempt + 1, MAX_RETRIES + 1
                                );
                                if attempt < MAX_RETRIES {
                                    continue 'retry;
                                }
                                write_entry_error_file(
                                    &format!("filter超時失敗 batch={}", seq),
                                    &entries,
                                );
                                break 'retry;
                            }

                            let component_ns = plugin.call_report_usage(&mut store).unwrap_or(0);
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

                            print_filter_batch(
                                seq,
                                input_count,
                                kept,
                                dropped,
                                wasm_mem_peak,
                                mem_limit_bytes,
                                elapsed,
                                component_ns,
                            );

                            stats.total_batches += 1;
                            stats.total_input_entries += input_count as u64;
                            stats.total_kept_entries += kept as u64;
                            stats.total_dropped_entries += dropped as u64;
                            stats.total_elapsed += elapsed;
                            stats.total_component_ns += component_ns;
                            if wasm_mem_peak > stats.wasm_mem_peak_max {
                                stats.wasm_mem_peak_max = wasm_mem_peak;
                            }

                            if !filtered.is_empty() {
                                let out = ParsedBatch { entries: filtered, targettags: kept_tags, seq };
                                if tx.send(out).is_err() {
                                    channel_closed = true;
                                }
                            }
                            break 'retry;
                        }
                        Ok(Err(e)) => {
                            eprintln!(
                                "[filter-error] batch={} 嘗試 {}/{}: {:?}",
                                seq, attempt + 1, MAX_RETRIES + 1, e
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
                                seq, attempt + 1, MAX_RETRIES + 1, e
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

fn format_loop(
    rx: Receiver<ParsedBatch>,
    tx: SyncSender<FormattedBatch>,
    shared: SharedPlugin,
    mem_limit_bytes: usize,
    max_chunk: usize,
    stage_timeout: Duration,
) -> wasmtime::Result<FormatStats> {
    let mut current_version = shared.read().unwrap().version;
    let mut stats = FormatStats::default();

    loop {
        match rx.recv() {
            Ok(pb) => {
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
                        }
                        (slot.engine.clone(), slot.component.clone(), slot.linker.clone())
                    };

                    let batch_started = Instant::now();
                    let mut all_lines: Vec<Vec<u8>> = Vec::new();
                    let mut batch_wasm_peak: usize = 0;
                    let mut batch_logic_ns: u64 = 0;
                    let mut batch_ok = true;

                    for (chunk_idx, chunk) in pb.entries.chunks(max_chunk).enumerate() {
                        let state = MyState {
                            ctx: WasiCtxBuilder::new().inherit_stdio().inherit_env().build(),
                            table: ResourceTable::new(),
                            limiter: MyLimiter::new(mem_limit_bytes),
                            http: WasiHttpCtx::new(),
                        };
                        let mut store = Store::new(&engine, state);
                        store.limiter(|s| &mut s.limiter);

                        match FormatPlugin::instantiate(&mut store, &component, &linker) {
                            Ok(plugin) => {
                                match plugin.call_format(&mut store, chunk) {
                                    Ok(Ok(bytes)) => {
                                        let logic_ns = plugin.call_report_usage(&mut store).unwrap_or(0);
                                        batch_logic_ns += logic_ns;
                                        let wasm_mem_peak = store.data().limiter.wasm_mem_peak;
                                        if wasm_mem_peak > batch_wasm_peak {
                                            batch_wasm_peak = wasm_mem_peak;
                                        }
                                        // parse [4-byte LE length][data] frames
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
                                                    seq, chunk_idx, len, bytes.len() - pos
                                                );
                                                batch_ok = false;
                                                break;
                                            }
                                        }
                                    }
                                    Ok(Err(_)) => {
                                        eprintln!(
                                            "[format-error] batch={} chunk={} 嘗試 {}/{}",
                                            seq, chunk_idx, attempt + 1, MAX_RETRIES + 1
                                        );
                                        batch_ok = false;
                                    }
                                    Err(e) => {
                                        eprintln!(
                                            "[format-oom] batch={} chunk={} 嘗試 {}/{}: {:?}",
                                            seq, chunk_idx, attempt + 1, MAX_RETRIES + 1, e
                                        );
                                        batch_ok = false;
                                    }
                                }
                                store.data().limiter.print_max("format");
                            }
                            Err(e) => {
                                eprintln!(
                                    "[format] 實例化失敗 batch={} chunk={} 嘗試 {}/{}: {}",
                                    seq, chunk_idx, attempt + 1, MAX_RETRIES + 1, e
                                );
                                batch_ok = false;
                            }
                        }

                        if !batch_ok {
                            break;
                        }
                    }

                    let elapsed = batch_started.elapsed();

                    if batch_ok && !all_lines.is_empty() {
                        if elapsed > stage_timeout {
                            eprintln!(
                                "[format] batch={} 超時 {}ms > {}ms 嘗試 {}/{}",
                                seq, elapsed.as_millis(), stage_timeout.as_millis(),
                                attempt + 1, MAX_RETRIES + 1
                            );
                            if attempt < MAX_RETRIES {
                                continue 'retry;
                            }
                            write_entry_error_file(
                                &format!("format超時失敗 batch={}", seq),
                                &pb.entries,
                            );
                            break 'retry;
                        }

                        let output_lines = all_lines.len();
                        let total_bytes: usize = all_lines.iter().map(|l| l.len()).sum();

                        print_format_batch(
                            seq,
                            entry_count,
                            output_lines,
                            batch_wasm_peak,
                            mem_limit_bytes,
                            elapsed,
                            batch_logic_ns,
                        );

                        stats.total_batches += 1;
                        stats.total_input_entries += entry_count as u64;
                        stats.total_output_lines += output_lines as u64;
                        stats.total_elapsed += elapsed;
                        stats.total_component_ns += batch_logic_ns;
                        if batch_wasm_peak > stats.wasm_mem_peak_max {
                            stats.wasm_mem_peak_max = batch_wasm_peak;
                        }

                        debug_assert_eq!(
                            all_lines.len(), pb.targettags.len(),
                            "format: lines/targettags length mismatch (batch={})", seq
                        );
                        final_batch = Some(FormattedBatch {
                            lines: all_lines,
                            targettags: pb.targettags.clone(),
                            seq,
                            total_bytes,
                        });
                        break 'retry;
                    } else {
                        eprintln!(
                            "[format] batch={} 失敗 嘗試 {}/{}",
                            seq, attempt + 1, MAX_RETRIES + 1
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
// buffer 達 max_transport_bytes 或 timeout → 組成 SendTask 推給 N workers。

fn transport_router(
    rx: Receiver<FormattedBatch>,
    tx_work: std::sync::mpsc::SyncSender<SendTask>,
    cfg: Arc<RwLock<AppConfig>>,
    max_transport_bytes: usize,
) {
    use std::collections::HashMap;

    // key → (buffer, buffer_bytes)
    let mut buffers: HashMap<String, (Vec<Vec<u8>>, usize)> = HashMap::new();
    let max_wait = Duration::from_millis(
        cfg.read().unwrap().batch.max_wait_ms,
    );

    let flush_buffer = |key: &str,
                        buf: &mut Vec<Vec<u8>>,
                        endpoint_map: &HashMap<String, String>,
                        tx: &std::sync::mpsc::SyncSender<SendTask>| {
        if buf.is_empty() { return; }
        let url = match endpoint_map.get(key) {
            Some(u) => u.clone(),
            None => {
                eprintln!("[router] unknown endpoint key '{}', dropping {} lines", key, buf.len());
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
        match rx.recv_timeout(max_wait) {
            Ok(batch) => {
                let endpoint_map = cfg.read().unwrap().endpoint.0.clone();
                for (line, tag) in batch.lines.into_iter().zip(batch.targettags.iter()) {
                    for key in tag.chars().map(|c| c.to_string()) {
                        let (buf, buf_bytes) = buffers.entry(key.clone()).or_default();
                        *buf_bytes += line.len();
                        buf.push(line.clone());
                        if *buf_bytes >= max_transport_bytes {
                            let (buf, buf_bytes) = buffers.get_mut(&key).unwrap();
                            flush_buffer(&key, buf, &endpoint_map, &tx_work);
                            *buf_bytes = 0;
                        }
                    }
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                let endpoint_map = cfg.read().unwrap().endpoint.0.clone();
                for (key, (buf, buf_bytes)) in &mut buffers {
                    flush_buffer(key, buf, &endpoint_map, &tx_work);
                    *buf_bytes = 0;
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                let endpoint_map = cfg.read().unwrap().endpoint.0.clone();
                for (key, (buf, buf_bytes)) in &mut buffers {
                    flush_buffer(key, buf, &endpoint_map, &tx_work);
                    *buf_bytes = 0;
                }
                break;
            }
        }
    }
}

// ── Transport Worker (async) ──────────────────────────────────────────────────

async fn build_transport_store(
    shared: &SharedPlugin,
    mem_limit_bytes: usize,
) -> wasmtime::Result<(Store<MyState>, TransportPlugin)> {
    let (engine, component, linker) = {
        let slot = shared.read().unwrap();
        (slot.engine.clone(), slot.component.clone(), slot.linker.clone())
    };
    let state = MyState {
        ctx: WasiCtxBuilder::new().inherit_stdio().inherit_env().build(),
        table: ResourceTable::new(),
        limiter: MyLimiter::new(mem_limit_bytes),
        http: WasiHttpCtx::new(),
    };
    let mut store = Store::new(&engine, state);
    store.limiter(|s| &mut s.limiter);
    let plugin = TransportPlugin::instantiate_async(&mut store, &component, &linker).await?;
    Ok((store, plugin))
}

async fn transport_worker(
    rx: Arc<Mutex<std::sync::mpsc::Receiver<SendTask>>>,
    shared: SharedPlugin,
    mem_limit_bytes: usize,
    max_chunk_bytes: usize,
    id: usize,
    stage_timeout: Duration,
) -> wasmtime::Result<TransportStats> {
    let make_config = |ep: &str| TransportConfig {
        endpoint: ep.to_string(),
        auth: AuthMethod::None,
        connect_timeout_ms: 5_000,
        request_timeout_ms: 30_000,
        retry: None,
        tls: None,
        extra_headers: vec![],
        max_batch_bytes: 4096,
    };

    let mut stats = TransportStats::default();

    loop {
        let task = match rx.lock().unwrap().recv() {
            Ok(t) => t,
            Err(_) => break,
        };

        // 每個 task 建立獨立 WASM 實例（stateless）
        let (mut store, mut plugin) = match build_transport_store(&shared, mem_limit_bytes).await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[transport-{}] build store failed: {}", id, e);
                continue;
            }
        };

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

        let batch_started = Instant::now();
        let mut batch_ok = true;
        let mut lines_sent: usize = 0;
        let mut bytes_sent: usize = 0;
        let mut wasm_peak: usize = 0;

        let mut chunk_start = 0;
        while chunk_start < task.lines.len() {
            let mut chunk_end = chunk_start;
            let mut acc_bytes = 0usize;
            while chunk_end < task.lines.len() {
                let line_len = task.lines[chunk_end].len();
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
                let chunk_started = Instant::now();
                match plugin.call_send(&mut store, chunk).await {
                    Ok(Ok(())) => {
                        let elapsed = chunk_started.elapsed();
                        if elapsed > stage_timeout {
                            eprintln!(
                                "[transport-{}] send 超時 {}ms url={} 嘗試 {}/{}",
                                id, elapsed.as_millis(), task.url,
                                attempt + 1, MAX_RETRIES + 1
                            );
                            if attempt < MAX_RETRIES {
                                continue;
                            }
                            write_bytes_error_file(
                                &format!("transport超時 url={}", task.url),
                                chunk,
                            );
                            break;
                        }
                        let peak = store.data().limiter.wasm_mem_peak;
                        if peak > wasm_peak { wasm_peak = peak; }
                        lines_sent += chunk.len();
                        bytes_sent += chunk_bytes;
                        chunk_ok = true;
                        break;
                    }
                    Ok(Err(e)) => {
                        eprintln!(
                            "[transport-{}] send error url={} 嘗試 {}/{}: {:?}",
                            id, task.url, attempt + 1, MAX_RETRIES + 1, e
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
                            id, task.url, attempt + 1, MAX_RETRIES + 1, e
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
            let elapsed = batch_started.elapsed();
            //eprintln!(
            //    "[transport-{}] url={} lines={} bytes={} elapsed={:.1}ms",
            //    id, task.url, lines_sent, bytes_sent, elapsed.as_secs_f64() * 1000.0
            //);
            stats.total_batches += 1;
            stats.total_input_lines += lines_sent as u64;
            stats.total_input_bytes += bytes_sent as u64;
            stats.total_elapsed += elapsed;
            if wasm_peak > stats.wasm_mem_peak_max {
                stats.wasm_mem_peak_max = wasm_peak;
            }
        }
    }

    Ok(stats)
}
