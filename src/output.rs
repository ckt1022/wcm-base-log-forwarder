use std::time::Duration;

use crate::config::{
    PipelineStages,
    Batch, BatchConfig, FilterStats, FlushReason, FormatStats, ParseDiffTiming, ParseStats,
    TransportStats,
};

pub fn print_startup(cfg: &BatchConfig, safe_data_budget: usize, stages: &PipelineStages) {
    let stage_str = format!(
        "parse{}{}{}",
        if stages.filter { " → filter" } else { "" },
        if stages.format { " → format" } else { "" },
        if stages.transport { " → transport" } else { "" },
    );
    eprintln!("=== WCM Log Forwarder ===");
    eprintln!("Parse Batch Max Size {}", cfg.max_batch_lines);
    eprintln!("Memory Limit : {} MB", cfg.mem_limit_mb);
    eprintln!(
        "Safe Budget  : {} KB ({:.0}% of limit)",
        safe_data_budget / 1024,
        cfg.safe_data_ratio * 100.0
    );
    eprintln!("Stages       : {}", stage_str);
    /*
    if stages.transport {
        eprintln!("Endpoint     : {}", cfg.transport_endpoint.as_deref().unwrap_or("(via endpoint map)"));
    }
    */
    eprintln!("========================");
}

// ── Per-batch aligned output ──────────────────────────────────────────────────
//
// All four stage lines share the same fixed-width columns so numbers stay
// aligned both within a stage across batches and across stages within a batch.
//
// Column layout (chars):
//   label   [<9 stage_name> #<4 seq>]          = 17
//   In      In=<6 lines>/<7.0 KB>KB            = 21  (no-KB stages: 10 spaces for the /KB part)
//   Mem     Mem=<7.0 KB>KB(<5.1 %>%)           = 23
//   Time    Time=<7.1 ms>ms                    = 14
//   tput    <7.0>/s                            = 9
//   sep     " | "                              =  3
//   extras  stage-specific fields
//
// Field widths:
//   lines/entries  {:>6}    up to 999 999
//   KB             {:>7.0}  up to ~999 999 KB (~1 GB)
//   time ms        {:>7.1}  up to ~9 999.9 ms
//   pct            {:>5.1}  up to 100.0 %
//   tput           {:>7.0}  up to ~9 999 999 /s

pub fn print_flush_header(seq: u64, batch: &Batch, reason: &FlushReason) {
    let why = if reason.eof             { "eof  " }
              else if reason.size       { "size " }
              else if reason.line_count { "lines" }
              else                      { "time " };
    eprintln!(
        "\n--- Flush #{:>4} [{}] | {:>6} lines / {:>7.0}KB / age={:>5}ms ---",
        seq, why,
        batch.len(),
        batch.bytes as f64 / 1024.0,
        batch.elapsed().as_millis(),
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
    diff: Option<ParseDiffTiming>,
) {
    // [latency-bench] Mem%・comp/abi 分解・Grow 統計は延遲測試に不要。
    //                 毎バッチの format! + eprintln は stderr syscall コストにより p99 に影響する。
    //                 原始詳細版を以下に保留（除錯時はコメントを外して使用）：
    // let ratio      = wasm_mem_peak as f64 / mem_limit_bytes as f64 * 100.0;
    // let tput       = output_entries as f64 / elapsed.as_secs_f64().max(1e-9);
    // let elapsed_ms = elapsed.as_secs_f64() * 1000.0;
    // let comp_ms    = component_ns as f64 / 1_000_000.0;
    // let abi_ms     = (elapsed_ms - comp_ms).max(0.0);
    // eprintln!(
    //     "[{:<9} #{:>4}]  In={:>6}/{:>7.0}KB  Mem={:>7.0}KB({:>5.1}%)  Time={:>7.1}ms  {:>7.0}/s  |  Out={:>6}  comp={:>7.1}ms  abi={:>7.1}ms  Grow={:>4}x/{:>7.0}KB",
    //     format!("parse-w{}", worker_id), seq,
    //     input_lines, input_bytes as f64 / 1024.0,
    //     wasm_mem_peak as f64 / 1024.0, ratio,
    //     elapsed_ms, tput,
    //     output_entries, comp_ms, abi_ms,
    //     grow_count, grow_delta_bytes as f64 / 1024.0,
    // );
    // if let Some(d) = diff {
    //     eprintln!(
    //         "                    diff:  copy-in={:>7.2}ms  guest={:>7.2}ms  copy-out={:>7.2}ms  (noop={:>7.2}ms  noop-guest={:>7.2}ms)",
    //         d.copy_in_ns as f64 / 1_000_000.0,
    //         d.guest_ns as f64 / 1_000_000.0,
    //         d.copy_out_ns as f64 / 1_000_000.0,
    //         d.noop_elapsed_ns as f64 / 1_000_000.0,
    //         d.noop_component_ns as f64 / 1_000_000.0,
    //     );
    // }

    // 延遲測試簡化版：僅輸出吞吐量與批次資訊，移除資源使用量
    let _ = (component_ns, wasm_mem_peak, mem_limit_bytes, grow_count, grow_delta_bytes, diff);
    let tput       = output_entries as f64 / elapsed.as_secs_f64().max(1e-9);
    let elapsed_ms = elapsed.as_secs_f64() * 1000.0;
    eprintln!(
        "[parse-w{} #{:>4}]  In={:>6}/{:>7.0}KB  Time={:>7.1}ms  {:>7.0}/s  Out={:>6}",
        worker_id, seq,
        input_lines, input_bytes as f64 / 1024.0,
        elapsed_ms, tput,
        output_entries,
    );
}

pub fn print_filter_batch(
    seq: u64,
    input_entries: usize,
    kept: usize,
    dropped: usize,
    wasm_mem_peak: usize,
    mem_limit_bytes: usize,
    elapsed: Duration,
    logic_ns: u64,
) {
    // [latency-bench] Mem%・logic_ms は延遲測試に不要。毎バッチの eprintln は p99 に影響する。
    //                 原始詳細版を保留：
    // let ratio      = wasm_mem_peak as f64 / mem_limit_bytes as f64 * 100.0;
    // let elapsed_ms = elapsed.as_secs_f64() * 1000.0;
    // let logic_ms   = logic_ns as f64 / 1_000_000.0;
    // let drop_pct   = dropped as f64 / input_entries.max(1) as f64 * 100.0;
    // let tput       = input_entries as f64 / elapsed.as_secs_f64().max(1e-9);
    // // {:10} with "" fills the /{:>7.0}KB slot (1 + 7 + 2 = 10 chars) with spaces
    // eprintln!(
    //     "[{:<9} #{:>4}]  In={:>6}{:10}  Mem={:>7.0}KB({:>5.1}%)  Time={:>7.1}ms  {:>7.0}/s  |  kept={:>6}  drop={:>6}({:>5.1}%)  logic={:>7.2}ms",
    //     "filter", seq,
    //     input_entries, "",
    //     wasm_mem_peak as f64 / 1024.0, ratio,
    //     elapsed_ms, tput,
    //     kept, dropped, drop_pct, logic_ms,
    // );

    // 延遲測試簡化版：僅輸出 keep/drop 資訊與吞吐量
    let _ = (wasm_mem_peak, mem_limit_bytes, logic_ns);
    let tput     = input_entries as f64 / elapsed.as_secs_f64().max(1e-9);
    let drop_pct = dropped as f64 / input_entries.max(1) as f64 * 100.0;
    eprintln!(
        "[filter  #{:>4}]  In={:>6}  kept={:>6}  drop={:>5.1}%  Time={:>7.1}ms  {:>7.0}/s",
        seq, input_entries, kept, drop_pct, elapsed.as_secs_f64() * 1000.0, tput,
    );
}

pub fn print_format_batch(
    seq: u64,
    input_entries: usize,
    output_lines: usize,
    wasm_mem_peak: usize,
    mem_limit_bytes: usize,
    elapsed: Duration,
    logic_ns: u64,
) {
    // [latency-bench] Mem%・logic/copyin 分解は延遲測試に不要。毎バッチの eprintln は p99 に影響する。
    //                 原始詳細版を保留：
    // let ratio      = wasm_mem_peak as f64 / mem_limit_bytes as f64 * 100.0;
    // let tput       = output_lines as f64 / elapsed.as_secs_f64().max(1e-9);
    // let elapsed_ms = elapsed.as_secs_f64() * 1000.0;
    // let logic_ms   = logic_ns as f64 / 1_000_000.0;
    // let copyin_ms  = (elapsed_ms - logic_ms).max(0.0);
    // eprintln!(
    //     "[{:<9} #{:>4}]  In={:>6}{:10}  Mem={:>7.0}KB({:>5.1}%)  Time={:>7.1}ms  {:>7.0}/s  |  Out={:>6}  logic={:>7.2}ms  copyin={:>7.2}ms",
    //     "format", seq,
    //     input_entries, "",
    //     wasm_mem_peak as f64 / 1024.0, ratio,
    //     elapsed_ms, tput,
    //     output_lines, logic_ms, copyin_ms,
    // );

    // 延遲測試簡化版：僅輸出吞吐量與批次資訊
    let _ = (wasm_mem_peak, mem_limit_bytes, logic_ns);
    let tput = output_lines as f64 / elapsed.as_secs_f64().max(1e-9);
    eprintln!(
        "[format  #{:>4}]  In={:>6}  Out={:>6}  Time={:>7.1}ms  {:>7.0}/s",
        seq, input_entries, output_lines, elapsed.as_secs_f64() * 1000.0, tput,
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
    let tput  = input_lines as f64 / elapsed.as_secs_f64().max(1e-9);

    eprintln!(
        "[{:<9} #{:>4}]  In={:>6}/{:>7.0}KB  Mem={:>7.0}KB({:>5.1}%)  Time={:>7.1}ms  {:>7.0}/s",
        "transport", seq,
        input_lines, input_bytes as f64 / 1024.0,
        wasm_mem_peak as f64 / 1024.0, ratio,
        elapsed.as_secs_f64() * 1000.0, tput,
    );
}

// ── Aggregate / summary output ────────────────────────────────────────────────

pub fn print_parse_aggregate(stats: &ParseStats, workers: usize, wall: Duration, errors: u32) {
    let wall_tput = stats.total_output_entries as f64 / wall.as_secs_f64().max(1e-9);
    // [latency-bench] sum_worker_tput は延遲測試に不要、原始計算を保留：
    // let sum_worker_tput =
    //     stats.total_output_entries as f64 / stats.total_elapsed.as_secs_f64().max(1e-9);
    eprintln!(
        "\n[parse-aggregate] workers={workers}  batches={}  entries={}  errors={errors}",
        stats.total_batches, stats.total_output_entries,
    );
    eprintln!(
        "                  wall={:.3}s  throughput={:.0} entries/s",
        wall.as_secs_f64(),
        wall_tput,
    );
    // [latency-bench] sum-worker 詳細時間と diff copy-in/out 分解は延遲測試に不要。原始版を保留：
    // eprintln!(
    //     "                  sum-worker-elapsed={:.0}ms  avg-worker-throughput={:.0} entries/s",
    //     stats.total_elapsed.as_secs_f64() * 1000.0,
    //     sum_worker_tput,
    // );
    // if stats.total_diff_batches > 0 {
    //     let batches = stats.total_diff_batches as f64;
    //     eprintln!(
    //         "                  avg/batch: copy-in={:.2}ms  logic={:.2}ms  copy-out={:.2}ms",
    //         stats.total_copy_in_ns as f64 / batches / 1_000_000.0,
    //         stats.total_component_ns as f64 / batches / 1_000_000.0,
    //         stats.total_copy_out_ns as f64 / batches / 1_000_000.0,
    //     );
    // }
}

pub fn print_pipeline_summary(
    p: &ParseStats,
    fi: Option<&FilterStats>,
    f: Option<&FormatStats>,
    t: Option<&TransportStats>,
    wall: Duration,
    // [latency-bench] mem_limit_bytes は WasmMem% 計算でのみ使用、コメントアウト後は不要。
    _mem_limit_bytes: usize,
) {
    // [latency-bench] p_tput は sum-worker 出力にのみ使用され、延遲測試では不要。原始版を保留：
    // let p_tput = if p.total_elapsed.as_secs_f64() > 0.0 {
    //     p.total_input_lines as f64 / p.total_elapsed.as_secs_f64()
    // } else {
    //     0.0
    // };

    let e2e_tput = if wall.as_secs_f64() > 0.0 {
        p.total_input_lines as f64 / wall.as_secs_f64()
    } else {
        0.0
    };

    let output_lines = f.map_or(p.total_output_entries, |fs| fs.total_output_lines);

    eprintln!("\n╔══════════════════════════════════════════════════════════╗");
    eprintln!("║                   Pipeline Summary                      ║");
    eprintln!("╠══════════════════════════════════════════════════════════╣");
    eprintln!(
        "║ Parse  │ batches={:<6} input={:<10} entries={:<10}",
        p.total_batches, p.total_input_lines, p.total_output_entries
    );
    // [latency-bench] Parse の詳細タイミング（sum-worker・component/abi・diff・grows/WasmMem）は
    //                 資源使用量の記録で延遲測試に不要。原始版を保留：
    // eprintln!(
    //     "║        │ sum-worker-elapsed={:.2}ms  avg-worker-throughput={:.0} lines/s",
    //     p.total_elapsed.as_secs_f64() * 1000.0,
    //     p_tput
    // );
    // let total_abi_ms = (p.total_elapsed.as_secs_f64() * 1000.0
    //     - p.total_component_ns as f64 / 1_000_000.0)
    //     .max(0.0);
    // eprintln!(
    //     "║        │ component={:.0}ms  abi+store={:.0}ms",
    //     p.total_component_ns as f64 / 1_000_000.0,
    //     total_abi_ms,
    // );
    // if p.total_diff_batches > 0 {
    //     let diff_batches = p.total_diff_batches as f64;
    //     eprintln!(
    //         "║        │ diff-batches={}  copy-in={:.0}ms  guest={:.0}ms  copy-out={:.0}ms",
    //         p.total_diff_batches,
    //         p.total_copy_in_ns as f64 / 1_000_000.0,
    //         p.total_component_ns as f64 / 1_000_000.0,
    //         p.total_copy_out_ns as f64 / 1_000_000.0,
    //     );
    //     eprintln!(
    //         "║        │ avg/batch copy-in={:.2}ms  logic={:.2}ms  copy-out={:.2}ms",
    //         p.total_copy_in_ns as f64 / diff_batches / 1_000_000.0,
    //         p.total_component_ns as f64 / diff_batches / 1_000_000.0,
    //         p.total_copy_out_ns as f64 / diff_batches / 1_000_000.0,
    //     );
    // }
    // eprintln!(
    //     "║        │ grows={} total_delta={:.0}KB  WasmMem(peak)={:.0}KB ({:.1}%)",
    //     p.total_grow_count,
    //     p.total_grow_delta_bytes as f64 / 1024.0,
    //     p.wasm_mem_peak_max as f64 / 1024.0,
    //     p.wasm_mem_peak_max as f64 / mem_limit_bytes as f64 * 100.0,
    // );
    eprintln!("╠══════════════════════════════════════════════════════════╣");
    match fi {
        Some(fs) => {
            let fi_tput = if fs.total_elapsed.as_secs_f64() > 0.0 {
                fs.total_input_entries as f64 / fs.total_elapsed.as_secs_f64()
            } else {
                0.0
            };
            let drop_pct = fs.total_dropped_entries as f64
                / fs.total_input_entries.max(1) as f64
                * 100.0;
            eprintln!(
                "║ Filter │ batches={:<6} in={:<10} kept={:<10} dropped={} ({:.1}%)",
                fs.total_batches,
                fs.total_input_entries,
                fs.total_kept_entries,
                fs.total_dropped_entries,
                drop_pct,
            );
            eprintln!(
                "║        │ elapsed={:.2}ms  throughput={:.0} entries/s",
                fs.total_elapsed.as_secs_f64() * 1000.0,
                fi_tput,
            );
            // [latency-bench] WasmMem% は資源使用量で延遲測試に不要。原始版を保留：
            // eprintln!(
            //     "║        │ WasmMem(peak)={:.0}KB ({:.1}%)",
            //     fs.wasm_mem_peak_max as f64 / 1024.0,
            //     fs.wasm_mem_peak_max as f64 / mem_limit_bytes as f64 * 100.0,
            // );
        }
        None => {
            eprintln!("║ Filter │ (disabled)                                           ║");
        }
    }
    eprintln!("╠══════════════════════════════════════════════════════════╣");
    match f {
        Some(fs) => {
            let f_tput = if fs.total_elapsed.as_secs_f64() > 0.0 {
                fs.total_input_entries as f64 / fs.total_elapsed.as_secs_f64()
            } else {
                0.0
            };
            // [latency-bench] total_logic_ms・total_copy_in_ms は logic/copyin 分解出力でのみ使用、延遲測試に不要。
            // let total_logic_ms = fs.total_component_ns as f64 / 1_000_000.0;
            // let total_copy_in_ms =
            //     (fs.total_elapsed.as_secs_f64() * 1000.0 - total_logic_ms).max(0.0);
            eprintln!(
                "║ Format │ batches={:<6} entries={:<10} lines={:<10}",
                fs.total_batches, fs.total_input_entries, fs.total_output_lines
            );
            eprintln!(
                "║        │ elapsed={:.2}ms  throughput={:.0} entries/s",
                fs.total_elapsed.as_secs_f64() * 1000.0,
                f_tput
            );
            // [latency-bench] logic/copyin 分解と WasmMem% は資源使用量で延遲測試に不要。原始版を保留：
            // eprintln!(
            //     "║        │ logic={:.0}ms  copy-in={:.0}ms",
            //     total_logic_ms, total_copy_in_ms,
            // );
            // eprintln!(
            //     "║        │ WasmMem(peak)={:.0}KB ({:.1}%)",
            //     fs.wasm_mem_peak_max as f64 / 1024.0,
            //     fs.wasm_mem_peak_max as f64 / mem_limit_bytes as f64 * 100.0,
            // );
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
            } else {
                0.0
            };
            eprintln!(
                "║ Trans  │ batches={:<6} lines={:<10} bytes={:<12}",
                ts.total_batches, ts.total_input_lines, ts.total_input_bytes
            );
            eprintln!(
                "║        │ elapsed={:.2}ms  throughput={:.0} lines/s",
                ts.total_elapsed.as_secs_f64() * 1000.0,
                t_tput
            );
            // [latency-bench] plugin-reported と WasmMem% は資源使用量で延遲測試に不要。原始版を保留：
            // eprintln!(
            //     "║        │ plugin-reported={} B  WasmMem(peak)={:.0}KB ({:.1}%)",
            //     ts.total_bytes_reported,
            //     ts.wasm_mem_peak_max as f64 / 1024.0,
            //     ts.wasm_mem_peak_max as f64 / mem_limit_bytes as f64 * 100.0,
            // );
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
        wall.as_secs_f64() * 1000.0,
        e2e_tput
    );
    eprintln!("╚══════════════════════════════════════════════════════════╝");
}
