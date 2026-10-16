//! Core Wasm JSON ABI 基準測試的主程式。
//!
//! 本檔案負責載入指定階段（解析、過濾或格式化）的 Wasm 模組，建立全新的執行個體，
//! 將請求以 JSON ABI 傳入 guest，驗證輸出，並輸出跨 ABI 邊界、guest 執行及記憶體
//! 成長等測量結果。
//!
//! # 輸入範例
//!
//! 一筆原始 log（實際 benchmark 會將 `msg` 補長，使每筆 log 恰好為 1024 bytes）：
//!
//! ```json
//! {"ts":"2026-08-14T00:00:00Z","level":"warn","msg":"hello","att":{"source":"parse-benchmark","sequence":"fixed"}}
//! ```
//!
//! Parse guest 的輸入是 [`ParseRequest`]。原始 log 會作為字串放入 `records`，因此內層
//! JSON 的雙引號在外層 JSON 中會被跳脫：
//!
//! ```json
//! {"records":["{\"ts\":\"2026-08-14T00:00:00Z\",\"level\":\"warn\",\"msg\":\"hello\",\"att\":{\"source\":\"parse-benchmark\",\"sequence\":\"fixed\"}}"]}
//! ```
//!
//! Filter guest 的輸入是 [`EntriesRequest`]：
//!
//! ```json
//! {"entries":[{"id":0,"timestamp":"2026-08-14T00:00:00Z","level":"warn","message":"hello","tags":[["lang","Rust"],["sequence","fixed"],["source","parse-benchmark"]]}]}
//! ```
//!
//! # 輸出範例
//!
//! Parse guest 成功回應：
//!
//! ```json
//! {"ok":{"entries":[{"timestamp":"2026-08-14T00:00:00Z","level":"warn","message":"hello","tags":[["lang","Rust"],["sequence","fixed"],["source","parse-benchmark"]],"targettag":"A"}]}}
//! ```
//!
//! Filter guest 成功回應；`warn` 層級符合保留條件，因此 `keep` 為 `true`：
//!
//! ```json
//! {"ok":{"results":[{"id":0,"keep":true}]}}
//! ```
//!
//! Format guest 將 `[4-byte little-endian 長度][flat JSON bytes]` 以標準 Base64 包裝：
//!
//! ```json
//! {"ok":{"framed_output_base64":"iAAAAHsiaWQiOjAsInRpbWVzdGFtcCI6IjIwMjYtMDgtMTRUMDA6MDA6MDBaIiwibGV2ZWwiOiJ3YXJuIiwibWVzc2FnZSI6ImhlbGxvIiwibGFuZyI6IlJ1c3QiLCJzZXF1ZW5jZSI6ImZpeGVkIiwic291cmNlIjoicGFyc2UtYmVuY2htYXJrIn0="}}
//! ```
//!
//! 上述 Base64 解碼後，前 4 bytes 是 body 長度 `136`，後方 flat JSON body 為：
//!
//! ```json
//! {"id":0,"timestamp":"2026-08-14T00:00:00Z","level":"warn","message":"hello","lang":"Rust","sequence":"fixed","source":"parse-benchmark"}
//! ```
//!
//! Format guest 使用相同的 [`EntriesRequest`] 格式：
//!
//! ```json
//! {"entries":[{"id":0,"timestamp":"2026-08-14T00:00:00Z","level":"warn","message":"hello","tags":[["lang","Rust"],["sequence","fixed"],["source","parse-benchmark"]]}]}
//! ```
//!
//! # 計時邊界
//!
//! - H0–H1：host JSON 序列化、guest 線性記憶體配置、host 寫入，以及 guest JSON 反序列化。
//! - H1–H2：guest 執行 parse、filter 或 format 的核心邏輯。
//! - H2–H3：guest 回應編碼、host 記憶體讀取與 JSON 解碼，以及 ABI buffer 釋放。
//!
//! 因此 H0–H1 與 H2–H3 是完整的 JSON ABI 邊界成本，不是單純的記憶體複製時間。

mod fixture;
mod stats;

use std::collections::VecDeque;
use std::env;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Instant;

use anyhow::{Context, Result, bail, ensure};
use base64::Engine as _;
use core_wasm_json_common::{
    AbiResponse, EntriesRequest, FilterOk, FilterResult, FormatOk, ParseOk, ParseRequest,
    ParsedEntry,
};
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
    /// 回傳目前測試階段使用於命令列、檔名與輸出欄位的字串標籤。
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

    /// 將命令列中的階段名稱轉換為對應的測試階段。
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

    /// 清除前一次呼叫的統計，並以目前記憶體大小作為本次測量的初始峰值。
    fn reset_call(&mut self, initial_bytes: usize) {
        self.grow_count = 0;
        self.grow_delta_bytes = 0;
        self.peak_bytes = initial_bytes;
        self.phase = 0;
    }
}

impl ResourceLimiter for GrowthTracker {
    /// 記錄 Wasm 記憶體成長次數、累計增量與峰值，並判斷是否仍在允許上限內。
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

    /// 允許 Wasm 資料表成長；此基準測試只限制並追蹤線性記憶體。
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
    /// 建立一份尚未開始計時、並套用預設記憶體上限的 host 狀態。
    fn new() -> Self {
        Self {
            marker_count: 0,
            h1: None,
            h2: None,
            growth: GrowthTracker::new(MEMORY_LIMIT_BYTES),
        }
    }

    /// 重設時間標記與記憶體成長統計，準備執行下一筆樣本。
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
    /// 建立執行個體池，並預先補滿可用的全新 Wasm 執行個體。
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

    /// 實例化一個 Wasm 模組，取得其記憶體及 ABI 匯出函數。
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

    /// 持續建立執行個體，直到池內數量達到設定容量。
    fn replenish(&mut self) -> Result<()> {
        while self.ready.len() < FRESH_POOL_SIZE {
            self.ready.push_back(self.create_one()?);
        }
        Ok(())
    }

    /// 從池中取出一個全新執行個體；池為空時立即建立一個。
    fn acquire(&mut self) -> Result<CoreInstance> {
        self.ready
            .pop_front()
            .map(Ok)
            .unwrap_or_else(|| self.create_one())
    }

    /// 丟棄已執行過的個體並補滿池，避免不同樣本共享 guest 狀態。
    fn discard_and_replenish(&mut self, instance: CoreInstance) -> Result<()> {
        drop(instance);
        self.replenish()
    }
}

/// 保存各測試階段的 ABI 請求，以及 host 參考實作產生的預期輸出。
enum Prepared {
    Parse {
        request: ParseRequest,
        expected: Vec<ParsedEntry>,
    },
    Filter {
        request: EntriesRequest,
        expected: Vec<FilterResult>,
    },
    Format {
        request: EntriesRequest,
        expected: Vec<u8>,
    },
}
impl Prepared {
    /// 回傳這組已準備資料所屬的測試階段。
    fn stage(&self) -> Stage {
        match self {
            Self::Parse { .. } => Stage::Parse,
            Self::Filter { .. } => Stage::Filter,
            Self::Format { .. } => Stage::Format,
        }
    }

    /// 將目前階段的 ABI 請求序列化為要寫入 Wasm 記憶體的 JSON 位元組。
    ///
    /// 此函數在 H0 之後執行，因此序列化耗時會計入 H0–H1 的輸入邊界時間。
    fn serialize_request(&self) -> Result<Vec<u8>> {
        Ok(match self {
            // ParseRequest 格式請見檔案開頭的 Parse guest 輸入範例。
            Self::Parse { request, .. } => serde_json::to_vec(request)?,
            // Filter 與 Format 共用 EntriesRequest 格式，範例請見檔案開頭。
            Self::Filter { request, .. } | Self::Format { request, .. } => {
                serde_json::to_vec(request)?
            }
        })
    }
}

enum Decoded {
    Parse(Vec<ParsedEntry>),
    Filter(Vec<FilterResult>),
    Format(Vec<u8>),
}

impl Decoded {
    /// 計算解碼結果所含的輸出項目數；格式化結果則依 frame 格式逐筆計數。
    fn entry_count(&self) -> usize {
        match self {
            Self::Parse(entries) => entries.len(),
            Self::Filter(results) => results.len(),
            Self::Format(bytes) => count_frames(bytes).unwrap_or_default(),
        }
    }

    /// 將解碼結果轉成穩定的位元組表示，以便計算輸出雜湊。
    fn bytes_for_hash(&self) -> Result<Vec<u8>> {
        Ok(match self {
            Self::Parse(entries) => serde_json::to_vec(entries)?,
            Self::Filter(results) => serde_json::to_vec(results)?,
            Self::Format(bytes) => bytes.clone(),
        })
    }
}

#[derive(Clone, Copy)]
struct Sample {
    batch: usize,               // 此樣本包含的 log 記錄筆數。
    round: usize,               // 此批次中的暖機或正式測量輪次索引。
    logical_input_bytes: usize, // 尚未包裝成 ABI JSON 前的原始資料大小。
    abi_input_bytes: usize,     // 序列化後寫入 guest 的 JSON request 大小。
    abi_output_bytes: usize,    // 從 guest 讀出的 JSON response 大小。
    output_entries: usize,      // guest 產生的結果或 frame 筆數。

    input_boundary_ns: u128,  // H0–H1 的完整輸入邊界時間，不是純 copy-in。
    guest_ns: u128,           // H1–H2 的 guest 核心邏輯執行時間。
    output_boundary_ns: u128, // H2–H3 的完整輸出邊界時間，不是純 copy-out。
    total_ns: u128,           // H0–H3 的單次呼叫總時間。

    grow_count: u64,          // 本次呼叫期間 Wasm 線性記憶體的成長次數。
    grow_delta_bytes: u64,    // 本次呼叫期間線性記憶體的累計成長量。
    memory_peak_bytes: usize, // 本次呼叫期間線性記憶體的峰值。

    marker_count: u8,  // guest 回報的時間標記數量，正常情況應為 2。
    fixture_hash: u64, // 原始 fixture ParseRequest 的雜湊，用來確認測試資料一致。
    output_hash: u64,  // 解碼後輸出的雜湊，用來確認運算結果一致。
}

/// 依測試階段解析 guest 回傳的 JSON ABI 回應，並將錯誤回應轉為 host 錯誤。
fn decode_response(stage: Stage, bytes: &[u8]) -> Result<Decoded> {
    match stage {
        Stage::Parse => match serde_json::from_slice::<AbiResponse<ParseOk>>(bytes)? {
            AbiResponse::Ok { ok } => Ok(Decoded::Parse(ok.entries)),
            AbiResponse::Err { err } => bail!("parse guest error {}: {}", err.kind, err.message),
        },
        Stage::Filter => match serde_json::from_slice::<AbiResponse<FilterOk>>(bytes)? {
            AbiResponse::Ok { ok } => Ok(Decoded::Filter(ok.results)),
            AbiResponse::Err { err } => bail!("filter guest error {}: {}", err.kind, err.message),
        },
        Stage::Format => match serde_json::from_slice::<AbiResponse<FormatOk>>(bytes)? {
            AbiResponse::Ok { ok } => Ok(Decoded::Format(
                base64::engine::general_purpose::STANDARD
                    .decode(ok.framed_output_base64)
                    .context("decode format Base64")?,
            )),
            AbiResponse::Err { err } => bail!("format guest error {}: {}", err.kind, err.message),
        },
    }
}

/// 比對 guest 實際輸出與 host 端參考實作的預期結果。
fn validate(prepared: &Prepared, decoded: &Decoded) -> Result<()> {
    match (prepared, decoded) {
        (Prepared::Parse { expected, .. }, Decoded::Parse(actual)) => {
            ensure!(actual == expected, "parse output differs from reference")
        }
        (Prepared::Filter { expected, .. }, Decoded::Filter(actual)) => {
            ensure!(actual == expected, "filter output differs from reference")
        }
        (Prepared::Format { expected, .. }, Decoded::Format(actual)) => {
            ensure!(actual == expected, "format output differs from reference")
        }
        _ => bail!("decoded response type does not match stage"),
    }
    Ok(())
}

/// 執行單次基準樣本，完成 ABI 傳入、guest 呼叫、ABI 傳出、驗證及測量結果彙整。
fn run_sample(
    instance: &mut CoreInstance,
    prepared: &Prepared,
    batch: usize,
    round: usize,
    logical_input_bytes: usize,
    fixture_hash: u64,
) -> Result<Sample> {
    // H0 前先清除上一筆樣本的標記與記憶體成長統計；這段不計入測量。
    let initial_bytes = instance.memory.data_size(&instance.store);
    instance.store.data_mut().reset_call(initial_bytes);

    // H0：完整 ABI 呼叫的計時起點。
    let h0 = Instant::now();
    // 將此階段的請求序列化成 JSON；這段工作屬於 H0–H1。
    let request_bytes = prepared.serialize_request()?;

    // 呼叫 guest 匯出的 alloc，在其線性記憶體配置 request buffer 並取得指標。
    let request_len = u32::try_from(request_bytes.len()).context("request exceeds u32")?;
    let request_ptr = instance.alloc.call(&mut instance.store, request_len)?;
    ensure!(request_ptr != 0, "guest alloc returned null");

    // 將 JSON request 寫入 guest 的線性記憶體。
    instance
        .memory
        .write(&mut instance.store, request_ptr as usize, &request_bytes)?;

    // 呼叫 guest 匯出的 process，執行 parse、filter 或 format。
    let packed = instance
        .process
        .call(&mut instance.store, (request_ptr, request_len))?;

    // process 將 response 指標放在 u64 高 32 位、長度放在低 32 位。
    let response_ptr = (packed >> 32) as u32;
    let response_len = packed as u32;
    ensure!(
        response_ptr != 0 && response_len != 0,
        "guest returned an empty response"
    );

    // 從 guest 線性記憶體讀出 JSON response，並依目前階段解碼。
    let mut response_bytes = vec![0_u8; response_len as usize];
    instance
        .memory
        .read(&instance.store, response_ptr as usize, &mut response_bytes)?;
    let decoded_result = decode_response(prepared.stage(), &response_bytes);

    // 釋放 guest 中的 request 與 response buffer；這段工作仍屬於 H2–H3。
    instance
        .dealloc
        .call(&mut instance.store, (request_ptr, request_len))?;
    instance
        .dealloc
        .call(&mut instance.store, (response_ptr, response_len))?;

    // H3：完成 response 解碼與兩個 ABI buffer 釋放後停止計時。
    let h3 = Instant::now();

    let decoded = decoded_result?;
    // 驗證與雜湊位於 H3 之後，不計入 benchmark 的 total_ns。
    validate(prepared, &decoded)?;
    let output_hash = fixture::fnv1a(&decoded.bytes_for_hash()?);

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

    // 以四個時間點切分輸入邊界、guest 邏輯、輸出邊界及總耗時，組成 CSV 樣本。
    Ok(Sample {
        batch,
        round,
        logical_input_bytes,
        abi_input_bytes: request_bytes.len(),
        abi_output_bytes: response_bytes.len(),
        output_entries: decoded.entry_count(),
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

/// 解析以 4 位元組小端序長度為前綴的 frame 串流，並回傳完整 frame 數量。
fn count_frames(bytes: &[u8]) -> Result<usize> {
    let mut position = 0;
    let mut count = 0;
    while position < bytes.len() {
        ensure!(position + 4 <= bytes.len(), "truncated frame header");
        let length = u32::from_le_bytes(bytes[position..position + 4].try_into().unwrap()) as usize;
        position += 4;
        ensure!(position + length <= bytes.len(), "truncated frame body");
        position += length;
        count += 1;
    }
    Ok(count)
}

/// 解析指定位置的命令列參數；參數不存在時回傳預設值，格式錯誤時附上參數名稱。
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

/// 依測試階段從 fixture 建立 ABI 請求及 host 參考實作的預期輸出。
fn prepare(stage: Stage, fixture: &fixture::Fixture) -> Prepared {
    match stage {
        Stage::Parse => Prepared::Parse {
            request: ParseRequest {
                records: fixture.raw.clone(),
            },
            expected: fixture.parsed.clone(),
        },
        Stage::Filter => Prepared::Filter {
            request: EntriesRequest {
                entries: fixture.entries.clone(),
            },
            expected: core_wasm_json_common::filter_entries(&fixture.entries),
        },
        Stage::Format => Prepared::Format {
            request: EntriesRequest {
                entries: fixture.entries.clone(),
            },
            expected: core_wasm_json_common::format_entries(&fixture.entries),
        },
    }
}

/// 根據測試階段組合預設的 release Wasm 模組路徑。
fn default_module_path(stage: Stage) -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    root.join(format!(
        "guest-{}/target/wasm32-unknown-unknown/release/core_wasm_json_{}.wasm",
        stage.label(),
        stage.label()
    ))
}

/// 解析命令列設定、初始化 Wasmtime，執行各批次的暖機與正式測量，並輸出 CSV 結果。
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
    // 在 host linker 中定義 guest 匯入的 bench::time-sign 函數；guest 呼叫它兩次以記錄 H1/H2。
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

    // 建立全新 Core Wasm 執行個體的預備池。
    let mut pool = FreshPool::new(engine, module, linker)?;

    eprintln!("Rust Core Wasm JSON ABI benchmark");
    eprintln!("stage: {}", stage.label());
    eprintln!("module: {}", module_path.display());
    eprintln!("fresh pool: {FRESH_POOL_SIZE}");
    eprintln!("rounds: {rounds} (+ {warmup} warm-up per batch)");
    println!(
        "stage,batch_count,round,logical_input_bytes,abi_input_bytes,abi_output_bytes,output_entries,input_boundary_ns,guest_ns,output_boundary_ns,total_ns,copy_in_ns,copy_out_ns,wasm_grow_count,wasm_grow_delta_bytes,wasm_mem_peak_bytes,instance_mode,module_mode,marker_count,fixture_hash,output_hash"
    );

    // 從 start 到 end 逐次加倍 batch；batch 表示單次樣本包含的 log 記錄筆數。
    let mut batch = start;
    while batch <= end {
        // 建立包含 batch 筆原始 log 的固定測試資料及其衍生資料。
        let fixture = fixture::make_fixture(batch);

        // 依 stage 準備要傳入 guest 的 ABI request 與預期輸出。
        let prepared = prepare(stage, &fixture);

        // 熱機輪次
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
            // 每輪暖機後也丟棄已使用的個體，並補入全新個體。
            pool.discard_and_replenish(instance)?;
        }

        // 正式測量輪次
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
            // 丟棄已使用的個體，確保下一個樣本不會沿用 guest 狀態。
            pool.discard_and_replenish(instance)?;
            ensure!(
                sample.input_boundary_ns + sample.guest_ns + sample.output_boundary_ns
                    == sample.total_ns,
                "timing sum mismatch"
            );
            println!(
                "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},fresh-pool,core-wasm-json,{},{:016x},{:016x}",
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
