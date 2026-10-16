// 本資料夾負責時間函數呼叫的成本實測
// 實驗原因 : 在量測copy-in與copy-out的時間成本過程中，向外部取得時間的函數也會佔有成本，若是沒有剃除
// 可能會對量測複製的成本產生誤差，若是時間函數呼叫的成本遠低於copy-in與copy-out，則該成本可以忽略不計，
// 大幅減少實驗的複雜度。
// 
// 實驗方式 : 有兩種函數 
// 1. baseline() 內部沒有呼叫任何向外的函數，只有重複的迴圈呼叫與算術 T_baseline = component呼叫成本(固定) + 迴圈呼叫成本(N x L每次計算成本)
// 2. with-noop-sign() 內部有向外呼叫，但沒有執行時間呼叫， T_no-op = component呼叫成本(固定) + 迴圈呼叫成本(N x L向外呼叫與計算)
// 3. with-time-sign() 內部有向外呼叫，且執行時間呼叫並回傳， T_time =  component呼叫成本(固定) + 迴圈呼叫成本(N x L向外呼叫+時間+計算)

use std::env;
use std::path::PathBuf;
use std::time::Instant;

use wasmtime::component::{Component, Linker, TypedFunc};
use wasmtime::{Config, Engine, OptLevel, Store};

#[derive(Default)]
struct HostState {
    noop_calls: u64,
    timed_calls: u64,
    last_timestamp: Option<Instant>,
}

#[derive(Clone, Copy)]
struct Sample {
    round: usize,
    baseline_ns: u128,
    noop_ns: u128,
    timed_ns: u128,
    noop_per_call: f64,
    timed_per_call: f64,
    clock_read_per_call: f64,
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

// 量測函數呼叫時間
// 可傳入函數，因此empty與time使用相同函數測量。
fn timed_call(
    function: &TypedFunc<(u32, u64), (u64,)>,
    store: &mut Store<HostState>,
    iterations: u32,
    seed: u64,
) -> wasmtime::Result<(u64, u128)> {
    // 時間起點
    let started = Instant::now();

    // function call
    let (checksum,) = function.call(store, (iterations, seed))?;

    // 結束並計算函數的呼叫
    Ok((checksum, started.elapsed().as_nanos()))
}

fn percentile(sorted: &[f64], fraction: f64) -> f64 {
    let rank = ((sorted.len() - 1) as f64 * fraction).ceil() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

fn print_stats(label: &str, values: impl Iterator<Item = f64>) {
    let mut values: Vec<f64> = values.collect();
    values.sort_by(f64::total_cmp);
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / values.len() as f64;
    eprintln!(
        "{label:<20} mean={mean:>8.3}  median={:>8.3}  p95={:>8.3}  min/max={:>8.3}/{:>8.3}  stddev={:>7.3} ns",
        percentile(&values, 0.50),
        percentile(&values, 0.95),
        values[0],
        values[values.len() - 1],
        variance.sqrt(),
    );
}

fn main() -> wasmtime::Result<()> {
    let args: Vec<String> = env::args().collect();
    let iterations =
        parse_arg(&args, 1, 100_000_u32, "iterations").map_err(wasmtime::Error::msg)?;
    let rounds = parse_arg(&args, 2, 30_usize, "rounds").map_err(wasmtime::Error::msg)?;
    let warmup_rounds =
        parse_arg(&args, 3, 5_usize, "warmup rounds").map_err(wasmtime::Error::msg)?;
    if iterations == 0 || rounds == 0 {
        return Err(wasmtime::Error::msg(
            "iterations and rounds must both be greater than zero",
        ));
    }

    let component_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../component/target/wasm32-wasip2/release/marker_call_component.wasm");
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.cranelift_opt_level(OptLevel::Speed);
    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, &component_path)?;

    let mut linker = Linker::<HostState>::new(&engine);
    linker.root().func_wrap("noop-sign", |mut store, (): ()| {
        store.data_mut().noop_calls += 1;
        Ok(())
    })?;
    linker.root().func_wrap("time-sign", |mut store, (): ()| {
        let timestamp = Instant::now();
        let state = store.data_mut();
        state.timed_calls += 1;
        state.last_timestamp = Some(timestamp);
        Ok(())
    })?;

    let mut store = Store::new(&engine, HostState::default());
    let instance = linker.instantiate(&mut store, &component)?;
    let baseline = instance.get_typed_func::<(u32, u64), (u64,)>(&mut store, "baseline")?;
    let with_noop = instance.get_typed_func::<(u32, u64), (u64,)>(&mut store, "with-noop-sign")?;
    let with_time = instance.get_typed_func::<(u32, u64), (u64,)>(&mut store, "with-time-sign")?;

    for warmup in 0..warmup_rounds {
        let seed = 0x1234_5678_9abc_def0_u64.wrapping_add(warmup as u64);
        let (control, _) = timed_call(&baseline, &mut store, iterations, seed)?;
        let (noop, _) = timed_call(&with_noop, &mut store, iterations, seed)?;
        let (timed, _) = timed_call(&with_time, &mut store, iterations, seed)?;
        if control != noop || control != timed {
            return Err(wasmtime::Error::msg("warm-up checksum mismatch"));
        }
    }
    store.data_mut().noop_calls = 0;
    store.data_mut().timed_calls = 0;

    let mut samples = Vec::with_capacity(rounds);
    for round in 0..rounds {
        let seed = 0xd1b5_4a32_d192_ed03_u64.wrapping_add(round as u64);
        // Rotate the three call orders to reduce drift-related bias.
        let (control, noop, timed) = match round % 3 {
            0 => (
                timed_call(&baseline, &mut store, iterations, seed)?,
                timed_call(&with_noop, &mut store, iterations, seed)?,
                timed_call(&with_time, &mut store, iterations, seed)?,
            ),
            1 => {
                let noop = timed_call(&with_noop, &mut store, iterations, seed)?;
                let timed = timed_call(&with_time, &mut store, iterations, seed)?;
                let control = timed_call(&baseline, &mut store, iterations, seed)?;
                (control, noop, timed)
            }
            _ => {
                let timed = timed_call(&with_time, &mut store, iterations, seed)?;
                let control = timed_call(&baseline, &mut store, iterations, seed)?;
                let noop = timed_call(&with_noop, &mut store, iterations, seed)?;
                (control, noop, timed)
            }
        };
        if control.0 != noop.0 || control.0 != timed.0 {
            return Err(wasmtime::Error::msg(format!(
                "checksum mismatch in round {round}"
            )));
        }

        let divisor = iterations as f64;
        let noop_per_call = (noop.1 as i128 - control.1 as i128) as f64 / divisor;
        let timed_per_call = (timed.1 as i128 - control.1 as i128) as f64 / divisor;
        let clock_read_per_call = (timed.1 as i128 - noop.1 as i128) as f64 / divisor;
        samples.push(Sample {
            round,
            baseline_ns: control.1,
            noop_ns: noop.1,
            timed_ns: timed.1,
            noop_per_call,
            timed_per_call,
            clock_read_per_call,
        });
    }

    let expected_calls = iterations as u64 * rounds as u64;
    if store.data().noop_calls != expected_calls || store.data().timed_calls != expected_calls {
        return Err(wasmtime::Error::msg(format!(
            "callback count mismatch: expected {expected_calls}, got noop={} timed={}",
            store.data().noop_calls,
            store.data().timed_calls
        )));
    }
    if store.data().last_timestamp.is_none() {
        return Err(wasmtime::Error::msg("time-sign did not save a timestamp"));
    }

    println!(
        "round,iterations,baseline_ns,noop_sign_ns,time_sign_ns,noop_ns_per_call,time_sign_ns_per_call,clock_read_increment_ns"
    );
    for sample in &samples {
        println!(
            "{},{},{},{},{},{:.6},{:.6},{:.6}",
            sample.round,
            iterations,
            sample.baseline_ns,
            sample.noop_ns,
            sample.timed_ns,
            sample.noop_per_call,
            sample.timed_per_call,
            sample.clock_read_per_call,
        );
    }

    eprintln!("Rust component -> Rust host synchronous callback benchmark");
    eprintln!("component : {}", component_path.display());
    eprintln!("iterations: {iterations} per round");
    eprintln!("rounds    : {rounds} (+ {warmup_rounds} warm-up)");
    eprintln!("samples   : {expected_calls} calls for each callback kind");
    print_stats("empty callback", samples.iter().map(|s| s.noop_per_call));
    print_stats(
        "timestamp callback",
        samples.iter().map(|s| s.timed_per_call),
    );
    print_stats(
        "Instant::now delta",
        samples.iter().map(|s| s.clock_read_per_call),
    );
    Ok(())
}
