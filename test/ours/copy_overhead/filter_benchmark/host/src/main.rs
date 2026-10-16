use std::collections::VecDeque;
use std::env;
use std::path::PathBuf;
use std::time::Instant;

use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, OptLevel, ResourceLimiter, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    path: "../wit",
    world: "reduction-plugin",
});

use local::log_process::pipeline_process::LogLevel;

const DEFAULT_START_COUNT: usize = 64;
const DEFAULT_END_COUNT: usize = 32768;
const DEFAULT_ROUNDS: usize = 20;
const DEFAULT_WARMUP_ROUNDS: usize = 3;
const SOURCE_JSON_BYTES_PER_ENTRY: usize = 1024;
const MESSAGE_BYTES: usize = 917;
const FILTER_RESULT_LOGICAL_BYTES: usize = 9;
const MARKER_P95_NS: f64 = 47.575;
const FRESH_POOL_SIZE: usize = 3;
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
        Ok(true)
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

struct FilterInstance {
    store: Store<HostState>,
    plugin: ReductionPlugin,
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

#[derive(Clone, Copy)]
struct Sample {
    batch_count: usize,
    round: usize,
    source_json_bytes: usize,
    input_logical_bytes: usize,
    output_entries: usize,
    kept_entries: usize,
    output_logical_bytes: usize,
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
    let entries: Vec<_> = (0..batch_count)
        .map(|index| LogEntry {
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
        .collect();

    if let Some(entry) = entries.first() {
        assert_eq!(entry_string_bytes(entry), 979);
        assert_eq!(entry_logical_bytes(entry), 988);
    }
    entries
}

fn entry_string_bytes(entry: &LogEntry) -> usize {
    entry.timestamp.len()
        + entry.message.len()
        + entry
            .tags
            .iter()
            .map(|(key, value)| key.len() + value.len())
            .sum::<usize>()
}

fn entry_logical_bytes(entry: &LogEntry) -> usize {
    8 + 1 + entry_string_bytes(entry)
}

fn create_instance(
    engine: &Engine,
    component: &Component,
    linker: &Linker<HostState>,
) -> wasmtime::Result<FilterInstance> {
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
    let plugin = ReductionPlugin::instantiate(&mut store, component, linker)?;
    Ok(FilterInstance { store, plugin })
}

fn call_filter(
    plugin: &ReductionPlugin,
    store: &mut Store<HostState>,
    entries: &[LogEntry],
    batch_count: usize,
    round: usize,
) -> wasmtime::Result<Sample> {
    store.data_mut().reset_call();
    let host_before = Instant::now();
    let result = plugin.call_filter(&mut *store, entries);
    let host_after = Instant::now();
    let result = result?;

    let (guest_enter, guest_exit, marker_count) = {
        let state = store.data();
        (state.guest_enter, state.guest_exit, state.marker_count)
    };
    if marker_count != 2 {
        return Err(wasmtime::Error::msg(format!(
            "expected two time-sign calls, observed {marker_count}"
        )));
    }
    let guest_enter = guest_enter.expect("first marker missing");
    let guest_exit = guest_exit.expect("second marker missing");
    let results = result
        .map_err(|error| wasmtime::Error::msg(format!("filter returned an error: {error:?}")))?;

    if results.len() != batch_count {
        return Err(wasmtime::Error::msg(format!(
            "expected {batch_count} filter results, got {}",
            results.len()
        )));
    }
    for (index, result) in results.iter().enumerate() {
        let expected_keep = index % 2 == 1;
        if result.id != entries[index].id || result.keep != expected_keep {
            return Err(wasmtime::Error::msg(format!(
                "unexpected filter result at index {index}: id={}, keep={}",
                result.id, result.keep
            )));
        }
    }

    let input_logical_bytes = entries.iter().map(entry_logical_bytes).sum();
    Ok(Sample {
        batch_count,
        round,
        source_json_bytes: batch_count * SOURCE_JSON_BYTES_PER_ENTRY,
        input_logical_bytes,
        output_entries: results.len(),
        kept_entries: results.iter().filter(|result| result.keep).count(),
        output_logical_bytes: results.len() * FILTER_RESULT_LOGICAL_BYTES,
        copy_in_ns: guest_enter.duration_since(host_before).as_nanos(),
        guest_ns: guest_exit.duration_since(guest_enter).as_nanos(),
        copy_out_ns: host_after.duration_since(guest_exit).as_nanos(),
        total_ns: host_after.duration_since(host_before).as_nanos(),
        wasm_grow_count: store.data().growth.grow_count,
        wasm_grow_delta_bytes: store.data().growth.grow_delta_bytes,
        wasm_mem_peak_bytes: store.data().growth.wasm_mem_peak_bytes,
        stage_grow_count: store.data().growth.stage_grow_count,
        stage_grow_delta_bytes: store.data().growth.stage_grow_delta_bytes,
    })
}

fn run_fresh_sample(
    fresh_pool: &mut VecDeque<FilterInstance>,
    engine: &Engine,
    component: &Component,
    linker: &Linker<HostState>,
    entries: &[LogEntry],
    batch_count: usize,
    round: usize,
) -> wasmtime::Result<Sample> {
    let mut instance = fresh_pool
        .pop_front()
        .expect("fresh pool must remain populated");
    let result = call_filter(
        &instance.plugin,
        &mut instance.store,
        entries,
        batch_count,
        round,
    );
    drop(instance);

    // Each instance is called exactly once. Replacement happens outside the
    // measured H0..H3 interval, so instantiate cost is not counted as copy.
    fresh_pool.push_back(create_instance(engine, component, linker)?);
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

    let sizes = if let Some(custom_sizes) = args.get(5).filter(|value| !value.trim().is_empty()) {
        parse_sizes(custom_sizes).map_err(wasmtime::Error::msg)?
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
            .join("../component/target/wasm32-wasip2/release/filter_copy_benchmark_component.wasm")
    });
    let component_language = args.get(7).map(String::as_str).unwrap_or("rust");

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

    let mut fresh_pool = VecDeque::with_capacity(FRESH_POOL_SIZE);
    for _ in 0..FRESH_POOL_SIZE {
        fresh_pool.push_back(create_instance(&engine, &component, &linker)?);
    }

    eprintln!("Filter copy benchmark");
    eprintln!("input: production LogEntry shape; 979 dynamic string bytes/entry");
    eprintln!("levels: alternating info/warn (50% keep)");
    eprintln!("batch sizes: {sizes:?}");
    eprintln!("rounds: {rounds} (+ {warmup_rounds} warm-up per size)");
    eprintln!(
        "instance mode: fresh-pool ({FRESH_POOL_SIZE} ready instances; discard after every call)"
    );
    eprintln!("marker p95 reference: {MARKER_P95_NS:.3} ns/call");
    eprintln!("component language: {component_language}");
    eprintln!("component path: {}", component_path.display());
    println!(
        "batch_count,round,source_json_bytes,input_logical_bytes,output_entries,kept_entries,output_logical_bytes,copy_in_ns,guest_ns,copy_out_ns,total_ns,wasm_grow_count,wasm_grow_delta_bytes,wasm_mem_peak_bytes,warmup_grow_count,warmup_grow_delta_bytes,instance_mode,component_language,copy_in_grow_count,guest_grow_count,copy_out_grow_count,copy_in_grow_bytes,guest_grow_bytes,copy_out_grow_bytes"
    );

    for batch_count in sizes {
        let entries = make_entries(batch_count);
        let mut warmup_grow_count = 0;
        let mut warmup_grow_delta_bytes = 0;
        for warmup in 0..warmup_rounds {
            let sample = run_fresh_sample(
                &mut fresh_pool,
                &engine,
                &component,
                &linker,
                &entries,
                batch_count,
                warmup,
            )?;
            warmup_grow_count += sample.wasm_grow_count;
            warmup_grow_delta_bytes += sample.wasm_grow_delta_bytes;
        }

        let mut samples = Vec::with_capacity(rounds);
        for round in 0..rounds {
            let sample = run_fresh_sample(
                &mut fresh_pool,
                &engine,
                &component,
                &linker,
                &entries,
                batch_count,
                round,
            )?;
            println!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},fresh-pool,{},{},{},{},{},{},{}",
                sample.batch_count,
                sample.round,
                sample.source_json_bytes,
                sample.input_logical_bytes,
                sample.output_entries,
                sample.kept_entries,
                sample.output_logical_bytes,
                sample.copy_in_ns,
                sample.guest_ns,
                sample.copy_out_ns,
                sample.total_ns,
                sample.wasm_grow_count,
                sample.wasm_grow_delta_bytes,
                sample.wasm_mem_peak_bytes,
                warmup_grow_count,
                warmup_grow_delta_bytes,
                component_language,
                sample.stage_grow_count[0],
                sample.stage_grow_count[1],
                sample.stage_grow_count[2],
                sample.stage_grow_delta_bytes[0],
                sample.stage_grow_delta_bytes[1],
                sample.stage_grow_delta_bytes[2],
            );
            samples.push(sample);
        }

        let (in_mean, in_median, in_p95) = summary(&samples, |sample| sample.copy_in_ns);
        let (guest_mean, guest_median, guest_p95) = summary(&samples, |sample| sample.guest_ns);
        let (out_mean, out_median, out_p95) = summary(&samples, |sample| sample.copy_out_ns);
        let (_, total_median, total_p95) = summary(&samples, |sample| sample.total_ns);
        eprintln!(
            "batch={batch_count:>5} in(mean/med/p95)={in_mean:.0}/{in_median:.0}/{in_p95:.0} ns  guest={guest_mean:.0}/{guest_median:.0}/{guest_p95:.0} ns  out={out_mean:.0}/{out_median:.0}/{out_p95:.0} ns  total(med/p95)={total_median:.0}/{total_p95:.0} ns"
        );
    }

    Ok(())
}
