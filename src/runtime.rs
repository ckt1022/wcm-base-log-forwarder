use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::Instant;

use wasmtime::{
    component::{Component, Linker},
    Engine, Store,
};
use wasmtime_wasi::{ResourceTable, WasiCtxBuilder};

use crate::app::{MyState, ParserPlugin};
use crate::config::{Batch, BatchConfig, BatchReport, FlushReason, LineItem};
use crate::output::{print_batch_report, print_flush_header};

pub fn run_batch_loop(
    rx: &Receiver<LineItem>,
    engine: &Engine,
    component: &Component,
    linker: &Linker<MyState>,
    cfg: BatchConfig,
) -> wasmtime::Result<()> {
    let mut batch = Batch::new();
    let mut batch_seq: u64 = 0;
    let mut total_input_lines: u64 = 0;

    let mem_limit_bytes = cfg.mem_limit_mb * 1024 * 1024;
    let safe_data_budget = (mem_limit_bytes as f64 * cfg.safe_data_ratio) as usize;

    loop {
        let timeout = if batch.is_empty() {
            cfg.max_wait
        } else {
            cfg.max_wait.saturating_sub(batch.elapsed())
        };

        match rx.recv_timeout(timeout) {
            Ok(item) => {
                let line_len = item.bytes.len();
                let size_trigger = !batch.is_empty() && batch.bytes + line_len > safe_data_budget;
                let line_trigger = !batch.is_empty() && batch.len() >= cfg.max_batch_lines;

                if size_trigger || line_trigger {
                    let processed = batch.len() as u64;
                    process_batch(
                        engine,
                        linker,
                        component,
                        &mut batch,
                        batch_seq,
                        mem_limit_bytes,
                        FlushReason {
                            size: size_trigger,
                            time: false,
                            line_count: line_trigger,
                            eof: false,
                        },
                    )?;
                    total_input_lines += processed;
                    batch_seq += 1;
                }

                batch.push(item.bytes);
            }
            Err(RecvTimeoutError::Timeout) => {
                if !batch.is_empty() {
                    let processed = batch.len() as u64;
                    process_batch(
                        engine,
                        linker,
                        component,
                        &mut batch,
                        batch_seq,
                        mem_limit_bytes,
                        FlushReason {
                            size: false,
                            time: true,
                            line_count: false,
                            eof: false,
                        },
                    )?;
                    total_input_lines += processed;
                    batch_seq += 1;
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                if !batch.is_empty() {
                    let processed = batch.len() as u64;
                    process_batch(
                        engine,
                        linker,
                        component,
                        &mut batch,
                        batch_seq,
                        mem_limit_bytes,
                        FlushReason {
                            size: false,
                            time: false,
                            line_count: false,
                            eof: true,
                        },
                    )?;
                    total_input_lines += processed;
                }
                println!("[done] total_lines_processed={}", total_input_lines);
                break;
            }
        }
    }

    Ok(())
}

fn process_batch(
    engine: &Engine,
    linker: &Linker<MyState>,
    component: &Component,
    batch: &mut Batch,
    batch_seq: u64,
    mem_limit_bytes: usize,
    reason: FlushReason,
) -> wasmtime::Result<()> {
    let state = MyState {
        ctx: WasiCtxBuilder::new().inherit_stdio().inherit_env().build(),
        table: ResourceTable::new(),
        limiter: wasmtime::StoreLimitsBuilder::new()
            .memory_size(mem_limit_bytes)
            .build(),
    };

    let mut store = Store::new(engine, state);
    store.limiter(|s| &mut s.limiter);

    let plugin = ParserPlugin::instantiate(&mut store, component, linker)?;

    let count = batch.len();
    let batch_bytes = batch.bytes;
    let started = Instant::now();

    match plugin.call_parse(&mut store, &batch.lines) {
        Ok(Ok(entries)) => {
            let elapsed = started.elapsed();
            let mem_peak = plugin.call_report_usage(&mut store).unwrap_or(0);

            print_flush_header(batch_seq, batch, reason);
            print_batch_report(BatchReport {
                batch_seq,
                input_lines: count,
                input_bytes: batch_bytes,
                output_lines: entries.len(),
                mem_alloc: mem_peak,
                mem_limit_bytes,
                elapsed,
            });
        }
        Ok(Err(e)) => {
            eprintln!("[wasm-logic-error] batch={} error={e:?}", batch_seq);
        }
        Err(e) => {
            eprintln!("\n!!! [CRITICAL] Memory Overflow at Batch #{} !!!", batch_seq);
            return Err(e);
        }
    }

    batch.clear();
    Ok(())
}