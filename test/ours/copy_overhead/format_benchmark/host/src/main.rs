use std::collections::VecDeque;
use std::env;
use std::path::PathBuf;
use std::time::Instant;

use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, OptLevel, ResourceLimiter, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    path: "../wit",
    world: "format-plugin",
});

use local::log_process::pipeline_process::LogLevel;

const DEFAULT_START_COUNT: usize = 64;
const DEFAULT_END_COUNT: usize = 32768;
const DEFAULT_ROUNDS: usize = 20;
const DEFAULT_WARMUP_ROUNDS: usize = 3;
const SOURCE_JSON_BYTES_PER_ENTRY: usize = 1024;
const MESSAGE_BYTES: usize = 917;
const MARKER_P95_NS: f64 = 47.575;
const FRESH_POOL_SIZE: usize = 3;
const MEMORY_LIMIT_BYTES: usize = 512 * 1024 * 1024;
const PRODUCTION_MEASURED_ID_BASE: u64 = 1_000_000;

#[derive(Default)]
struct GrowthTracker {
    grow_count: u64,
    grow_delta_bytes: u64,
    wasm_mem_peak_bytes: usize,
    phase: u8,
    stage_grow_count: [u64; 3],
    stage_grow_delta_bytes: [u64; 3],
}

impl GrowthTracker {
    fn reset_call(&mut self) {
        self.grow_count = 0;
        self.grow_delta_bytes = 0;
        self.phase = 0;
        self.stage_grow_count = [0; 3];
        self.stage_grow_delta_bytes = [0; 3];
    }
}

impl ResourceLimiter for GrowthTracker {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let delta = desired.saturating_sub(current) as u64;
        self.grow_count += 1;
        self.grow_delta_bytes += delta;
        self.wasm_mem_peak_bytes = self.wasm_mem_peak_bytes.max(desired);
        let phase = usize::from(self.phase.min(2));
        self.stage_grow_count[phase] += 1;
        self.stage_grow_delta_bytes[phase] += delta;
        Ok(desired <= MEMORY_LIMIT_BYTES)
    }

    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(true)
    }
}

struct HostState {
    ctx: WasiCtx,
    table: ResourceTable,
    marker_count: u8,
    guest_enter: Option<Instant>,
    guest_exit: Option<Instant>,
    growth: GrowthTracker,
}

impl HostState {
    fn reset_call(&mut self) {
        self.marker_count = 0;
        self.guest_enter = None;
        self.guest_exit = None;
        self.growth.reset_call();
    }
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

struct FormatInstance {
    store: Store<HostState>,
    plugin: FormatPlugin,
}

#[derive(Clone, Copy)]
struct Sample {
    batch_count: usize,
    round: usize,
    source_json_bytes: usize,
    input_logical_bytes: usize,
    output_bytes: usize,
    output_frames: usize,
    copy_in_ns: u128,
    guest_ns: u128,
    copy_out_ns: u128,
    total_ns: u128,
    wasm_grow_count: u64,
    wasm_grow_delta_bytes: u64,
    wasm_mem_peak_bytes: usize,
    stage_grow_count: [u64; 3],
    stage_grow_delta_bytes: [u64; 3],
}

fn parse_arg<T>(args: &[String], index: usize, default: T, name: &str) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
{
    match args.get(index) {
        Some(value) => value
            .parse::<T>()
            .map_err(|error| format!("invalid {name} '{value}': {error}")),
        None => Ok(default),
    }
}

fn parse_sizes(value: &str) -> Result<Vec<usize>, String> {
    let mut sizes = vec![0];
    for item in value.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        let size = item
            .parse::<usize>()
            .map_err(|error| format!("invalid batch size '{item}': {error}"))?;
        if size == 0 {
            return Err("custom non-empty batch sizes must be greater than zero".to_string());
        }
        if !sizes.contains(&size) {
            sizes.push(size);
        }
    }
    if sizes.len() == 1 {
        return Err("custom batch-size list is empty".to_string());
    }
    Ok(sizes)
}

fn make_entries(batch_count: usize) -> Vec<LogEntry> {
    let timestamp = "2026-08-14T00:00:00Z";
    let message = "x".repeat(MESSAGE_BYTES);
    (0..batch_count)
        .map(|index| LogEntry {
            // The src pipeline starts measured calls after ten warm-ups and
            // assigns seq * 100_000 + index. Use the first measured sequence
            // so decimal IDs (and therefore formatted output bytes) match src.
            id: PRODUCTION_MEASURED_ID_BASE + index as u64,
            timestamp: timestamp.to_string(),
            level: if index % 2 == 0 {
                LogLevel::Info
            } else {
                LogLevel::Warn
            },
            message: message.clone(),
            tags: vec![
                ("lang".to_string(), "Rust".to_string()),
                ("sequence".to_string(), "fixed".to_string()),
                ("source".to_string(), "parse-benchmark".to_string()),
            ],
        })
        .collect()
}

fn entry_logical_bytes(entry: &LogEntry) -> usize {
    8 + 1
        + entry.timestamp.len()
        + entry.message.len()
        + entry
            .tags
            .iter()
            .map(|(key, value)| key.len() + value.len())
            .sum::<usize>()
}

fn json_escape(out: &mut Vec<u8>, value: &str) {
    out.push(b'"');
    for byte in value.bytes() {
        match byte {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            byte if byte < 0x20 => out.extend_from_slice(format!("\\u{byte:04x}").as_bytes()),
            byte => out.push(byte),
        }
    }
    out.push(b'"');
}

fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Debug => "debug",
        LogLevel::Info => "info",
        LogLevel::Warn => "warn",
        LogLevel::Error => "error",
        LogLevel::Crit => "crit",
        LogLevel::Alert => "alert",
        LogLevel::Emerg => "emerg",
    }
}

fn expected_output(entries: &[LogEntry]) -> Vec<u8> {
    let mut output = Vec::new();
    for entry in entries {
        let mut encoded = Vec::new();
        encoded.extend_from_slice(b"{\"id\":");
        encoded.extend_from_slice(entry.id.to_string().as_bytes());
        encoded.extend_from_slice(b",\"timestamp\":");
        json_escape(&mut encoded, &entry.timestamp);
        encoded.extend_from_slice(b",\"level\":\"");
        encoded.extend_from_slice(level_name(entry.level).as_bytes());
        encoded.extend_from_slice(b"\",\"message\":");
        json_escape(&mut encoded, &entry.message);
        for (key, value) in &entry.tags {
            encoded.push(b',');
            json_escape(&mut encoded, key);
            encoded.push(b':');
            json_escape(&mut encoded, value);
        }
        encoded.push(b'}');
        output.extend_from_slice(&(encoded.len() as u32).to_le_bytes());
        output.extend_from_slice(&encoded);
    }
    output
}

fn count_frames(bytes: &[u8]) -> wasmtime::Result<usize> {
    let mut position = 0;
    let mut frames = 0;
    while position < bytes.len() {
        if position + 4 > bytes.len() {
            return Err(wasmtime::Error::msg("truncated frame length"));
        }
        let len = u32::from_le_bytes(bytes[position..position + 4].try_into().unwrap()) as usize;
        position += 4;
        if position + len > bytes.len() {
            return Err(wasmtime::Error::msg("truncated frame payload"));
        }
        position += len;
        frames += 1;
    }
    Ok(frames)
}

fn create_instance(
    engine: &Engine,
    component: &Component,
    linker: &Linker<HostState>,
) -> wasmtime::Result<FormatInstance> {
    let state = HostState {
        ctx: WasiCtxBuilder::new().build(),
        table: ResourceTable::new(),
        marker_count: 0,
        guest_enter: None,
        guest_exit: None,
        growth: GrowthTracker::default(),
    };
    let mut store = Store::new(engine, state);
    store.limiter(|state| &mut state.growth);
    let plugin = FormatPlugin::instantiate(&mut store, component, linker)?;
    Ok(FormatInstance { store, plugin })
}

fn call_format(
    instance: &mut FormatInstance,
    entries: &[LogEntry],
    expected: &[u8],
    batch_count: usize,
    round: usize,
) -> wasmtime::Result<Sample> {
    instance.store.data_mut().reset_call();
    let h0 = Instant::now();
    let result = instance.plugin.call_format(&mut instance.store, entries);
    let h3 = Instant::now();
    let result = result??;

    let state = instance.store.data();
    if state.marker_count != 2 {
        return Err(wasmtime::Error::msg(format!(
            "expected two time-sign calls, observed {}",
            state.marker_count
        )));
    }
    let h1 = state.guest_enter.expect("first marker missing");
    let h2 = state.guest_exit.expect("second marker missing");
    if result != expected {
        return Err(wasmtime::Error::msg(
            "format output differs from host reference",
        ));
    }
    let output_frames = count_frames(&result)?;
    if output_frames != batch_count {
        return Err(wasmtime::Error::msg(format!(
            "expected {batch_count} frames, observed {output_frames}"
        )));
    }
    let input_logical_bytes = entries.iter().map(entry_logical_bytes).sum();
    Ok(Sample {
        batch_count,
        round,
        source_json_bytes: batch_count * SOURCE_JSON_BYTES_PER_ENTRY,
        input_logical_bytes,
        output_bytes: result.len(),
        output_frames,
        copy_in_ns: h1.duration_since(h0).as_nanos(),
        guest_ns: h2.duration_since(h1).as_nanos(),
        copy_out_ns: h3.duration_since(h2).as_nanos(),
        total_ns: h3.duration_since(h0).as_nanos(),
        wasm_grow_count: state.growth.grow_count,
        wasm_grow_delta_bytes: state.growth.grow_delta_bytes,
        wasm_mem_peak_bytes: state.growth.wasm_mem_peak_bytes,
        stage_grow_count: state.growth.stage_grow_count,
        stage_grow_delta_bytes: state.growth.stage_grow_delta_bytes,
    })
}

fn run_fresh_sample(
    pool: &mut VecDeque<FormatInstance>,
    engine: &Engine,
    component: &Component,
    linker: &Linker<HostState>,
    entries: &[LogEntry],
    expected: &[u8],
    batch_count: usize,
    round: usize,
) -> wasmtime::Result<Sample> {
    let mut instance = pool.pop_front().expect("fresh pool must remain populated");
    let result = call_format(&mut instance, entries, expected, batch_count, round);
    drop(instance);
    pool.push_back(create_instance(engine, component, linker)?);
    result
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    let rank = ((sorted.len() - 1) as f64 * fraction).ceil() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

fn summary(samples: &[Sample], field: impl Fn(&Sample) -> u128) -> (f64, f64, f64) {
    let mut values: Vec<f64> = samples.iter().map(|sample| field(sample) as f64).collect();
    values.sort_by(f64::total_cmp);
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    (mean, percentile(&values, 0.50), percentile(&values, 0.95))
}

fn main() -> wasmtime::Result<()> {
    let args: Vec<String> = env::args().collect();
    let start_count =
        parse_arg(&args, 1, DEFAULT_START_COUNT, "start count").map_err(wasmtime::Error::msg)?;
    let end_count =
        parse_arg(&args, 2, DEFAULT_END_COUNT, "end count").map_err(wasmtime::Error::msg)?;
    let rounds = parse_arg(&args, 3, DEFAULT_ROUNDS, "rounds").map_err(wasmtime::Error::msg)?;
    let warmup_rounds = parse_arg(&args, 4, DEFAULT_WARMUP_ROUNDS, "warm-up rounds")
        .map_err(wasmtime::Error::msg)?;
    if start_count == 0 || end_count < start_count || rounds == 0 {
        return Err(wasmtime::Error::msg(
            "require start_count > 0, end_count >= start_count, and rounds > 0",
        ));
    }
    let sizes = if let Some(custom) = args.get(5).filter(|value| !value.trim().is_empty()) {
        parse_sizes(custom).map_err(wasmtime::Error::msg)?
    } else {
        let mut sizes = vec![0];
        let mut count = start_count;
        while count <= end_count {
            sizes.push(count);
            count = count
                .checked_mul(2)
                .ok_or_else(|| wasmtime::Error::msg("batch count overflow"))?;
        }
        sizes
    };
    let component_path = args.get(6).map(PathBuf::from).unwrap_or_else(|| {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../component/target/wasm32-wasip2/release/format_copy_benchmark_component.wasm")
    });

    let mut config = Config::new();
    config.wasm_component_model(true);
    config.cranelift_opt_level(OptLevel::Speed);
    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, &component_path)?;
    let mut linker = Linker::<HostState>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    linker.root().func_wrap("time-sign", |mut store, (): ()| {
        let timestamp = Instant::now();
        let state = store.data_mut();
        match state.marker_count {
            0 => state.guest_enter = Some(timestamp),
            1 => state.guest_exit = Some(timestamp),
            _ => {}
        }
        state.marker_count = state.marker_count.saturating_add(1);
        state.growth.phase = state.marker_count.min(2);
        Ok(())
    })?;

    let mut pool = VecDeque::with_capacity(FRESH_POOL_SIZE);
    for _ in 0..FRESH_POOL_SIZE {
        pool.push_back(create_instance(&engine, &component, &linker)?);
    }

    eprintln!("Isolated format copy benchmark");
    eprintln!("batch sizes: {sizes:?}");
    eprintln!("rounds: {rounds} (+ {warmup_rounds} warm-up per size)");
    eprintln!("instance mode: fresh-pool ({FRESH_POOL_SIZE} ready instances)");
    eprintln!("marker p95 reference: {MARKER_P95_NS:.3} ns/call");
    eprintln!("component path: {}", component_path.display());
    println!(
        "batch_count,round,source_json_bytes,input_logical_bytes,output_bytes,output_frames,copy_in_ns,guest_ns,copy_out_ns,total_ns,wasm_grow_count,wasm_grow_delta_bytes,wasm_mem_peak_bytes,warmup_grow_count,warmup_grow_delta_bytes,instance_mode,component_language,marker_count,copy_in_grow_count,guest_grow_count,copy_out_grow_count,copy_in_grow_bytes,guest_grow_bytes,copy_out_grow_bytes"
    );

    for batch_count in sizes {
        let entries = make_entries(batch_count);
        let expected = expected_output(&entries);
        let mut warmup_grow_count = 0;
        let mut warmup_grow_delta_bytes = 0;
        for warmup in 0..warmup_rounds {
            let sample = run_fresh_sample(
                &mut pool,
                &engine,
                &component,
                &linker,
                &entries,
                &expected,
                batch_count,
                warmup,
            )?;
            warmup_grow_count += sample.wasm_grow_count;
            warmup_grow_delta_bytes += sample.wasm_grow_delta_bytes;
        }

        let mut samples = Vec::with_capacity(rounds);
        for round in 0..rounds {
            let sample = run_fresh_sample(
                &mut pool,
                &engine,
                &component,
                &linker,
                &entries,
                &expected,
                batch_count,
                round,
            )?;
            println!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},fresh-pool,rust,2,{},{},{},{},{},{}",
                sample.batch_count,
                sample.round,
                sample.source_json_bytes,
                sample.input_logical_bytes,
                sample.output_bytes,
                sample.output_frames,
                sample.copy_in_ns,
                sample.guest_ns,
                sample.copy_out_ns,
                sample.total_ns,
                sample.wasm_grow_count,
                sample.wasm_grow_delta_bytes,
                sample.wasm_mem_peak_bytes,
                warmup_grow_count,
                warmup_grow_delta_bytes,
                sample.stage_grow_count[0],
                sample.stage_grow_count[1],
                sample.stage_grow_count[2],
                sample.stage_grow_delta_bytes[0],
                sample.stage_grow_delta_bytes[1],
                sample.stage_grow_delta_bytes[2],
            );
            samples.push(sample);
        }
        let (in_mean, in_median, in_p95) = summary(&samples, |s| s.copy_in_ns);
        let (guest_mean, guest_median, guest_p95) = summary(&samples, |s| s.guest_ns);
        let (out_mean, out_median, out_p95) = summary(&samples, |s| s.copy_out_ns);
        let (_, total_median, total_p95) = summary(&samples, |s| s.total_ns);
        eprintln!(
            "batch={batch_count:>5} in={in_mean:.0}/{in_median:.0}/{in_p95:.0} ns guest={guest_mean:.0}/{guest_median:.0}/{guest_p95:.0} ns out={out_mean:.0}/{out_median:.0}/{out_p95:.0} ns total(med/p95)={total_median:.0}/{total_p95:.0} ns"
        );
    }
    Ok(())
}
