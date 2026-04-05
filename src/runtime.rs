use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::thread;
use std::time::Instant;

use wasmtime::{
    component::{Component, Linker},
    Engine, Store,
};
use wasmtime_wasi::{ResourceTable, WasiCtxBuilder};

use crate::app::{
    format_bindings::FormatPlugin,
    local::log_process::pipeline_process::{LogEntry, ParsedEntry},
    MyLimiter, MyState, ParserPlugin,
};
use crate::config::{Batch, BatchConfig, FlushReason, FormatStats, LineItem, ParseStats};
use crate::output::{print_flush_header, print_format_batch, print_parse_batch, print_pipeline_summary};

// 從 parse 送往 format 的批次資料
struct ParsedBatch {
    entries: Vec<LogEntry>,
    seq: u64,
}

// ── 對外入口 ─────────────────────────────────────────────────────────────

/// Pipeline 入口：
///   stdin → parse thread → channel<ParsedBatch> → [format loop] → println!
///
/// `format` 為 `None` 時跳過 format 階段；未來其他可選階段以相同方式擴充。
pub fn run_pipeline(
    rx_raw: Receiver<LineItem>,
    engine_parse: Engine,
    cmp_parse: Component,
    lnk_parse: Linker<MyState>,
    format: Option<(&Engine, &Component, &Linker<MyState>)>,
    cfg: BatchConfig,
) -> wasmtime::Result<()> {
    let mem_limit_bytes = cfg.mem_limit_mb * 1024 * 1024;

    // channel 容量設小（32 batch）：避免 parse 跑太快把大量 LogEntry 堆在記憶體
    let (tx_parsed, rx_parsed) = std::sync::mpsc::sync_channel::<ParsedBatch>(32);

    let wall_start = Instant::now();

    // Parse 在獨立 thread 執行
    let parse_handle = thread::spawn(move || {
        parse_loop(rx_raw, tx_parsed, engine_parse, cmp_parse, lnk_parse, cfg, mem_limit_bytes)
    });

    // Format 在主 thread 執行（可借用 format 元件），或直接捨棄批次
    let fmt_stats = match format {
        Some((engine_fmt, cmp_fmt, lnk_fmt)) => {
            Some(format_loop(rx_parsed, engine_fmt, cmp_fmt, lnk_fmt, mem_limit_bytes, cfg.max_format_chunk)?)
        }
        None => {
            // format 停用：排空 channel 讓 parse thread 能正常結束
            drain_loop(rx_parsed);
            None
        }
    };

    let wall_elapsed = wall_start.elapsed();

    let parse_stats = parse_handle.join().expect("parse thread panicked")?;

    print_pipeline_summary(&parse_stats, fmt_stats.as_ref(), wall_elapsed, mem_limit_bytes);

    Ok(())
}

// ── Parse Loop ───────────────────────────────────────────────────────────

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
                    if let Some(pb) = do_parse_batch(&engine, &component, &linker, &mut batch, seq, mem_limit_bytes, reason, &mut stats)? {
                        if tx.send(pb).is_err() { break; }
                    }
                    seq += 1;
                }
                batch.push(item.bytes);
            }
            Err(RecvTimeoutError::Timeout) => {
                if !batch.is_empty() {
                    let reason = FlushReason { size: false, time: true, line_count: false, eof: false };
                    if let Some(pb) = do_parse_batch(&engine, &component, &linker, &mut batch, seq, mem_limit_bytes, reason, &mut stats)? {
                        if tx.send(pb).is_err() { break; }
                    }
                    seq += 1;
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                if !batch.is_empty() {
                    let reason = FlushReason { size: false, time: false, line_count: false, eof: true };
                    if let Some(pb) = do_parse_batch(&engine, &component, &linker, &mut batch, seq, mem_limit_bytes, reason, &mut stats)? {
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

            // 由 host 分配 id：使用 (seq * MAX_BATCH + index) 保證全域唯一
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

// ── Format Loop ──────────────────────────────────────────────────────────

fn format_loop(
    rx: Receiver<ParsedBatch>,
    engine: &Engine,
    component: &Component,
    linker: &Linker<MyState>,
    mem_limit_bytes: usize,
    max_chunk: usize,
) -> wasmtime::Result<FormatStats> {
    let mut stats = FormatStats::default();

    loop {
        match rx.recv() {
            Ok(pb) => {
                let entry_count = pb.entries.len();
                let batch_started = Instant::now();
                let mut batch_output_lines: usize = 0;
                let mut batch_wasm_peak: usize = 0;
                let mut batch_ok = true;

                // 分成小批次呼叫，避免 TinyGo GC 無法及時回收大批次的中間字串緩衝
                for (chunk_idx, chunk) in pb.entries.chunks(max_chunk).enumerate() {
                    let state = MyState {
                        ctx: WasiCtxBuilder::new().inherit_stdio().inherit_env().build(),
                        table: ResourceTable::new(),
                        limiter: MyLimiter::new(mem_limit_bytes),
                    };
                    let mut store = Store::new(engine, state);
                    store.limiter(|s| &mut s.limiter);

                    let plugin = FormatPlugin::instantiate(&mut store, component, linker)?;

                    match plugin.call_format(&mut store, chunk) {
                        Ok(Ok(lines)) => {
                            let wasm_mem_peak = store.data().limiter.wasm_mem_peak;
                            if wasm_mem_peak > batch_wasm_peak {
                                batch_wasm_peak = wasm_mem_peak;
                            }
                            batch_output_lines += lines.len();
                            
                            /*
                            // 測試format功能
                            let mut count = 0;
                            for line_bytes in &lines {
                                if std::str::from_utf8(line_bytes).is_err() {
                                    eprintln!("[format-warn] batch={} chunk={} non-utf8 skipped", pb.seq, chunk_idx);
                                }
                                if count < 1 {
                                    println!("前五條log: {}",String::from_utf8_lossy(&line_bytes));
                                    count += 1;
                                }
                            }
                            */
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
                    print_format_batch(pb.seq, entry_count, batch_output_lines,
                                       batch_wasm_peak, mem_limit_bytes, elapsed);
                    stats.total_batches += 1;
                    stats.total_input_entries += entry_count as u64;
                    stats.total_output_lines += batch_output_lines as u64;
                    stats.total_elapsed += elapsed;
                    if batch_wasm_peak > stats.wasm_mem_peak_max {
                        stats.wasm_mem_peak_max = batch_wasm_peak;
                    }
                }
            }
            Err(_) => break, // channel 關閉，parse thread 已結束
        }
    }

    Ok(stats)
}

// ── Drain Loop（format 停用時使用）────────────────────────────────────────

/// 排空 ParsedBatch channel，讓 parse thread 不會 block 在 tx.send()。
fn drain_loop(rx: Receiver<ParsedBatch>) {
    while rx.recv().is_ok() {}
}
