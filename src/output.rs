use crate::config::{Batch, BatchConfig, BatchReport, FlushReason};

pub fn print_startup(cfg: BatchConfig, safe_data_budget: usize) {
    println!("=== WCM Log Forwarder (Pooling Mode) ===");
    println!("Memory Limit: {} MB", cfg.mem_limit_mb);
    println!(
        "Safe Budget: {} KB ({:.0}% of limit)",
        safe_data_budget / 1024,
        cfg.safe_data_ratio * 100.0
    );
    println!("========================================");
}

pub fn print_batch_report(r: BatchReport) {
    let mem_ratio = (r.mem_alloc as f64 / r.mem_limit_bytes as f64) * 100.0;
    let elapsed_secs = r.elapsed.as_secs_f64();
    let throughput = if elapsed_secs > 0.0 {
        r.output_lines as f64 / elapsed_secs
    } else {
        0.0
    };

    println!(
        "Batch #{}: In={} lines/{}B, Out={} lines, PeakMem={}B ({:.2}%), Time={:.2}ms",
        r.batch_seq,
        r.input_lines,
        r.input_bytes,
        r.output_lines,
        r.mem_alloc,
        mem_ratio,
        r.elapsed.as_secs_f64() * 1000.0
    );
    println!("吞吐量: {:.2} lines/s", throughput);
}

pub fn print_flush_header(batch_seq: u64, batch: &Batch, reason: FlushReason) {
    println!("\n-------------------------------------------------------------");
    println!(
        "Flush batch #{} (Reason: size={} time={} lines={} eof={})",
        batch_seq, reason.size, reason.time, reason.line_count, reason.eof
    );
    println!(
        "Queued: {} lines, {} bytes | Age: {} ms",
        batch.len(),
        batch.bytes,
        batch.elapsed().as_millis()
    );
}