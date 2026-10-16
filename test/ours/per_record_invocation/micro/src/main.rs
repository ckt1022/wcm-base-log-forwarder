//! Per-record vs batch plugin invocation micro benchmark.
//!
//! 以正式 forwarder 的 WIT（wit/ 由 repo 根目錄複製）與正式 plugin .wasm，
//! 處理固定 N 筆（預設 1024 筆、每筆約 1 KB）記錄，每次呼叫帶 B 筆：
//!
//!   reuse      : 一個 Store + instance，連續呼叫 ceil(N/B) 次（使用者提議的模式）
//!   fresh-pre  : 每次呼叫都新建 Store，並以預先 link 好的 InstancePre 實例化
//!   fresh-link : 每次呼叫都新建 Store，並以 Linker 完整實例化（同 forwarder 的 instantiate 路徑）
//!
//! Engine 設定與 forwarder `build_runtime` 相同（Cranelift / OptLevel::Speed / epoch_interruption），
//! Store 帶與 forwarder 相同的 ResourceLimiter（128 MiB）與 WasiCtx（inherit_stdio）。
//! 每次呼叫前與 forwarder 一樣 set_epoch_deadline。
//!
//! 若 component 有 import `time-sign`（copy_overhead 的 instrumented 版本），
//! 每次呼叫額外記錄 H0..H3 並累加 copy-in / guest / copy-out（與 copy_overhead 方法相同）。
//! 未 instrumented 的正式 plugin 只量整段（N 筆）的總時間，不做逐呼叫計時。
//!
//! 輸入 fixture 與 test/ours/copy_overhead 的 parse/filter/format benchmark 相同：
//!   parse : 1024-byte JSON（ts/level/msg/att），info/warn 交替
//!   filter/format : LogEntry（message 917 B、3 個 tag、id 從 1_000_000 起）
//!
//! 用法:
//!   percall_micro <stage> <wasm> <lang> <mode> <sizes> <records> <rounds> <warmup> <run>
//! 例:
//!   percall_micro parse parse.wasm rust reuse 1,4,16,64,256,1024 1024 100 10 1 > out.csv

use std::env;
use std::ops::Range;
use std::time::Instant;

use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime::{Config, Engine, OptLevel, ResourceLimiter, Store, Strategy};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};

wasmtime::component::bindgen!({
    world: "parser-plugin",
    path: "wit",
});

pub mod format_bindings {
    wasmtime::component::bindgen!({
        world: "format-plugin",
        path: "wit",
        with: {
            "local:log-process/pipeline-process":
                super::local::log_process::pipeline_process,
        }
    });
}

pub mod reduction_bindings {
    wasmtime::component::bindgen!({
        world: "reduction-plugin",
        path: "wit",
        with: {
            "local:log-process/pipeline-process":
                super::local::log_process::pipeline_process,
        }
    });
}

use format_bindings::{FormatPlugin, FormatPluginPre};
use local::log_process::pipeline_process::{LogEntry, LogLevel};
use reduction_bindings::{ReductionPlugin, ReductionPluginPre};

const JSON_BYTES: usize = 1024;
const MESSAGE_BYTES: usize = 917;
const ID_BASE: u64 = 1_000_000;
const MEM_LIMIT_BYTES: usize = 128 * 1024 * 1024; // forwarder_membound_b1.yaml mem_limit_mb
const EPOCH_DEADLINE: u64 = 3_000; // 與 forwarder 相同：stage_timeout 30 s / 10 ms tick（epoch 從不遞增）

// ── Host state（對應 forwarder 的 MyState / MyLimiter）────────────────────────

struct Limiter {
    peak: usize,
    grow_count: u64,
}

impl ResourceLimiter for Limiter {
    fn memory_growing(&mut self, _current: usize, desired: usize, _max: Option<usize>) -> wasmtime::Result<bool> {
        self.peak = self.peak.max(desired);
        self.grow_count += 1;
        Ok(desired <= MEM_LIMIT_BYTES)
    }
    fn table_growing(&mut self, _c: usize, _d: usize, _m: Option<usize>) -> wasmtime::Result<bool> {
        Ok(true)
    }
}

struct HostState {
    ctx: WasiCtx,
    table: ResourceTable,
    limiter: Limiter,
    marker_count: u8,
    h1: Option<Instant>,
    h2: Option<Instant>,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView { ctx: &mut self.ctx, table: &mut self.table }
    }
}

fn new_store(engine: &Engine) -> Store<HostState> {
    let state = HostState {
        ctx: WasiCtxBuilder::new().inherit_stdio().build(),
        table: ResourceTable::new(),
        limiter: Limiter { peak: 0, grow_count: 0 },
        marker_count: 0,
        h1: None,
        h2: None,
    };
    let mut store = Store::new(engine, state);
    store.limiter(|s| &mut s.limiter);
    store.set_epoch_deadline(EPOCH_DEADLINE);
    store
}

// ── Stage / plugin 抽象 ──────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Stage {
    Parse,
    Filter,
    Format,
}

enum Plugin {
    Parse(ParserPlugin),
    Filter(ReductionPlugin),
    Format(FormatPlugin),
}

enum Pre {
    Parse(ParserPluginPre<HostState>),
    Filter(ReductionPluginPre<HostState>),
    Format(FormatPluginPre<HostState>),
}

struct Inputs {
    lines: Vec<String>,
    entries: Vec<LogEntry>,
}

#[derive(Default, Clone, Copy)]
struct Breakdown {
    copy_in: u128,
    guest: u128,
    copy_out: u128,
}

impl Pre {
    fn new(stage: Stage, linker: &Linker<HostState>, component: &Component) -> wasmtime::Result<Self> {
        let ip = linker.instantiate_pre(component)?;
        Ok(match stage {
            Stage::Parse => Pre::Parse(ParserPluginPre::new(ip)?),
            Stage::Filter => Pre::Filter(ReductionPluginPre::new(ip)?),
            Stage::Format => Pre::Format(FormatPluginPre::new(ip)?),
        })
    }
    fn instantiate(&self, store: &mut Store<HostState>) -> wasmtime::Result<Plugin> {
        Ok(match self {
            Pre::Parse(p) => Plugin::Parse(p.instantiate(store)?),
            Pre::Filter(p) => Plugin::Filter(p.instantiate(store)?),
            Pre::Format(p) => Plugin::Format(p.instantiate(store)?),
        })
    }
}

fn instantiate_link(
    stage: Stage,
    store: &mut Store<HostState>,
    component: &Component,
    linker: &Linker<HostState>,
) -> wasmtime::Result<Plugin> {
    Ok(match stage {
        Stage::Parse => Plugin::Parse(ParserPlugin::instantiate(store, component, linker)?),
        Stage::Filter => Plugin::Filter(ReductionPlugin::instantiate(store, component, linker)?),
        Stage::Format => Plugin::Format(FormatPlugin::instantiate(store, component, linker)?),
    })
}

impl Plugin {
    /// 呼叫一次 plugin（帶 inputs[r]）。validate=true 時檢查輸出筆數。
    /// instrumented=true 時回傳 H0..H3 拆解。
    fn call(
        &self,
        store: &mut Store<HostState>,
        inputs: &Inputs,
        r: Range<usize>,
        validate: bool,
        instrumented: bool,
    ) -> wasmtime::Result<Option<Breakdown>> {
        store.set_epoch_deadline(EPOCH_DEADLINE);
        let h0 = if instrumented {
            let s = store.data_mut();
            s.marker_count = 0;
            s.h1 = None;
            s.h2 = None;
            Some(Instant::now())
        } else {
            None
        };
        let n = r.len();
        let (h3, produced) = match self {
            Plugin::Parse(p) => {
                let out = p.call_parse(&mut *store, &inputs.lines[r])?;
                let h3 = h0.map(|_| Instant::now());
                let v = out.map_err(|e| wasmtime::Error::msg(format!("parse error: {e:?}")))?;
                (h3, v.len())
            }
            Plugin::Filter(p) => {
                let out = p.call_filter(&mut *store, &inputs.entries[r])?;
                let h3 = h0.map(|_| Instant::now());
                let v = out.map_err(|e| wasmtime::Error::msg(format!("filter error: {e:?}")))?;
                (h3, v.len())
            }
            Plugin::Format(p) => {
                let out = p.call_format(&mut *store, &inputs.entries[r])?;
                let h3 = h0.map(|_| Instant::now());
                let bytes = out.map_err(|e| wasmtime::Error::msg(format!("format error: {e:?}")))?;
                let frames = if validate { count_frames(&bytes) } else { n };
                (h3, frames)
            }
        };
        if validate && produced != n {
            return Err(wasmtime::Error::msg(format!("expected {n} outputs, got {produced}")));
        }
        match (h0, h3) {
            (Some(h0), Some(h3)) => {
                let s = store.data();
                if s.marker_count != 2 {
                    return Err(wasmtime::Error::msg(format!(
                        "instrumented component: expected 2 time-sign calls, saw {}",
                        s.marker_count
                    )));
                }
                let (h1, h2) = (s.h1.unwrap(), s.h2.unwrap());
                Ok(Some(Breakdown {
                    copy_in: h1.duration_since(h0).as_nanos(),
                    guest: h2.duration_since(h1).as_nanos(),
                    copy_out: h3.duration_since(h2).as_nanos(),
                }))
            }
            _ => Ok(None),
        }
    }
}

/// format 輸出為 [4-byte LE 長度][資料] frame 串接（與 forwarder format_loop 相同）。
fn count_frames(bytes: &[u8]) -> usize {
    let mut pos = 0;
    let mut n = 0;
    while pos + 4 <= bytes.len() {
        let len = u32::from_le_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        pos += 4 + len;
        n += 1;
    }
    n
}

// ── Fixtures（與 copy_overhead benchmark 相同）───────────────────────────────

fn make_json_1024(level: &str) -> String {
    let prefix = format!(r#"{{"ts":"2026-08-14T00:00:00Z","level":"{level}","msg":""#);
    const SUFFIX: &str = r#"","att":{"source":"parse-benchmark","sequence":"fixed"}}"#;
    let padding = JSON_BYTES - (prefix.len() + SUFFIX.len());
    let json = format!("{prefix}{}{SUFFIX}", "x".repeat(padding));
    assert_eq!(json.len(), JSON_BYTES);
    json
}

fn make_inputs(records: usize) -> Inputs {
    let info = make_json_1024("info");
    let warn = make_json_1024("warn");
    let lines = (0..records).map(|i| if i % 2 == 0 { info.clone() } else { warn.clone() }).collect();
    let message = "x".repeat(MESSAGE_BYTES);
    let entries = (0..records)
        .map(|i| LogEntry {
            id: ID_BASE + i as u64,
            timestamp: "2026-08-14T00:00:00Z".to_string(),
            level: if i % 2 == 0 { LogLevel::Info } else { LogLevel::Warn },
            message: message.clone(),
            tags: vec![
                ("lang".to_string(), "Rust".to_string()),
                ("sequence".to_string(), "fixed".to_string()),
                ("source".to_string(), "parse-benchmark".to_string()),
            ],
        })
        .collect();
    Inputs { lines, entries }
}

fn ranges(records: usize, b: usize) -> Vec<Range<usize>> {
    (0..records).step_by(b).map(|s| s..(s + b).min(records)).collect()
}

fn median(v: &mut [f64]) -> f64 {
    v.sort_by(f64::total_cmp);
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

fn main() -> wasmtime::Result<()> {
    let a: Vec<String> = env::args().collect();
    if a.len() < 10 {
        eprintln!("usage: {} <parse|filter|format> <wasm> <lang> <reuse|fresh-pre|fresh-link> <sizes> <records> <rounds> <warmup> <run>", a[0]);
        std::process::exit(2);
    }
    let stage = match a[1].as_str() {
        "parse" => Stage::Parse,
        "filter" => Stage::Filter,
        "format" => Stage::Format,
        s => return Err(wasmtime::Error::msg(format!("bad stage {s}"))),
    };
    let wasm = &a[2];
    let lang = &a[3];
    let mode = a[4].as_str();
    if !matches!(mode, "reuse" | "fresh-pre" | "fresh-link") {
        return Err(wasmtime::Error::msg(format!("bad mode {mode}")));
    }
    let sizes: Vec<usize> = a[5].split(',').map(|s| s.trim().parse().unwrap()).collect();
    let records: usize = a[6].parse()?;
    let rounds: usize = a[7].parse()?;
    let warmup: usize = a[8].parse()?;
    let run = &a[9];

    let mut config = Config::new();
    config.wasm_component_model(true);
    config.strategy(Strategy::Cranelift);
    config.cranelift_opt_level(OptLevel::Speed);
    config.epoch_interruption(true);
    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, wasm)?;
    let mut linker: Linker<HostState> = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    // copy_overhead instrumented component 的 time-sign marker；正式 plugin 不 import，定義也無害。
    linker.root().func_wrap("time-sign", |mut store, (): ()| {
        let t = Instant::now();
        let s = store.data_mut();
        match s.marker_count {
            0 => s.h1 = Some(t),
            1 => s.h2 = Some(t),
            _ => {}
        }
        s.marker_count = s.marker_count.saturating_add(1);
        Ok(())
    })?;
    let instrumented = component
        .component_type()
        .imports(&engine)
        .any(|(name, _)| name == "time-sign");
    let pre = Pre::new(stage, &linker, &component)?;
    let inputs = make_inputs(records);

    eprintln!(
        "[micro] stage={} lang={lang} mode={mode} records={records} rounds={rounds} warmup={warmup} run={run} instrumented={instrumented} wasm={wasm}",
        a[1]
    );
    println!("stage,lang,mode,call_size,run,round,records,calls,total_ns,ns_per_record,records_per_sec,copy_in_ns,guest_ns,copy_out_ns,instrumented,wasm_mem_peak_bytes,grow_count");

    for &b0 in &sizes {
        let b = b0.min(records).max(1);
        let rs = ranges(records, b);
        // reuse：每個 B 一個 Store + instance，warm-up 與量測都用它
        let mut reused: Option<(Store<HostState>, Plugin)> = None;
        if mode == "reuse" {
            let mut store = new_store(&engine);
            let plugin = pre.instantiate(&mut store)?;
            reused = Some((store, plugin));
        }
        let mut totals = Vec::with_capacity(rounds);
        for round in 0..(warmup + rounds) {
            let measured = round >= warmup;
            let validate = !measured; // warm-up 回合驗證輸出筆數；量測回合不做額外工作
            let mut bd = Breakdown::default();
            let t0 = Instant::now();
            for r in rs.iter().cloned() {
                let res = match mode {
                    "reuse" => {
                        let (store, plugin) = reused.as_mut().unwrap();
                        plugin.call(store, &inputs, r, validate, instrumented)?
                    }
                    "fresh-pre" => {
                        let mut store = new_store(&engine);
                        let plugin = pre.instantiate(&mut store)?;
                        plugin.call(&mut store, &inputs, r, validate, instrumented)?
                    }
                    _ => {
                        let mut store = new_store(&engine);
                        let plugin = instantiate_link(stage, &mut store, &component, &linker)?;
                        plugin.call(&mut store, &inputs, r, validate, instrumented)?
                    }
                };
                if let Some(x) = res {
                    bd.copy_in += x.copy_in;
                    bd.guest += x.guest;
                    bd.copy_out += x.copy_out;
                }
            }
            let total = t0.elapsed().as_nanos();
            if !measured {
                continue;
            }
            let (peak, grows) = reused
                .as_ref()
                .map(|(s, _)| (s.data().limiter.peak, s.data().limiter.grow_count))
                .unwrap_or((0, 0));
            let nspr = total as f64 / records as f64;
            let rps = records as f64 * 1e9 / total as f64;
            let (ci, g, co) = if instrumented {
                (bd.copy_in.to_string(), bd.guest.to_string(), bd.copy_out.to_string())
            } else {
                (String::new(), String::new(), String::new())
            };
            println!(
                "{},{lang},{mode},{b},{run},{},{records},{},{total},{nspr:.1},{rps:.1},{ci},{g},{co},{instrumented},{peak},{grows}",
                a[1],
                round - warmup,
                rs.len(),
            );
            totals.push(total as f64);
        }
        let med = median(&mut totals);
        eprintln!(
            "[micro]   B={b:>5} calls={:>5} median_total={:.3} ms  ns/record={:.0}  rec/s={:.0}",
            rs.len(),
            med / 1e6,
            med / records as f64,
            records as f64 * 1e9 / med
        );
    }
    Ok(())
}
