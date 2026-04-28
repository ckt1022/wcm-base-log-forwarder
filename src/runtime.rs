use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;
use std::fs::File;
use std::io::Write;

use wasmtime::{
    component::{Component, Linker},
    Engine, Store,
};
use wasmtime_wasi::{ResourceTable, WasiCtxBuilder};
use wasmtime_wasi_http::WasiHttpCtx;

use crate::app::{
    format_bindings::FormatPlugin,
    local::log_process::pipeline_process::{LogEntry, ParsedEntry},
    transport_bindings::{
        local::log_process::transport_types::{AuthMethod, TransportConfig},
        TransportPlugin,
    },
    MyLimiter, MyState, ParserPlugin,
};
use crate::config::{
    Batch, BatchConfig, FlushReason, FormatStats, ParseStats, TransportStats,
};
use crate::output::{
    print_flush_header, print_format_batch, print_parse_batch, print_pipeline_summary,
    print_transport_batch,
};

// ── 階段間傳遞的批次結構 ────────────────────────────────────────────────────

struct ParsedBatch {
    entries: Vec<LogEntry>,
    seq: u64,
}

struct FormattedBatch {
    lines: Vec<Vec<u8>>,
    seq: u64,
    total_bytes: usize,
}

// ── 對外入口 ─────────────────────────────────────────────────────────────

/// Pipeline 入口：stdin → [parse thread] → [format thread] → [transport thread]
///
/// Parse / format 使用 sync WASI（typed bindgen）。
/// Transport 使用 async WASI + HTTP，在獨立 single-thread tokio runtime 內執行，
/// 解除 sync 模式下 blocking-write-and-flush ≤ 4096 B 的限制。
pub fn run_pipeline(
    rx_raw: Receiver<String>,
    parse: Option<(Engine, Component, Linker<MyState>)>,
    format: Option<(Engine, Component, Linker<MyState>)>,
    transport: Option<(Engine, Component, Linker<MyState>)>,
    cfg: BatchConfig,
) -> wasmtime::Result<()> {
    // 設定每個instance的最大輸入上限
    let mem_limit_bytes = cfg.mem_limit_mb * 1024 * 1024;

    let (tx_parsed, rx_parsed) = std::sync::mpsc::sync_channel::<ParsedBatch>(20000);
    let (tx_formatted, rx_formatted) = std::sync::mpsc::sync_channel::<FormattedBatch>(20000);

    // Extract fields needed by multiple closures before any move.
    let endpoint = cfg.transport_endpoint.clone();
    let max_format_chunk = cfg.max_format_chunk;
    let max_transport_bytes = cfg.max_transport_bytes;
    let transport_workers = cfg.transport_workers;
    let cfg_for_parse = cfg.clone();

    let wall_start = Instant::now();

    // ── Parse thread (sync) ───────────────────────────────────────────────
    let parse_handle =
    if let Some((engine, component, linker)) = parse {
        Some(thread::spawn(move || {
            parse_loop(rx_raw, tx_parsed, engine, component, linker, cfg_for_parse, mem_limit_bytes)
        }))
    } else {
        drop(tx_parsed);
        thread::spawn(move || { while rx_raw.recv().is_ok() {} });
        None
    };

    // ── Format thread (sync) ──────────────────────────────────────────────
    let format_handle =
    if let Some((engine, component, linker)) = format {
        Some(thread::spawn(move || {
            format_loop(
                rx_parsed, tx_formatted,
                engine, component, linker,
                mem_limit_bytes, max_format_chunk,
            )
        }))
    } else {
        drop(tx_formatted);
        thread::spawn(move || { while rx_parsed.recv().is_ok() {} });
        None
    };

    //println!("checkpoint 1");

    // ── Transport thread (async, N workers) ──────────────────────────────
    // N 個 worker 各自持有獨立 WASM store，共享同一個 rx_formatted（Mutex 保護）。
    // 每個 worker 在自己的 single-thread tokio runtime 中執行，允許同時進行多個 HTTP POST。
    let transport_handle =
    if let Some((engine, component, linker)) = transport {
        Some(thread::spawn(move || {
            //println!("checkpoint 2");
            let rx_shared = Arc::new(Mutex::new(rx_formatted));
            let mut worker_handles = Vec::new();

            for i in 0..transport_workers {
                let rx = Arc::clone(&rx_shared);
                let eng = engine.clone();
                let comp = component.clone();
                let lnk = linker.clone();
                let ep = endpoint.clone();

                worker_handles.push(thread::spawn(move || {
                    tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap()
                        .block_on(transport_worker(
                            rx, eng, comp, lnk,
                            mem_limit_bytes, ep, max_transport_bytes,i
                        ))
                }));
            }

            // 匯總所有 worker 的統計數據
            let mut combined = TransportStats::default();
            for h in worker_handles {
                match h.join() {
                    Ok(Ok(stats)) => {
                        combined.total_batches       += stats.total_batches;
                        combined.total_input_lines   += stats.total_input_lines;
                        combined.total_input_bytes   += stats.total_input_bytes;
                        combined.total_bytes_reported += stats.total_bytes_reported;
                        combined.total_elapsed       += stats.total_elapsed;
                        if stats.wasm_mem_peak_max > combined.wasm_mem_peak_max {
                            combined.wasm_mem_peak_max = stats.wasm_mem_peak_max;
                        }
                    }
                    Ok(Err(e)) => return Err(e),
                    Err(_)    => return Err(wasmtime::Error::msg("transport worker panicked")),
                }
            }
            Ok(combined)
        }))
    } else {
        thread::spawn(move || { while rx_formatted.recv().is_ok() {} });
        None
    };

    // ── Collect results ───────────────────────────────────────────────────
    let parse_stats = parse_handle
        .map(|h| h.join().expect("parse thread panicked"))
        .transpose()?;
    let format_stats = format_handle
        .map(|h| h.join().expect("format thread panicked"))
        .transpose()?;
    let transport_stats = transport_handle
        .map(|h| h.join().expect("transport thread panicked"))
        .transpose()?;

    let wall_elapsed = wall_start.elapsed();

    if let Some(ps) = &parse_stats {
        print_pipeline_summary(ps, format_stats.as_ref(), transport_stats.as_ref(), wall_elapsed, mem_limit_bytes);
    }

    Ok(())
}

fn flush_batch(
    engine: &Engine,
    component: &Component,
    linker: &Linker<MyState>,
    batch: &mut Batch,
    seq: u64,
    mem_limit_bytes: usize,
    reason: &FlushReason,
    stats: &mut ParseStats,
    tx: &SyncSender<ParsedBatch>,
    error_count: &mut u32,
    max_retries: u32,
) -> bool {
    for attempt in 0..=max_retries {
        match do_parse_batch(engine, component, linker, batch, seq, mem_limit_bytes, reason, stats) {
            Ok(Some(pb)) => {
                if attempt == 0 {
                    //println!("time:{} msg:{}", pb.entries[0].timestamp, pb.entries[0].message);
                }
                return tx.send(pb).is_ok();
            }
            Ok(None) => {
                eprintln!("skip batch {}", seq);
                return true;
            }
            Err(e) => {
                eprintln!("[第 {} 次][Parse OOM]: {}", attempt + 1, e);
                if attempt < max_retries {
                    eprintln!("決定重試 ({}/{})", attempt + 1, max_retries);
                } else {
                    eprintln!("已達最大重試次數，寫入 error file，skip batch");
                    *error_count += 1;
                    write_error_file("以下這批是OOM", &batch.lines);
                    batch.clear();
                }
            }
        }
    }

    true
}

/// 寫錯誤檔的小工具，避免重複的 File::create / writeln
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

fn parse_loop(
    rx: Receiver<String>,
    tx: SyncSender<ParsedBatch>,
    engine: Engine,
    component: Component,
    linker: Linker<MyState>,
    cfg: BatchConfig,
    mem_limit_bytes: usize,
) -> wasmtime::Result<ParseStats> {
    let mut batch = Batch::new();
    let mut seq: u64 = 0;
    let safe_data_budget = (mem_limit_bytes as f64 * cfg.safe_data_ratio) as usize;
    let mut stats = ParseStats::default();
    let mut error_count: u32 = 0;

    loop {
        let timeout = if batch.is_empty() {
            cfg.max_wait
        } else {
            cfg.max_wait.saturating_sub(batch.elapsed())
        };

        match rx.recv_timeout(timeout) {
            Ok(item) => {
                let line_len = item.len();
                let size_trigger = !batch.is_empty() && batch.bytes + line_len > safe_data_budget;
                let line_trigger = !batch.is_empty() && batch.len() >= cfg.max_batch_lines;

                if size_trigger || line_trigger {
                    let reason = FlushReason { size: size_trigger, time: false, line_count: line_trigger, eof: false };
                    if !flush_batch(&engine, &component, &linker, &mut batch, seq, mem_limit_bytes, &reason, &mut stats, &tx, &mut error_count,3) {
                        break;
                    }
                    seq += 1;
                }
                batch.push(item);
            }

            Err(RecvTimeoutError::Timeout) => {
                if !batch.is_empty() {
                    let reason = FlushReason { size: false, time: true, line_count: false, eof: false };
                    if !flush_batch(&engine, &component, &linker, &mut batch, seq, mem_limit_bytes, &reason, &mut stats, &tx, &mut error_count,3) {
                        break;
                    }
                    seq += 1;
                }
            }

            Err(RecvTimeoutError::Disconnected) => {
                if !batch.is_empty() {
                    let reason = FlushReason { size: false, time: false, line_count: false, eof: true };
                    flush_batch(&engine, &component, &linker, &mut batch, seq, mem_limit_bytes, &reason, &mut stats, &tx, &mut error_count,3);
                }
                break;
            }
        }
    }

    println!("Error Batch Count = {error_count}");
    Ok(stats)
}

// ── Parse Loop (sync) ─────────────────────────────────────────────────────
// Wasmtime 的資源限制是 store-level
fn parse_loop_(
    rx: Receiver<String>,
    tx: SyncSender<ParsedBatch>,
    engine: Engine,
    component: Component,
    linker: Linker<MyState>,
    cfg: BatchConfig,
    mem_limit_bytes: usize,
) -> wasmtime::Result<ParseStats> {
    let mut batch = Batch::new();
    let mut seq: u64 = 0;
    let safe_data_budget = (mem_limit_bytes as f64 * cfg.safe_data_ratio) as usize;
    let mut stats = ParseStats::default();
    let mut error_count: u32 = 0;

    loop {
        let timeout = if batch.is_empty() {
            cfg.max_wait
        } else {
            cfg.max_wait.saturating_sub(batch.elapsed())
        };

        match rx.recv_timeout(timeout) {
            Ok(item) => {
                let line_len = item.len();
                let size_trigger =
                    !batch.is_empty() && batch.bytes + line_len > safe_data_budget;
                let line_trigger =
                    !batch.is_empty() && batch.len() >= cfg.max_batch_lines;

                if size_trigger || line_trigger {
                    let reason = FlushReason {
                        size: size_trigger, time: false,
                        line_count: line_trigger, eof: false,
                    };
                    // 這裡應該要傳入參考，若發生錯誤，則重試。
                    match do_parse_batch(&engine, &component, &linker, &mut batch, 
                                        seq,mem_limit_bytes, &reason, &mut stats) {
                        Ok(Some(pb)) => {
                            println!("time:{} msg:{}",pb.entries[0].timestamp,pb.entries[0].message);
                            if tx.send(pb).is_err() { break; }
                        }
                        Ok(None) => {
                            // parse 失敗，可以記 log、計數、或跳過
                            eprintln!("skip batch {}", seq);
                        }
                        Err(e) => {
                            // OOM，決定要停止還是繼續
                            eprintln!("[第一次][Parse OOM]: {}", e);
                            eprintln!("決定重試");
                            match do_parse_batch(&engine, &component, &linker, &mut batch, 
                                                seq, mem_limit_bytes, &reason, &mut stats) {
                                Ok(Some(pb)) => {
                                    if tx.send(pb).is_err() { break; }
                                }
                                Ok(None) => {
                                    eprintln!("第一次OOM，第二次Parse Error {}", seq);
                                    error_count += 1;

                                    // 寫入File
                                    let mut file = File::create("error.txt")?;
                                    writeln!(file, "以下這批是Parse Error")?;
                                    for line in &batch.lines {
                                        writeln!(file, "{}", line)?;
                                    }
                                    batch.clear();

                                }
                                Err(e)=>{
                                    eprintln!("[第二次][Parse OOM]: {}", e);
                                    eprintln!("該筆資料寫入error file，skip batch");
                                    error_count += 1;

                                    let mut file = File::create("error.txt")?;
                                    writeln!(file, "以下這批是OOM")?;
                                    for line in &batch.lines {
                                        writeln!(file, "{}", line)?;
                                    }
                                    batch.clear();
                                }
                            }
                        }
                    }
                    seq += 1;
                }
                batch.push(item);
            }
            Err(RecvTimeoutError::Timeout) => {
                if !batch.is_empty() {
                    let reason = FlushReason { size: false, time: true, line_count: false, eof: false };
                    match do_parse_batch(&engine, &component, &linker, &mut batch, 
                                        seq,mem_limit_bytes, &reason, &mut stats) {
                        Ok(Some(pb)) => {
                            if tx.send(pb).is_err() { break; }
                        }
                        Ok(None) => {
                            // parse 失敗，可以記 log、計數、或跳過
                            eprintln!("skip batch {}", seq);
                        }
                        Err(e) => {
                            // OOM，決定要停止還是繼續
                            eprintln!("[第一次][Parse OOM]: {}", e);
                            eprintln!("決定重試");
                            match do_parse_batch(&engine, &component, &linker, &mut batch, 
                                                seq, mem_limit_bytes, &reason, &mut stats) {
                                Ok(Some(pb)) => {
                                    if tx.send(pb).is_err() { break; }
                                }
                                Ok(None) => {
                                    eprintln!("第一次OOM，第二次Parse Error {}", seq);
                                    error_count += 1;
                                    // 寫入File
                                    let mut file = File::create("error.txt")?;
                                    writeln!(file, "以下這批是Parse Error")?;
                                    for line in &batch.lines {
                                        writeln!(file, "{}", line)?;
                                    }
                                    batch.clear();
                                }
                                Err(e)=>{
                                    eprintln!("[第二次][Parse OOM]: {}", e);
                                    eprintln!("該筆資料寫入error file，skip batch");
                                    error_count += 1;
                                    let mut file = File::create("error.txt")?;
                                    writeln!(file, "以下這批是OOM")?;
                                    for line in &batch.lines {
                                        writeln!(file, "{}", line)?;
                                    }
                                    batch.clear();
                                }
                            }
                        }
                    }
                    seq += 1;
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                if !batch.is_empty() {
                    let reason = FlushReason { size: false, time: false, line_count: false, eof: true };
                    match do_parse_batch(&engine, &component, &linker, &mut batch, 
                                        seq,mem_limit_bytes, &reason, &mut stats) {
                        Ok(Some(pb)) => {
                            if tx.send(pb).is_err() { break; }
                        }
                        Ok(None) => {
                            // parse 失敗，可以記 log、計數、或跳過
                            eprintln!("skip batch {}", seq);
                        }
                        Err(e) => {
                            // OOM，決定要停止還是繼續
                            eprintln!("[第一次][Parse OOM]: {}", e);
                            eprintln!("決定重試");
                            match do_parse_batch(&engine, &component, &linker, &mut batch, 
                                                seq, mem_limit_bytes, &reason, &mut stats) {
                                Ok(Some(pb)) => {
                                    if tx.send(pb).is_err() { break; }
                                }
                                Ok(None) => {
                                    eprintln!("第一次OOM，第二次Parse Error {}", seq);
                                    error_count += 1;
                                    // 寫入File
                                    let mut file = File::create("error.txt")?;
                                    writeln!(file, "以下這批是Parse Error")?;
                                    for line in &batch.lines {
                                        writeln!(file, "{}", line)?;
                                    }
                                    batch.clear();
                                }
                                Err(e)=>{
                                    eprintln!("[第二次][Parse OOM]: {}", e);
                                    eprintln!("該筆資料寫入error file，skip batch");
                                    error_count += 1;

                                    let mut file = File::create("error.txt")?;
                                    writeln!(file, "以下這批是OOM")?;
                                    for line in &batch.lines {
                                        writeln!(file, "{}", line)?;
                                    }
                                    batch.clear();
                                }
                            }
                        }
                    }
                }
                break;
            }
        }
    }
    println!("Error Batch Count = {error_count}");
    Ok(stats)
}

fn do_parse_batch(
    engine: &Engine,
    component: &Component,
    linker: &Linker<MyState>,
    batch: &mut Batch,
    seq: u64,
    mem_limit_bytes: usize,
    reason: &FlushReason,
    stats: &mut ParseStats,
) -> wasmtime::Result<Option<ParsedBatch>> {
    let state = MyState {
        ctx: WasiCtxBuilder::new().inherit_stdio().inherit_env().build(),
        table: ResourceTable::new(),
        limiter: MyLimiter::new(mem_limit_bytes),
        http: WasiHttpCtx::new(),
    };
    let mut store = Store::new(engine, state);
    store.limiter(|s| &mut s.limiter);

    let plugin = ParserPlugin::instantiate(&mut store, component, linker)?;

    let input_lines = batch.len();
    let input_bytes = batch.bytes;
    let started = Instant::now();

    let result = match plugin.call_parse(&mut store, &batch.lines) {
        Ok(Ok(parsed)) => {
            let elapsed = started.elapsed();
            // report-usage() 現在回傳 component 內部執行時間（ns）。
            let component_ns = plugin.call_report_usage(&mut store).unwrap_or(0);
            let wasm_mem_peak = store.data().limiter.wasm_mem_peak;
            let grow_count = store.data().limiter.grow_count;
            let grow_delta_bytes = store.data().limiter.grow_total_delta_bytes;

            let entries: Vec<LogEntry> = parsed
                .into_iter()
                .enumerate()
                .map(|(i, p): (usize, ParsedEntry)| LogEntry {
                    id: seq * 100_000 + i as u64,
                    timestamp: p.timestamp,
                    level: p.level,
                    message: p.message,
                    tags: p.tags,
                })
                .collect();

            print_flush_header(seq, batch, &reason);
            print_parse_batch(
                seq, input_lines, input_bytes, entries.len(),
                component_ns, wasm_mem_peak, mem_limit_bytes, elapsed,
                grow_count, grow_delta_bytes,
            );

            stats.total_batches += 1;
            stats.total_input_lines += input_lines as u64;
            stats.total_input_bytes += input_bytes as u64;
            stats.total_output_entries += entries.len() as u64;
            stats.total_elapsed += elapsed;
            if component_ns > stats.go_heap_peak_max { stats.go_heap_peak_max = component_ns; }
            if wasm_mem_peak > stats.wasm_mem_peak_max { stats.wasm_mem_peak_max = wasm_mem_peak; }
            stats.total_grow_count += grow_count;
            stats.total_grow_delta_bytes += grow_delta_bytes;
            stats.total_component_ns += component_ns;

            Some(ParsedBatch { entries, seq })
        }
        Ok(Err(e)) => {
            eprintln!("[parse-error] batch={} {:?}", seq, e);
            None
        }
        Err(e) => {
            //eprintln!("[parse-oom] batch={}", seq);
            //batch.clear();
            return Err(e);
        }
    };

    // memory grow max
    store.data().limiter.print_max("parse");

    batch.clear();
    Ok(result)
}

// ── Format Loop (sync) ────────────────────────────────────────────────────

fn format_loop(
    rx: Receiver<ParsedBatch>,
    tx: SyncSender<FormattedBatch>,
    engine: Engine,
    component: Component,
    linker: Linker<MyState>,
    mem_limit_bytes: usize,
    max_chunk: usize,
) -> wasmtime::Result<FormatStats> {
    let mut stats = FormatStats::default();

    loop {
        match rx.recv() {
            Ok(pb) => {
                let entry_count = pb.entries.len();
                let batch_started = Instant::now();
                let mut all_lines: Vec<Vec<u8>> = Vec::new();
                let mut batch_wasm_peak: usize = 0;
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
                    
                    let plugin = FormatPlugin::instantiate(&mut store, &component, &linker)?;

                    match plugin.call_format(&mut store, chunk) {
                        Ok(Ok(lines)) => {
                            let wasm_mem_peak = store.data().limiter.wasm_mem_peak;
                            if wasm_mem_peak > batch_wasm_peak {
                                batch_wasm_peak = wasm_mem_peak;
                            }
                            all_lines.extend(lines);
                        }
                        Ok(Err(_)) => {
                            eprintln!("[format-error] batch={} chunk={}", pb.seq, chunk_idx);
                            batch_ok = false;
                        }
                        Err(e) => {
                            eprintln!("[format-oom] batch={} chunk={}: {:?}", pb.seq, chunk_idx, e);
                            batch_ok = false;
                        }
                    }
                    // memory grow max
                    store.data().limiter.print_max("format");
                }

                if batch_ok {
                    let elapsed = batch_started.elapsed();
                    let output_lines = all_lines.len();
                    let total_bytes: usize = all_lines.iter().map(|l| l.len()).sum();

                    print_format_batch(pb.seq, entry_count, output_lines,
                                       batch_wasm_peak, mem_limit_bytes, elapsed);

                    stats.total_batches += 1;
                    stats.total_input_entries += entry_count as u64;
                    stats.total_output_lines += output_lines as u64;
                    stats.total_elapsed += elapsed;
                    if batch_wasm_peak > stats.wasm_mem_peak_max {
                        stats.wasm_mem_peak_max = batch_wasm_peak;
                    }

                    let fb = FormattedBatch { lines: all_lines, seq: pb.seq, total_bytes };
                    if tx.send(fb).is_err() { break; }
                }
            }
            Err(_) => break, // parse thread finished
        }
    }

    Ok(stats)
}

// ── Transport Worker (async) ──────────────────────────────────────────────

/// 單一 transport worker — 使用 Val API + async 呼叫 transport-plugin。
///
/// 多個 worker 共享同一 Arc<Mutex<Receiver>>，各自持有獨立 Store，
/// 可同時進行多個 HTTP POST，解除單一 worker 的 serial 限制。
async fn transport_worker(
    rx: Arc<Mutex<Receiver<FormattedBatch>>>,
    engine: Engine,
    component: Component,
    linker: Linker<MyState>,
    mem_limit_bytes: usize,
    endpoint: String,
    max_chunk_bytes: usize,
    _id: usize,
) -> wasmtime::Result<TransportStats> {
    let state = MyState {
        ctx: WasiCtxBuilder::new().inherit_stdio().inherit_env().build(),
        table: ResourceTable::new(),
        limiter: MyLimiter::new(mem_limit_bytes),
        http: WasiHttpCtx::new(),
    };
    let mut store = Store::new(&engine, state);
    store.limiter(|s| &mut s.limiter);

    let plugin = TransportPlugin::instantiate_async(&mut store, &component, &linker).await?;

    // ── init ──────────────────────────────────────────────────────────────
    let config = TransportConfig {
        endpoint: endpoint.clone(),
        auth: AuthMethod::None,
        connect_timeout_ms: 5_000,
        request_timeout_ms: 30_000,
        retry: None,
        tls: None,
        extra_headers: vec![],
        // 0 = host already splits by max_transport_bytes; plugin only does 4096 B writes.
        max_batch_bytes: 4096,
    };
    match plugin.call_init(&mut store, &config).await? {
        Ok(()) => {}
        Err(e) => {
            eprintln!("[transport] init() failed: {:?}", e);
            return Ok(TransportStats::default());
        }
    }
    eprintln!("[transport] init() -> Ok  endpoint={}", endpoint);

    // ── send loop ─────────────────────────────────────────────────────────
    let mut stats = TransportStats::default();

    loop {
        // 從共享 receiver 取得下一個批次：lock → recv → release。
        // 持鎖期間呼叫 blocking recv()；其他 worker 等待鎖。
        // 一旦 batch 到達，此 worker 取走並立即釋放鎖，讓下一個 worker 等待。
        let recv_result = rx.lock().unwrap().recv();
        match recv_result {
            Ok(batch) => {
                let seq = batch.seq;
                let batch_started = Instant::now();
                let mut batch_ok = true;
                let mut batch_lines_sent: usize = 0;
                let mut batch_bytes_sent: usize = 0;
                let mut batch_wasm_peak: usize = 0;

                // 以累積 byte 數為單位分批，而非固定行數。
                // 每一批對應一次 HTTP POST；plugin 內部仍須以 ≤ 4096 B 分批寫入 body。
                let mut chunk_start = 0;
                while chunk_start < batch.lines.len() {
                    let mut chunk_end = chunk_start;
                    let mut acc_bytes = 0usize;
                    while chunk_end < batch.lines.len() {
                        let line_len = batch.lines[chunk_end].len();
                        if acc_bytes > 0 && acc_bytes + line_len > max_chunk_bytes {
                            break;
                        }
                        acc_bytes += line_len;
                        chunk_end += 1;
                    }
                    let chunk = &batch.lines[chunk_start..chunk_end];
                    let chunk_bytes = acc_bytes;

                    // typed bindgen: 直接傳 &[Vec<u8>]，wasmtime 寫入 WASM 線性記憶體，
                    // 省去原本 Val::List(Val::U8 per byte) 的 37x enum 包裝成本。
                    match plugin.call_send(&mut store, chunk).await? {
                        Ok(()) => {
                            let wasm_mem_peak = store.data().limiter.wasm_mem_peak;
                            if wasm_mem_peak > batch_wasm_peak {
                                batch_wasm_peak = wasm_mem_peak;
                            }
                            batch_lines_sent += chunk.len();
                            batch_bytes_sent += chunk_bytes;
                        }
                        Err(e) => {
                            eprintln!("[transport-error] send batch={}: {:?}", seq, e);
                            batch_ok = false;
                            break;
                        }
                    }
                    chunk_start = chunk_end;
                }

                if batch_ok && batch_lines_sent > 0 {
                    let elapsed = batch_started.elapsed();
                    print_transport_batch(seq, batch_lines_sent, batch_bytes_sent,
                                         batch_wasm_peak, mem_limit_bytes, elapsed);
                    stats.total_batches += 1;
                    stats.total_input_lines += batch_lines_sent as u64;
                    stats.total_input_bytes += batch_bytes_sent as u64;
                    stats.total_elapsed += elapsed;
                    if batch_wasm_peak > stats.wasm_mem_peak_max {
                        stats.wasm_mem_peak_max = batch_wasm_peak;
                    }
                }
            }
            Err(_) => break, // format thread finished
        }
    }

    // ── report-usage ──────────────────────────────────────────────────────
    let bytes = plugin.call_report_usage(&mut store).await?;
    stats.total_bytes_reported = bytes;
    eprintln!("[transport] report-usage() -> {} bytes", bytes);

    Ok(stats)
}
