//! Core Wasm Direct Memory ABI 基準測試的 host 主程式。
//!
//! 本檔案載入 parse、filter 或 format guest，直接在 guest 線性記憶體建立 compact binary
//! request，執行 fresh instance 樣本，驗證原始位元組輸出並輸出 H0–H3 與記憶體統計。
//!
//! 所有整數皆使用 little-endian；`offset` 是相對於 request buffer 起點的 byte offset。
//!
//! # 輸入範例
//!
//! Parse 的邏輯輸入是一筆原始 JSON log：
//!
//! ```json
//! {"ts":"2026-08-14T00:00:00Z","level":"warn","msg":"hello","att":{"source":"parse-benchmark","sequence":"fixed"}}
//! ```
//!
//! 寫入 guest memory 的 Parse request layout：
//!
//! ```text
//! [record_count: u32 = 1]
//! [record_0_offset: u32][record_0_length: u32]
//! [record_0 UTF-8 JSON bytes]
//! ```
//!
//! Filter 與 Format 共用 entries request；以下為一筆 `id=0`、`level=warn` 的 entry：
//!
//! ```text
//! [entry_count: u32 = 1][total_tag_count: u32 = 1]
//! [entry descriptor: 36 bytes]
//!   id: u64 = 0
//!   timestamp_offset: u32, timestamp_length: u32
//!   level: u8 = 2, padding: 3 bytes
//!   message_offset: u32, message_length: u32
//!   first_tag_index: u32 = 0, tag_count: u32 = 1
//! [tag descriptor: 16 bytes]
//!   key_offset: u32, key_length: u32
//!   value_offset: u32, value_length: u32
//! [payload: "2026-08-14T00:00:00Z" + "hello" + "lang" + "Rust"]
//! ```
//!
//! # 輸出範例
//!
//! Parse response 將每個 parsed entry 依序編碼：
//!
//! ```text
//! [entry_count: u32 = 1]
//! [timestamp_length: u32][timestamp UTF-8 bytes]
//! [level: u8 = 2]
//! [message_length: u32]["hello"]
//! [tag_count: u32 = 3]
//! [key_length: u32]["lang"][value_length: u32]["Rust"]
//! [key_length: u32]["sequence"][value_length: u32]["fixed"]
//! [key_length: u32]["source"][value_length: u32]["parse-benchmark"]
//! [targettag_length: u32]["A"]
//! ```
//!
//! Filter response 對每筆 entry 回傳固定 9 bytes 的結果：
//!
//! ```text
//! [result_count: u32 = 1][id: u64 = 0][keep: u8 = 1]
//! ```
//!
//! Format response 直接回傳與 WCM 相同的 length-prefixed flat JSON frame：
//!
//! ```text
//! [body_length: u32 = 90]
//! {"id":0,"timestamp":"2026-08-14T00:00:00Z","level":"warn","message":"hello","lang":"Rust"}
//! ```
//!
//! # 計時邊界
//!
//! - H0–H1：計算 request 大小、guest 配置，以及 host 直接寫入 offset tables/payload。
//! - H1–H2：guest 透過 borrowed views 執行 parse、filter 或 format 核心邏輯。
//! - H2–H3：guest response 編碼、host 複製原始輸出，以及 request/response buffer 釋放。
//!
//! H0–H1 與 H2–H3 是完整 Direct Memory ABI 邊界成本，不是單純的 copy 指令耗時。

mod fixture;
mod stats;

use std::collections::VecDeque;
use std::env;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};
use core_wasm_binary_common::LogEntry;
use wasmtime::{
    Caller, Config, Engine, Linker, Memory, Module, OptLevel, ResourceLimiter, Store, TypedFunc,
};

const FRESH_POOL_SIZE: usize = 3;
const MEMORY_LIMIT_BYTES: usize = 512 * 1024 * 1024;
const DEFAULT_START: usize = 64;
const DEFAULT_END: usize = 32768;
const DEFAULT_ROUNDS: usize = 20;
const DEFAULT_WARMUP: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    Parse,
    Filter,
    Format,
}

impl Stage {
    /// 回傳測試階段使用於命令列、模組路徑及 CSV 的字串標籤。
    fn label(self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::Filter => "filter",
            Self::Format => "format",
        }
    }
}

impl FromStr for Stage {
    type Err = anyhow::Error;

    /// 將命令列字串解析成 parse、filter 或 format 階段。
    fn from_str(value: &str) -> Result<Self> {
        match value {
            "parse" => Ok(Self::Parse),
            "filter" => Ok(Self::Filter),
            "format" => Ok(Self::Format),
            other => bail!("invalid stage '{other}'; expected parse, filter, or format"),
        }
    }
}

#[derive(Default)]
struct GrowthTracker {
    limit_bytes: usize,
    grow_count: u64,
    grow_delta_bytes: u64,
    peak_bytes: usize,
    phase: u8,
}

impl GrowthTracker {
    /// 建立具有指定 Wasm 線性記憶體上限的成長追蹤器。
    fn new(limit_bytes: usize) -> Self {
        Self {
            limit_bytes,
            ..Self::default()
        }
    }

    /// 清除前一個樣本的成長統計，並設定本次初始記憶體峰值。
    fn reset_call(&mut self, initial_bytes: usize) {
        self.grow_count = 0;
        self.grow_delta_bytes = 0;
        self.peak_bytes = initial_bytes;
        self.phase = 0;
    }
}

impl ResourceLimiter for GrowthTracker {
    /// 記錄線性記憶體成長次數、增量與峰值，並拒絕超過上限的要求。
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.grow_count += 1;
        self.grow_delta_bytes += desired.saturating_sub(current) as u64;
        self.peak_bytes = self.peak_bytes.max(desired);
        Ok(desired <= self.limit_bytes)
    }

    /// 允許 Wasm 資料表成長；本 benchmark 只限制線性記憶體。
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
    marker_count: u8,
    h1: Option<Instant>,
    h2: Option<Instant>,
    growth: GrowthTracker,
}

impl HostState {
    /// 建立尚未記錄 H1/H2 且套用預設記憶體上限的 host state。
    fn new() -> Self {
        Self {
            marker_count: 0,
            h1: None,
            h2: None,
            growth: GrowthTracker::new(MEMORY_LIMIT_BYTES),
        }
    }

    /// 重設時間標記與記憶體統計，準備執行下一個樣本。
    fn reset_call(&mut self, initial_bytes: usize) {
        self.marker_count = 0;
        self.h1 = None;
        self.h2 = None;
        self.growth.reset_call(initial_bytes);
    }
}

struct CoreInstance {
    store: Store<HostState>,
    memory: Memory,
    alloc: TypedFunc<u32, u32>,
    dealloc: TypedFunc<(u32, u32), ()>,
    process: TypedFunc<(u32, u32), u64>,
}

struct FreshPool {
    engine: Engine,
    module: Module,
    linker: Linker<HostState>,
    ready: VecDeque<CoreInstance>,
}

impl FreshPool {
    /// 建立並預先補滿只使用一次的 Core Wasm instance pool。
    fn new(engine: Engine, module: Module, linker: Linker<HostState>) -> Result<Self> {
        let mut pool = Self {
            engine,
            module,
            linker,
            ready: VecDeque::with_capacity(FRESH_POOL_SIZE),
        };
        pool.replenish()?;
        Ok(pool)
    }

    /// 實例化 guest 並取得 memory、alloc、dealloc 與 process 匯出項目。
    fn create_one(&self) -> Result<CoreInstance> {
        let mut store = Store::new(&self.engine, HostState::new());
        store.limiter(|state| &mut state.growth);
        let instance = self.linker.instantiate(&mut store, &self.module)?;
        let memory = instance
            .get_memory(&mut store, "memory")
            .context("guest does not export memory")?;
        let alloc = instance.get_typed_func::<u32, u32>(&mut store, "alloc")?;
        let dealloc = instance.get_typed_func::<(u32, u32), ()>(&mut store, "dealloc")?;
        let process = instance.get_typed_func::<(u32, u32), u64>(&mut store, "process")?;
        Ok(CoreInstance {
            store,
            memory,
            alloc,
            dealloc,
            process,
        })
    }

    /// 建立新 instances，直到 pool 恢復到設定容量。
    fn replenish(&mut self) -> Result<()> {
        while self.ready.len() < FRESH_POOL_SIZE {
            self.ready.push_back(self.create_one()?);
        }
        Ok(())
    }

    /// 從 pool 取得 fresh instance；pool 為空時立即建立一個。
    fn acquire(&mut self) -> Result<CoreInstance> {
        self.ready
            .pop_front()
            .map(Ok)
            .unwrap_or_else(|| self.create_one())
    }

    /// 丟棄已呼叫的 instance 並補滿 pool，避免樣本共享 guest state。
    fn discard_and_replenish(&mut self, instance: CoreInstance) -> Result<()> {
        drop(instance);
        self.replenish()
    }
}

enum Prepared {
    Parse {
        records: Vec<String>,
        expected: Vec<u8>,
    },
    Filter {
        entries: Vec<LogEntry>,
        expected: Vec<u8>,
    },
    Format {
        entries: Vec<LogEntry>,
        expected: Vec<u8>,
    },
}

impl Prepared {
    /// 回傳此 prepared fixture 對應的測試階段。
    fn stage(&self) -> Stage {
        match self {
            Self::Parse { .. } => Stage::Parse,
            Self::Filter { .. } => Stage::Filter,
            Self::Format { .. } => Stage::Format,
        }
    }

    /// 計算目前階段 Direct Memory request 所需的精確位元組數。
    fn request_size(&self) -> usize {
        match self {
            Self::Parse { records, .. } => core_wasm_binary_common::parse_request_size(records),
            Self::Filter { entries, .. } | Self::Format { entries, .. } => {
                core_wasm_binary_common::entries_request_size(entries)
            }
        }
    }

    /// 直接將目前階段的 offset table 與 payload 寫入 guest memory slice。
    fn write_request(&self, destination: &mut [u8]) -> Result<()> {
        match self {
            Self::Parse { records, .. } => {
                core_wasm_binary_common::write_parse_request(records, destination)?
            }
            Self::Filter { entries, .. } | Self::Format { entries, .. } => {
                core_wasm_binary_common::write_entries_request(entries, destination)?
            }
        }
        Ok(())
    }

    /// 回傳 host 參考實作預先建立的預期輸出位元組。
    fn expected(&self) -> &[u8] {
        match self {
            Self::Parse { expected, .. }
            | Self::Filter { expected, .. }
            | Self::Format { expected, .. } => expected,
        }
    }

    /// 回傳此樣本預期產生的結果或 frame 筆數。
    fn output_entries(&self) -> usize {
        match self {
            Self::Parse { records, .. } => records.len(),
            Self::Filter { entries, .. } | Self::Format { entries, .. } => entries.len(),
        }
    }
}

#[derive(Clone, Copy)]
struct Sample {
    batch: usize,
    round: usize,
    logical_input_bytes: usize,
    abi_input_bytes: usize,
    abi_output_bytes: usize,
    output_entries: usize,
    input_boundary_ns: u128,
    guest_ns: u128,
    output_boundary_ns: u128,
    total_ns: u128,
    grow_count: u64,
    grow_delta_bytes: u64,
    memory_peak_bytes: usize,
    marker_count: u8,
    fixture_hash: u64,
    output_hash: u64,
}

/// 執行一次 Direct Memory ABI 樣本，測量 H0–H3、驗證輸出並彙整統計。
fn run_sample(
    instance: &mut CoreInstance,
    prepared: &Prepared,
    batch: usize,
    round: usize,
    logical_input_bytes: usize,
    fixture_hash: u64,
) -> Result<Sample> {
    // H0 前先清除上一筆樣本的時間標記與記憶體成長統計；這段不計入測量。
    let initial_bytes = instance.memory.data_size(&instance.store);
    instance.store.data_mut().reset_call(initial_bytes);

    // H0：完整 Direct Memory ABI 呼叫的計時起點。
    let h0 = Instant::now();

    // 計算精確 request 大小，並呼叫 guest alloc 配置線性記憶體；屬於 H0–H1。
    let request_size = prepared.request_size();
    let request_len = u32::try_from(request_size).context("request exceeds u32")?;
    let request_ptr = instance.alloc.call(&mut instance.store, request_len)?;
    ensure!(request_ptr != 0, "guest alloc returned null");

    // 直接取得 guest memory slice，在其中寫入 descriptors、offsets 與 payload。
    {
        let start = request_ptr as usize;
        let end = start
            .checked_add(request_size)
            .context("request range overflow")?;
        let memory = instance.memory.data_mut(&mut instance.store);
        let destination = memory
            .get_mut(start..end)
            .context("allocated request is outside guest memory")?;
        prepared.write_request(destination)?;
    }

    // 呼叫 guest process；guest 開啟 borrowed views 後標記 H1，完成核心邏輯後標記 H2。
    let packed = instance
        .process
        .call(&mut instance.store, (request_ptr, request_len))?;

    // process 將 response 指標放在 u64 高 32 位、長度放在低 32 位。
    let response_ptr = (packed >> 32) as u32;
    let response_len = packed as u32;
    ensure!(
        response_ptr != 0 && response_len != 0,
        "guest rejected binary input or returned an empty response"
    );

    // 從 guest 線性記憶體借用 response 範圍，再複製成 host-owned bytes；屬於 H2–H3。
    let response_bytes = {
        let start = response_ptr as usize;
        let end = start
            .checked_add(response_len as usize)
            .context("response range overflow")?;
        instance
            .memory
            .data(&instance.store)
            .get(start..end)
            .context("response is outside guest memory")?
            .to_vec()
    };

    // 釋放 guest 中的 request 與 response buffers；這段仍屬於 H2–H3。
    instance
        .dealloc
        .call(&mut instance.store, (request_ptr, request_len))?;
    instance
        .dealloc
        .call(&mut instance.store, (response_ptr, response_len))?;

    // H3：完成輸出複製與兩個 ABI buffers 釋放後停止計時。
    let h3 = Instant::now();

    // 參考輸出比對與 hash 位於 H3 之後，不計入 benchmark 的 total_ns。
    ensure!(
        response_bytes == prepared.expected(),
        "{} output bytes differ from reference",
        prepared.stage().label()
    );
    let output_hash = fixture::fnv1a(&response_bytes);

    // 讀取 guest 透過 time-sign 回報的 H1/H2，並確認標記數量與時間順序正確。
    let state = instance.store.data();
    ensure!(
        state.marker_count == 2,
        "expected two time-sign markers, got {}",
        state.marker_count
    );
    let h1 = state.h1.context("missing H1")?;
    let h2 = state.h2.context("missing H2")?;
    ensure!(h0 <= h1 && h1 <= h2 && h2 <= h3, "invalid marker ordering");

    // 以 H0–H3 切分三段耗時，連同大小、記憶體及一致性資訊組成 CSV 樣本。
    Ok(Sample {
        batch,
        round,
        logical_input_bytes,
        abi_input_bytes: request_size,
        abi_output_bytes: response_len as usize,
        output_entries: prepared.output_entries(),
        input_boundary_ns: h1.duration_since(h0).as_nanos(),
        guest_ns: h2.duration_since(h1).as_nanos(),
        output_boundary_ns: h3.duration_since(h2).as_nanos(),
        total_ns: h3.duration_since(h0).as_nanos(),
        grow_count: state.growth.grow_count,
        grow_delta_bytes: state.growth.grow_delta_bytes,
        memory_peak_bytes: state.growth.peak_bytes,
        marker_count: state.marker_count,
        fixture_hash,
        output_hash,
    })
}

/// 解析指定位置的命令列參數；缺少時使用預設值，格式錯誤時回傳具名錯誤。
fn parse_arg<T>(args: &[String], index: usize, default: T, name: &str) -> Result<T>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    match args.get(index) {
        Some(value) => value
            .parse::<T>()
            .map_err(|error| anyhow::anyhow!("invalid {name} '{value}': {error}")),
        None => Ok(default),
    }
}

/// 依階段準備邏輯輸入及在 H0 前計算完成的參考輸出。
fn prepare(stage: Stage, fixture: &fixture::Fixture) -> Prepared {
    match stage {
        Stage::Parse => Prepared::Parse {
            records: fixture.raw.clone(),
            expected: core_wasm_binary_common::encode_parsed_entries(&fixture.parsed),
        },
        Stage::Filter => Prepared::Filter {
            entries: fixture.entries.clone(),
            expected: core_wasm_binary_common::encode_filter_results(
                &core_wasm_binary_common::filter_entries(&fixture.entries),
            ),
        },
        Stage::Format => Prepared::Format {
            entries: fixture.entries.clone(),
            expected: core_wasm_binary_common::format_entries(&fixture.entries),
        },
    }
}

/// 根據階段組合預設 release guest Wasm 模組路徑。
fn default_module_path(stage: Stage) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    root.join(format!(
        "guest-{}/target/wasm32-unknown-unknown/release/core_wasm_binary_{}.wasm",
        stage.label(),
        stage.label()
    ))
}

/// 解析 benchmark 參數、初始化 Wasmtime、執行暖機與正式樣本，並輸出 CSV。
fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();
    let stage = args
        .get(1)
        .map(String::as_str)
        .unwrap_or("parse")
        .parse::<Stage>()?;
    let start = parse_arg(&args, 2, DEFAULT_START, "start batch")?;
    let end = parse_arg(&args, 3, DEFAULT_END, "end batch")?;
    let rounds = parse_arg(&args, 4, DEFAULT_ROUNDS, "rounds")?;
    let warmup = parse_arg(&args, 5, DEFAULT_WARMUP, "warm-up rounds")?;
    ensure!(
        start > 0 && end >= start && rounds > 0,
        "invalid batch or round range"
    );
    let module_path = args
        .get(6)
        .map(PathBuf::from)
        .unwrap_or_else(|| default_module_path(stage));

    let mut config = Config::new();
    config.cranelift_opt_level(OptLevel::Speed);
    let engine = Engine::new(&config)?;
    let module = Module::from_file(&engine, &module_path)
        .map_err(|error| anyhow::anyhow!("load core module {}: {error}", module_path.display()))?;
    let mut linker = Linker::new(&engine);
    // 提供 guest 匯入的時間標記函數，依序記錄 H1 與 H2。
    linker.func_wrap("bench", "time-sign", |mut caller: Caller<'_, HostState>| {
        let now = Instant::now();
        let state = caller.data_mut();
        match state.marker_count {
            0 => state.h1 = Some(now),
            1 => state.h2 = Some(now),
            _ => {}
        }
        state.marker_count = state.marker_count.saturating_add(1);
        state.growth.phase = state.marker_count.min(2);
    })?;
    let mut pool = FreshPool::new(engine, module, linker)?;

    eprintln!("Rust Core Wasm direct-memory ABI benchmark");
    eprintln!("stage: {}", stage.label());
    eprintln!("module: {}", module_path.display());
    eprintln!("fresh pool: {FRESH_POOL_SIZE}");
    eprintln!("rounds: {rounds} (+ {warmup} warm-up per batch)");
    println!(
        "stage,batch_count,round,logical_input_bytes,abi_input_bytes,abi_output_bytes,output_entries,input_boundary_ns,guest_ns,output_boundary_ns,total_ns,copy_in_ns,copy_out_ns,wasm_grow_count,wasm_grow_delta_bytes,wasm_mem_peak_bytes,instance_mode,module_mode,marker_count,fixture_hash,output_hash"
    );

    let mut batch = start;
    while batch <= end {
        let fixture = fixture::make_fixture(batch);
        let prepared = prepare(stage, &fixture);
        for warmup_round in 0..warmup {
            let mut instance = pool.acquire()?;
            run_sample(
                &mut instance,
                &prepared,
                batch,
                warmup_round,
                fixture.logical_input_bytes,
                fixture.hash,
            )?;
            pool.discard_and_replenish(instance)?;
        }

        let mut totals = Vec::with_capacity(rounds);
        for round in 0..rounds {
            let mut instance = pool.acquire()?;
            let sample = run_sample(
                &mut instance,
                &prepared,
                batch,
                round,
                fixture.logical_input_bytes,
                fixture.hash,
            )?;
            pool.discard_and_replenish(instance)?;
            ensure!(
                sample.input_boundary_ns + sample.guest_ns + sample.output_boundary_ns
                    == sample.total_ns,
                "timing sum mismatch"
            );
            println!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},fresh-pool,core-wasm-direct-memory,{},{:016x},{:016x}",
                stage.label(),
                sample.batch,
                sample.round,
                sample.logical_input_bytes,
                sample.abi_input_bytes,
                sample.abi_output_bytes,
                sample.output_entries,
                sample.input_boundary_ns,
                sample.guest_ns,
                sample.output_boundary_ns,
                sample.total_ns,
                sample.input_boundary_ns,
                sample.output_boundary_ns,
                sample.grow_count,
                sample.grow_delta_bytes,
                sample.memory_peak_bytes,
                sample.marker_count,
                sample.fixture_hash,
                sample.output_hash,
            );
            totals.push(sample.total_ns);
        }
        eprintln!(
            "batch={batch:>5} total median={:.3} ms p95={:.3} ms",
            stats::median(&totals) / 1e6,
            stats::percentile(&totals, 0.95) as f64 / 1e6,
        );
        batch = batch.checked_mul(2).context("batch overflow")?;
    }
    Ok(())
}
