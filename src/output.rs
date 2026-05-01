use std::time::Duration;

use crate::config::{Batch, BatchConfig, FlushReason, FormatStats, ParseStats, TransportStats};
use crate::PipelineStages;

pub fn print_startup(cfg: &BatchConfig, safe_data_budget: usize, stages: &PipelineStages) {
    let stage_str = format!(
        "parse{}{}",
        if stages.format { " → format" } else { "" },
        if stages.transport { " → transport" } else { "" },
    );
    eprintln!("=== WCM Log Forwarder ===");
    eprintln!("Parse Batch Max Size {}",cfg.max_batch_lines);
    eprintln!("Memory Limit : {} MB", cfg.mem_limit_mb);
    eprintln!(
        "Safe Budget  : {} KB ({:.0}% of limit)",
        safe_data_budget / 1024,
        cfg.safe_data_ratio * 100.0
    );
    eprintln!("Stages       : {}", stage_str);
    if stages.transport {
        eprintln!("Endpoint     : {}", cfg.transport_endpoint);
    }

    eprintln!("========================");
}

pub fn print_flush_header(seq: u64, batch: &Batch, reason: &FlushReason) {
    eprintln!(
        "\n--- Flush #{} (size={} time={} lines={} eof={}) | {} lines {} bytes age={}ms ---",
        seq, reason.size, reason.time, reason.line_count, reason.eof,
        batch.len(), batch.bytes, batch.elapsed().as_millis()
    );
}

pub fn print_parse_batch(
    worker_id: usize,
    seq: u64,
    input_lines: usize,
    input_bytes: usize,
    output_entries: usize,
    component_ns: u64,
    wasm_mem_peak: usize,
    mem_limit_bytes: usize,
    elapsed: Duration,
    grow_count: u64,
    grow_delta_bytes: u64,
) {
    let ratio = wasm_mem_peak as f64 / mem_limit_bytes as f64 * 100.0;
    let tput = output_entries as f64 / elapsed.as_secs_f64().max(1e-9);
    let elapsed_ms = elapsed.as_secs_f64() * 1000.0;
    let component_ms = component_ns as f64 / 1_000_000.0;
    let abi_ms = (elapsed_ms - component_ms).max(0.0);
    eprintln!(
        "[parse-w{worker_id} #{seq}] In={input_lines}/{input_bytes}B  Out={output_entries}  \
         Time={elapsed_ms:.2}ms (component={component_ms:.2}ms abi+store={abi_ms:.2}ms)  \
         Grow={grow_count}x/{:.0}KB  WasmMem={:.0}KB({ratio:.1}%)  {tput:.0} entries/s",
        grow_delta_bytes as f64 / 1024.0,
        wasm_mem_peak as f64 / 1024.0,
    );
}

pub fn print_parse_aggregate(stats: &ParseStats, workers: usize, wall: Duration, errors: u32) {
    let wall_tput = stats.total_output_entries as f64 / wall.as_secs_f64().max(1e-9);
    let sum_worker_tput = stats.total_output_entries as f64
        / stats.total_elapsed.as_secs_f64().max(1e-9);
    eprintln!(
        "\n[parse-aggregate] workers={workers}  batches={}  entries={}  errors={errors}",
        stats.total_batches, stats.total_output_entries,
    );
    eprintln!(
        "                  wall={:.3}s  throughput={:.0} entries/s  (wall-clock, all workers)",
        wall.as_secs_f64(),
        wall_tput,
    );
    eprintln!(
        "                  sum-worker-elapsed={:.0}ms  avg-worker-throughput={:.0} entries/s",
        stats.total_elapsed.as_secs_f64() * 1000.0,
        sum_worker_tput,
    );
}

pub fn print_format_batch(
    seq: u64,
    input_entries: usize,
    output_lines: usize,
    wasm_mem_peak: usize,
    mem_limit_bytes: usize,
    elapsed: Duration,
) {
    let ratio = wasm_mem_peak as f64 / mem_limit_bytes as f64 * 100.0;
    let tput = output_lines as f64 / elapsed.as_secs_f64().max(1e-9);
    eprintln!(
        "[format #{seq}] In={input_entries} entries  Out={output_lines} lines  \
         WasmMem={:.0}KB({ratio:.1}%)  Time={:.2}ms  {tput:.0} lines/s",
        wasm_mem_peak as f64 / 1024.0,
        elapsed.as_secs_f64() * 1000.0,
    );
}

pub fn print_transport_batch(
    seq: u64,
    input_lines: usize,
    input_bytes: usize,
    wasm_mem_peak: usize,
    mem_limit_bytes: usize,
    elapsed: Duration,
) {
    let ratio = wasm_mem_peak as f64 / mem_limit_bytes as f64 * 100.0;
    let tput = input_lines as f64 / elapsed.as_secs_f64().max(1e-9);
    eprintln!(
        "[transport #{seq}] In={input_lines} lines/{input_bytes}B  \
         WasmMem={:.0}KB({ratio:.1}%)  Time={:.2}ms  {tput:.0} lines/s",
        wasm_mem_peak as f64 / 1024.0,
        elapsed.as_secs_f64() * 1000.0,
    );
}

pub fn print_pipeline_summary(
    p: &ParseStats,
    f: Option<&FormatStats>,
    t: Option<&TransportStats>,
    wall: Duration,
    mem_limit_bytes: usize,
) {
    let p_tput = if p.total_elapsed.as_secs_f64() > 0.0 {
        p.total_input_lines as f64 / p.total_elapsed.as_secs_f64()
    } else { 0.0 };

    let e2e_tput = if wall.as_secs_f64() > 0.0 {
        p.total_input_lines as f64 / wall.as_secs_f64()
    } else { 0.0 };

    let output_lines = f.map_or(p.total_output_entries, |fs| fs.total_output_lines);

    eprintln!("\n╔══════════════════════════════════════════════════════════╗");
    eprintln!("║                   Pipeline Summary                      ║");
    eprintln!("╠══════════════════════════════════════════════════════════╣");
    eprintln!(
        "║ Parse  │ batches={:<6} input={:<10} entries={:<10}",
        p.total_batches, p.total_input_lines, p.total_output_entries
    );
    eprintln!(
        "║        │ sum-worker-elapsed={:.2}ms  avg-worker-throughput={:.0} lines/s",
        p.total_elapsed.as_secs_f64() * 1000.0, p_tput
    );
    let total_abi_ms = (p.total_elapsed.as_secs_f64() * 1000.0
        - p.total_component_ns as f64 / 1_000_000.0).max(0.0);
    eprintln!(
        "║        │ component={:.0}ms  abi+store={:.0}ms",
        p.total_component_ns as f64 / 1_000_000.0,
        total_abi_ms,
    );
    eprintln!(
        "║        │ grows={} total_delta={:.0}KB  WasmMem(peak)={:.0}KB ({:.1}%)",
        p.total_grow_count,
        p.total_grow_delta_bytes as f64 / 1024.0,
        p.wasm_mem_peak_max as f64 / 1024.0,
        p.wasm_mem_peak_max as f64 / mem_limit_bytes as f64 * 100.0,
    );
    eprintln!("╠══════════════════════════════════════════════════════════╣");
    match f {
        Some(fs) => {
            let f_tput = if fs.total_elapsed.as_secs_f64() > 0.0 {
                fs.total_input_entries as f64 / fs.total_elapsed.as_secs_f64()
            } else { 0.0 };
            eprintln!(
                "║ Format │ batches={:<6} entries={:<10} lines={:<10}",
                fs.total_batches, fs.total_input_entries, fs.total_output_lines
            );
            eprintln!(
                "║        │ elapsed={:.2}ms  throughput={:.0} entries/s",
                fs.total_elapsed.as_secs_f64() * 1000.0, f_tput
            );
            eprintln!(
                "║        │ WasmMem(peak)={:.0}KB ({:.1}%)",
                fs.wasm_mem_peak_max as f64 / 1024.0,
                fs.wasm_mem_peak_max as f64 / mem_limit_bytes as f64 * 100.0,
            );
        }
        None => {
            eprintln!("║ Format │ (disabled)                                           ║");
        }
    }
    eprintln!("╠══════════════════════════════════════════════════════════╣");
    match t {
        Some(ts) => {
            let t_tput = if ts.total_elapsed.as_secs_f64() > 0.0 {
                ts.total_input_lines as f64 / ts.total_elapsed.as_secs_f64()
            } else { 0.0 };
            eprintln!(
                "║ Trans  │ batches={:<6} lines={:<10} bytes={:<12}",
                ts.total_batches, ts.total_input_lines, ts.total_input_bytes
            );
            eprintln!(
                "║        │ elapsed={:.2}ms  throughput={:.0} lines/s",
                ts.total_elapsed.as_secs_f64() * 1000.0, t_tput
            );
            eprintln!(
                "║        │ plugin-reported={} B  WasmMem(peak)={:.0}KB ({:.1}%)",
                ts.total_bytes_reported,
                ts.wasm_mem_peak_max as f64 / 1024.0,
                ts.wasm_mem_peak_max as f64 / mem_limit_bytes as f64 * 100.0,
            );
        }
        None => {
            eprintln!("║ Trans  │ (disabled)                                           ║");
        }
    }
    eprintln!("╠══════════════════════════════════════════════════════════╣");
    eprintln!(
        "║ E2E    │ input={} lines → output={} lines",
        p.total_input_lines, output_lines
    );
    eprintln!(
        "║        │ wall={:.2}ms  throughput={:.0} lines/s (all stages parallel)",
        wall.as_secs_f64() * 1000.0, e2e_tput
    );
    eprintln!("╚══════════════════════════════════════════════════════════╝");
}
