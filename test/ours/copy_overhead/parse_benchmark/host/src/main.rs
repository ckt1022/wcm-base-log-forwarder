use std::collections::VecDeque;
use std::env;
use std::path::PathBuf;
use std::time::Instant;

use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, OptLevel, ResourceLimiter, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    path: "../wit",
    world: "parser-plugin",
});

const JSON_BYTES: usize = 1024;
const DEFAULT_START_COUNT: usize = 64;
const DEFAULT_END_COUNT: usize = 32768;
const DEFAULT_ROUNDS: usize = 20;
const DEFAULT_WARMUP_ROUNDS: usize = 3;
const MARKER_P95_NS: f64 = 47.575;
const FRESH_POOL_SIZE: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InstanceMode {
    Reuse,
    FreshPool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExperimentMode {
    Full,
    EmptyList,
    CompactRecord,
    MessageOnly,
    Message256,
    OneMessage,
}

impl ExperimentMode {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "full" => Ok(Self::Full),
            "empty-list" => Ok(Self::EmptyList),
            "compact-record" => Ok(Self::CompactRecord),
            "message-only" => Ok(Self::MessageOnly),
            "message-256" => Ok(Self::Message256),
            "one-message" => Ok(Self::OneMessage),
            other => Err(format!(
                "invalid experiment mode '{other}'; expected full, empty-list, compact-record, message-only, message-256, or one-message"
            )),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::EmptyList => "empty-list",
            Self::CompactRecord => "compact-record",
            Self::MessageOnly => "message-only",
            Self::Message256 => "message-256",
            Self::OneMessage => "one-message",
        }
    }

    fn expected_entries(self, batch_count: usize) -> usize {
        match self {
            Self::EmptyList => 0,
            Self::OneMessage => usize::from(batch_count != 0),
            _ => batch_count,
        }
    }
}

impl InstanceMode {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "reuse" => Ok(Self::Reuse),
            "fresh" | "fresh-pool" => Ok(Self::FreshPool),
            other => Err(format!(
                "invalid instance mode '{other}'; expected reuse or fresh"
            )),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Reuse => "reuse",
            Self::FreshPool => "fresh-pool",
        }
    }
}

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
        self.grow_count += 1;
        self.grow_delta_bytes += desired.saturating_sub(current) as u64;
        self.wasm_mem_peak_bytes = self.wasm_mem_peak_bytes.max(desired);
        let phase = usize::from(self.phase.min(2));
        self.stage_grow_count[phase] += 1;
        self.stage_grow_delta_bytes[phase] += desired.saturating_sub(current) as u64;
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

struct ParseInstance {
    store: Store<HostState>,
    plugin: ParserPlugin,
}

impl HostState {
    fn reset_markers(&mut self) {
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
    input_bytes: usize,
    output_entries: usize,
    output_string_bytes: usize,
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

fn make_json_1024(level: &str) -> String {
    let prefix = format!(r#"{{"ts":"2026-08-14T00:00:00Z","level":"{level}","msg":""#);
    const SUFFIX: &str = r#"","att":{"source":"parse-benchmark","sequence":"fixed"}}"#;
    let padding = JSON_BYTES
        .checked_sub(prefix.len() + SUFFIX.len())
        .expect("JSON framing exceeds target size");
    let json = format!("{prefix}{}{SUFFIX}", "x".repeat(padding));
    assert_eq!(json.len(), JSON_BYTES);
    assert!(serde_json_syntax_check(&json));
    json
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

// The host deliberately has no serde_json dependency. This small structural
// check catches mistakes in the deterministic fixture framing before Wasm runs.
fn serde_json_syntax_check(json: &str) -> bool {
    json.starts_with('{')
        && json.ends_with('}')
        && json.contains(r#""msg":""#)
        && json.contains(r#""att":{"source":"parse-benchmark""#)
}

fn output_string_bytes(entries: &[ParsedEntry]) -> usize {
    entries
        .iter()
        .map(|entry| {
            entry.timestamp.len()
                + entry.message.len()
                + entry.targettag.len()
                + entry
                    .tags
                    .iter()
                    .map(|(key, value)| key.len() + value.len())
                    .sum::<usize>()
        })
        .sum()
}

fn create_instance(
    engine: &Engine,
    component: &Component,
    linker: &Linker<HostState>,
) -> wasmtime::Result<ParseInstance> {
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
    let plugin = ParserPlugin::instantiate(&mut store, component, linker)?;
    Ok(ParseInstance { store, plugin })
}

fn call_parse(
    plugin: &ParserPlugin,
    store: &mut Store<HostState>,
    lines: &[String],
    batch_count: usize,
    round: usize,
    experiment_mode: ExperimentMode,
) -> wasmtime::Result<Sample> {
    store.data_mut().reset_markers();
    let host_before = Instant::now();
    let result = plugin.call_parse(&mut *store, lines);
    let host_after = Instant::now();

    // Surface a component trap before checking instrumentation markers. A
    // failure during canonical ABI input lowering happens before H1 and would
    // otherwise be reported only as "expected two time-sign calls".
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
    let entries = match result {
        Ok(entries) => entries,
        Err(error) => {
            return Err(wasmtime::Error::msg(format!(
                "parse returned an error: {error:?}"
            )));
        }
    };

    let sample = Sample {
        batch_count,
        round,
        input_bytes: lines.iter().map(String::len).sum(),
        output_entries: entries.len(),
        output_string_bytes: output_string_bytes(&entries),
        copy_in_ns: guest_enter.duration_since(host_before).as_nanos(),
        guest_ns: guest_exit.duration_since(guest_enter).as_nanos(),
        copy_out_ns: host_after.duration_since(guest_exit).as_nanos(),
        total_ns: host_after.duration_since(host_before).as_nanos(),
        wasm_grow_count: store.data().growth.grow_count,
        wasm_grow_delta_bytes: store.data().growth.grow_delta_bytes,
        wasm_mem_peak_bytes: store.data().growth.wasm_mem_peak_bytes,
        stage_grow_count: store.data().growth.stage_grow_count,
        stage_grow_delta_bytes: store.data().growth.stage_grow_delta_bytes,
    };
    let expected_entries = experiment_mode.expected_entries(batch_count);
    if sample.output_entries != expected_entries {
        return Err(wasmtime::Error::msg(format!(
            "expected {expected_entries} output entries, got {}",
            sample.output_entries
        )));
    }
    Ok(sample)
}

fn run_sample(
    mode: InstanceMode,
    reused: &mut Option<ParseInstance>,
    fresh_pool: &mut VecDeque<ParseInstance>,
    engine: &Engine,
    component: &Component,
    linker: &Linker<HostState>,
    lines: &[String],
    batch_count: usize,
    round: usize,
    experiment_mode: ExperimentMode,
) -> wasmtime::Result<Sample> {
    match mode {
        InstanceMode::Reuse => {
            let instance = reused
                .as_mut()
                .expect("reuse mode must own one parse instance");
            call_parse(
                &instance.plugin,
                &mut instance.store,
                lines,
                batch_count,
                round,
                experiment_mode,
            )
        }
        InstanceMode::FreshPool => {
            let mut instance = fresh_pool
                .pop_front()
                .expect("fresh pool must remain populated");
            let result = call_parse(
                &instance.plugin,
                &mut instance.store,
                lines,
                batch_count,
                round,
                experiment_mode,
            );
            drop(instance);

            // Match src/runtime.rs ParsePool::discard_and_replenish: the used
            // instance is discarded and its replacement is created outside
            // the measured H0..H3 call interval.
            fresh_pool.push_back(create_instance(engine, component, linker)?);
            result
        }
    }
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
    let instance_mode = InstanceMode::parse(args.get(6).map(String::as_str).unwrap_or("reuse"))
        .map_err(wasmtime::Error::msg)?;
    let experiment_mode = ExperimentMode::parse(args.get(7).map(String::as_str).unwrap_or("full"))
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

    let default_component_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../component/target/wasm32-wasip2/release/parse_copy_benchmark_component.wasm");
    let component_path = args
        .get(8)
        .map(PathBuf::from)
        .unwrap_or(default_component_path);
    let component_language = args.get(9).map(String::as_str).unwrap_or("rust");
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
    let mut reused = match instance_mode {
        InstanceMode::Reuse => Some(create_instance(&engine, &component, &linker)?),
        InstanceMode::FreshPool => None,
    };
    let mut fresh_pool = VecDeque::with_capacity(FRESH_POOL_SIZE);
    if instance_mode == InstanceMode::FreshPool {
        for _ in 0..FRESH_POOL_SIZE {
            fresh_pool.push_back(create_instance(&engine, &component, &linker)?);
        }
    }

    let info_fixture = make_json_1024("info");
    let warn_fixture = make_json_1024("warn");
    eprintln!("Rust parse copy benchmark");
    eprintln!("JSON size : {} bytes", info_fixture.len());
    eprintln!("batch sizes: {sizes:?}");
    eprintln!("rounds: {rounds} (+ {warmup_rounds} warm-up per size)");
    eprintln!(
        "instance mode: {}{}",
        instance_mode.label(),
        if instance_mode == InstanceMode::FreshPool {
            " (3 pre-warmed, discard and replenish after every call)"
        } else {
            ""
        }
    );
    eprintln!("marker p95 reference: {MARKER_P95_NS:.3} ns/call");
    eprintln!("experiment mode: {}", experiment_mode.label());
    eprintln!("component language: {component_language}");
    eprintln!("component path: {}", component_path.display());
    println!(
        "batch_count,round,input_bytes,output_entries,output_string_bytes,copy_in_ns,guest_ns,copy_out_ns,total_ns,wasm_grow_count,wasm_grow_delta_bytes,wasm_mem_peak_bytes,warmup_grow_count,warmup_grow_delta_bytes,instance_mode,pool_size,experiment_mode,component_language,copy_in_grow_count,guest_grow_count,copy_out_grow_count,copy_in_grow_bytes,guest_grow_bytes,copy_out_grow_bytes"
    );

    for batch_count in sizes {
        let lines: Vec<String> = (0..batch_count)
            .map(|index| {
                if index % 2 == 0 {
                    info_fixture.clone()
                } else {
                    warn_fixture.clone()
                }
            })
            .collect();
        let mut warmup_grow_count = 0_u64;
        let mut warmup_grow_delta_bytes = 0_u64;
        for warmup in 0..warmup_rounds {
            let warmup_sample = run_sample(
                instance_mode,
                &mut reused,
                &mut fresh_pool,
                &engine,
                &component,
                &linker,
                &lines,
                batch_count,
                warmup,
                experiment_mode,
            )?;
            warmup_grow_count += warmup_sample.wasm_grow_count;
            warmup_grow_delta_bytes += warmup_sample.wasm_grow_delta_bytes;
        }

        let mut samples = Vec::with_capacity(rounds);
        for round in 0..rounds {
            let sample = run_sample(
                instance_mode,
                &mut reused,
                &mut fresh_pool,
                &engine,
                &component,
                &linker,
                &lines,
                batch_count,
                round,
                experiment_mode,
            )?;
            println!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
                sample.batch_count,
                sample.round,
                sample.input_bytes,
                sample.output_entries,
                sample.output_string_bytes,
                sample.copy_in_ns,
                sample.guest_ns,
                sample.copy_out_ns,
                sample.total_ns,
                sample.wasm_grow_count,
                sample.wasm_grow_delta_bytes,
                sample.wasm_mem_peak_bytes,
                warmup_grow_count,
                warmup_grow_delta_bytes,
                instance_mode.label(),
                if instance_mode == InstanceMode::FreshPool {
                    FRESH_POOL_SIZE
                } else {
                    1
                },
                experiment_mode.label(),
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
        let marker_percent_in = MARKER_P95_NS / in_median * 100.0;
        let marker_percent_out = MARKER_P95_NS / out_median * 100.0;
        let measured_grow_count: u64 = samples.iter().map(|sample| sample.wasm_grow_count).sum();
        let measured_grow_delta_bytes: u64 = samples
            .iter()
            .map(|sample| sample.wasm_grow_delta_bytes)
            .sum();
        let wasm_mem_peak_bytes = samples
            .last()
            .map(|sample| sample.wasm_mem_peak_bytes)
            .unwrap_or_default();
        eprintln!(
            "batch={batch_count:>5} in(mean/med/p95)={in_mean:.0}/{in_median:.0}/{in_p95:.0} ns  guest={guest_mean:.0}/{guest_median:.0}/{guest_p95:.0} ns  out={out_mean:.0}/{out_median:.0}/{out_p95:.0} ns  marker/in={marker_percent_in:.3}% marker/out={marker_percent_out:.3}%  grow(warmup/measured)={warmup_grow_count}/{measured_grow_count} delta={warmup_grow_delta_bytes}/{measured_grow_delta_bytes}B peak={wasm_mem_peak_bytes}B"
        );
    }
    Ok(())
}
