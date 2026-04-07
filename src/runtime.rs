use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::thread;
use std::time::Instant;

use wasmtime::{
    component::{Component, Linker, Val},
    Engine, Store,
};
use wasmtime_wasi::{ResourceTable, WasiCtxBuilder};
use wasmtime_wasi_http::WasiHttpCtx;

use crate::app::{
    format_bindings::FormatPlugin,
    local::log_process::pipeline_process::{LogEntry, ParsedEntry},
    MyLimiter, MyState, ParserPlugin,
};
use crate::config::{
    Batch, BatchConfig, FlushReason, FormatStats, LineItem, ParseStats, TransportStats,
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
    rx_raw: Receiver<LineItem>,
    parse: Option<(Engine, Component, Linker<MyState>)>,
    format: Option<(Engine, Component, Linker<MyState>)>,
    transport: Option<(Engine, Component, Linker<MyState>)>,
    cfg: BatchConfig,
) -> wasmtime::Result<()> {
    let mem_limit_bytes = cfg.mem_limit_mb * 1024 * 1024;

    let (tx_parsed, rx_parsed) = std::sync::mpsc::sync_channel::<ParsedBatch>(32);
    let (tx_formatted, rx_formatted) = std::sync::mpsc::sync_channel::<FormattedBatch>(16);

    // Extract fields needed by multiple closures before any move.
    let endpoint = cfg.transport_endpoint.clone();
    let max_format_chunk = cfg.max_format_chunk;
    let max_transport_bytes = cfg.max_transport_bytes;
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

    // ── Transport thread (async) ──────────────────────────────────────────
    // Transport uses async WASI + HTTP; wrapped in a single-thread tokio runtime
    // so that call_async can drive non-blocking I/O without write-size limits.
    let transport_handle =
    if let Some((engine, component, linker)) = transport {
        Some(thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap()
                .block_on(transport_loop(
                    rx_formatted, engine, component, linker,
                    mem_limit_bytes, endpoint, max_transport_bytes,
                ))
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

// ── Parse Loop (sync) ─────────────────────────────────────────────────────

fn parse_loop(
    rx: Receiver<LineItem>,
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

    loop {
        let timeout = if batch.is_empty() {
            cfg.max_wait
        } else {
            cfg.max_wait.saturating_sub(batch.elapsed())
        };

        match rx.recv_timeout(timeout) {
            Ok(item) => {
                let line_len = item.bytes.len();
                let size_trigger =
                    !batch.is_empty() && batch.bytes + line_len > safe_data_budget;
                let line_trigger =
                    !batch.is_empty() && batch.len() >= cfg.max_batch_lines;

                if size_trigger || line_trigger {
                    let reason = FlushReason {
                        size: size_trigger, time: false,
                        line_count: line_trigger, eof: false,
                    };
                    if let Some(pb) = do_parse_batch(
                        &engine, &component, &linker, &mut batch, seq,
                        mem_limit_bytes, reason, &mut stats,
                    )? {
                        if tx.send(pb).is_err() { break; }
                    }
                    seq += 1;
                }
                batch.push(item.bytes);
            }
            Err(RecvTimeoutError::Timeout) => {
                if !batch.is_empty() {
                    let reason = FlushReason { size: false, time: true, line_count: false, eof: false };
                    if let Some(pb) = do_parse_batch(
                        &engine, &component, &linker, &mut batch, seq,
                        mem_limit_bytes, reason, &mut stats,
                    )? {
                        if tx.send(pb).is_err() { break; }
                    }
                    seq += 1;
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                if !batch.is_empty() {
                    let reason = FlushReason { size: false, time: false, line_count: false, eof: true };
                    if let Some(pb) = do_parse_batch(
                        &engine, &component, &linker, &mut batch, seq,
                        mem_limit_bytes, reason, &mut stats,
                    )? {
                        let _ = tx.send(pb);
                    }
                }
                break;
            }
        }
    }

    Ok(stats)
}

fn do_parse_batch(
    engine: &Engine,
    component: &Component,
    linker: &Linker<MyState>,
    batch: &mut Batch,
    seq: u64,
    mem_limit_bytes: usize,
    reason: FlushReason,
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
            let go_heap_peak = plugin.call_report_usage(&mut store).unwrap_or(0);
            let wasm_mem_peak = store.data().limiter.wasm_mem_peak;

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

            print_flush_header(seq, batch, reason);
            print_parse_batch(seq, input_lines, input_bytes, entries.len(),
                              go_heap_peak, wasm_mem_peak, mem_limit_bytes, elapsed);

            stats.total_batches += 1;
            stats.total_input_lines += input_lines as u64;
            stats.total_input_bytes += input_bytes as u64;
            stats.total_output_entries += entries.len() as u64;
            stats.total_elapsed += elapsed;
            if go_heap_peak > stats.go_heap_peak_max { stats.go_heap_peak_max = go_heap_peak; }
            if wasm_mem_peak > stats.wasm_mem_peak_max { stats.wasm_mem_peak_max = wasm_mem_peak; }

            Some(ParsedBatch { entries, seq })
        }
        Ok(Err(e)) => {
            eprintln!("[parse-error] batch={} {:?}", seq, e);
            None
        }
        Err(e) => {
            eprintln!("[parse-oom] batch={}", seq);
            batch.clear();
            return Err(e);
        }
    };

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

// ── Transport Loop (async) ────────────────────────────────────────────────

/// Transport loop — 使用 Val API + async 呼叫 transport-plugin。
///
/// 執行於 single-thread tokio runtime，透過 `call_async` 驅動 wasi:http 非阻塞 I/O，
/// 解除 sync 模式下 blocking-write-and-flush ≤ 4096 B 的限制。
/// 單一長存活 Store：init 只呼叫一次，send 對每個 chunk 呼叫一次。
async fn transport_loop(
    rx: Receiver<FormattedBatch>,
    engine: Engine,
    component: Component,
    linker: Linker<MyState>,
    mem_limit_bytes: usize,
    endpoint: String,
    max_chunk_bytes: usize,
) -> wasmtime::Result<TransportStats> {
    let state = MyState {
        ctx: WasiCtxBuilder::new().inherit_stdio().inherit_env().build(),
        table: ResourceTable::new(),
        limiter: MyLimiter::new(mem_limit_bytes),
        http: WasiHttpCtx::new(),
    };
    let mut store = Store::new(&engine, state);
    store.limiter(|s| &mut s.limiter);

    let instance = linker.instantiate_async(&mut store, &component).await?;

    // ── init ──────────────────────────────────────────────────────────────
    let init_fn = instance
        .get_func(&mut store, "init")
        .ok_or_else(|| wasmtime::Error::msg("transport component has no export 'init'"))?;

    let config_val = make_transport_config_val(&endpoint);
    let mut init_results = vec![Val::Bool(false)];
    init_fn.call_async(&mut store, &[config_val], &mut init_results).await?;

    if !is_ok_result(&init_results[0]) {
        eprintln!("[transport] init() failed: {:?}", init_results[0]);
        return Ok(TransportStats::default());
    }
    eprintln!("[transport] init() -> Ok  endpoint={}", endpoint);

    // ── send loop ─────────────────────────────────────────────────────────
    let send_fn = instance
        .get_func(&mut store, "send")
        .ok_or_else(|| wasmtime::Error::msg("transport component has no export 'send'"))?;

    let mut stats = TransportStats::default();

    loop {
        // recv() is blocking; in a single-thread tokio runtime with no other tasks
        // this is acceptable — the thread blocks until format stage sends data.
        match rx.recv() {
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
                    let output_data = Val::List(
                        chunk.iter()
                            .map(|line| Val::List(line.iter().map(|&b| Val::U8(b)).collect()))
                            .collect(),
                    );

                    let mut send_results = vec![Val::Bool(false)];

                    //println!("chunk len = {}", chunk.len());

                    match send_fn.call_async(&mut store, &[output_data], &mut send_results).await {
                        Ok(()) => {
                            let wasm_mem_peak = store.data().limiter.wasm_mem_peak;
                            if wasm_mem_peak > batch_wasm_peak {
                                batch_wasm_peak = wasm_mem_peak;
                            }
                            if is_ok_result(&send_results[0]) {
                                batch_lines_sent += chunk.len();
                                batch_bytes_sent += chunk_bytes;
                            } else {
                                eprintln!("[transport-error] send batch={} result={:?}", seq, send_results[0]);
                                batch_ok = false;
                            }
                        }
                        Err(e) => {
                            eprintln!("[transport-trap] send batch={}: {:?}", seq, e);
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
    if let Some(usage_fn) = instance.get_func(&mut store, "report-usage") {
        let mut usage_results = vec![Val::U64(0)];
        if usage_fn.call_async(&mut store, &[], &mut usage_results).await.is_ok() {
            if let Val::U64(bytes) = usage_results[0] {
                stats.total_bytes_reported = bytes;
                eprintln!("[transport] report-usage() -> {} bytes", bytes);
            }
        }
    }

    Ok(stats)
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn make_transport_config_val(endpoint: &str) -> Val {
    Val::Record(vec![
        ("endpoint".into(), Val::String(endpoint.into())),
        ("auth".into(), Val::Variant("none".into(), None)),
        ("connect-timeout-ms".into(), Val::U32(5_000)),
        ("request-timeout-ms".into(), Val::U32(10_000)),
        ("retry".into(), Val::Option(None)),
        ("tls".into(), Val::Option(None)),
        ("extra-headers".into(), Val::List(vec![])),
        ("max-batch-bytes".into(), Val::U32(4096)),
    ])
}

fn is_ok_result(val: &Val) -> bool {
    matches!(val, Val::Result(Ok(_)))
}
