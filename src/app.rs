use std::io::{self, BufRead};
use std::sync::{Arc, RwLock, mpsc::SyncSender};
use std::thread;
use std::time::Duration;

use wasmtime::{
    Config, Engine, OptLevel, ResourceLimiter, Strategy,
    component::{Component, Linker},
};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};
use wasmtime_wasi_http::WasiHttpCtx;
use wasmtime_wasi_http::bindings::http::types::ErrorCode;
use wasmtime_wasi_http::body::HyperOutgoingBody;
use wasmtime_wasi_http::types::{HostFutureIncomingResponse, OutgoingRequestConfig, default_send_request};

use crate::capability::{self, Stage, StageGrant};

// parser-plugin world：產生 ParserPlugin 及 pipeline-process 所有型別
wasmtime::component::bindgen!({
    world: "parser-plugin",
    path: "wit",
});

// format-plugin world：用 `with` 重用上面已生成的 LogEntry / LogLevel，
// 使兩個 bindgen 共用同一 Rust 型別，channel 傳遞時無需轉換。
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

// reduction-plugin world：重用 pipeline-process 的 LogEntry / FilterResult。
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

// transport-plugin world：async 模式（需 WASI HTTP），重用 pipeline-process 的 PluginError。
// 用 typed bindgen 取代 Val API，讓 list<list<u8>> 直接映射成 &[Vec<u8>]，
// 省去原本每個 byte 都建立一個 Val::U8 enum 的 37x 額外成本。
pub mod transport_bindings {
    wasmtime::component::bindgen!({
        world: "transport-plugin",
        path: "wit",
        exports: { default: async },
        with: {
            "local:log-process/pipeline-process":
                super::local::log_process::pipeline_process,
        }
    });
}

/// WASM 線性記憶體追蹤器，實作 ResourceLimiter 介面。
pub struct MyLimiter {
    pub max_allocation: usize,
    mem_limit_bytes: usize,
    pub wasm_mem_peak: usize,
    /// 本次 store 生命週期內 memory.grow 被觸發的次數。
    pub grow_count: u64,
    /// 每次 grow 的 (desired - current) 累加，代表實際申請的增量 bytes。
    pub grow_total_delta_bytes: u64,
}

impl MyLimiter {
    pub fn new(mem_limit_bytes: usize) -> Self {
        Self {
            mem_limit_bytes,
            wasm_mem_peak: 0,
            max_allocation: 0,
            grow_count: 0,
            grow_total_delta_bytes: 0,
        }
    }
    pub fn print_max(&self, type_of_max: &str) {
        let ratio = self.max_allocation as f64 / self.mem_limit_bytes as f64;
        if type_of_max == "parse" {
            //println!(
            //    "{} 該次實例最大memoey用量:{},佔比: {}",
            //    type_of_max, self.max_allocation, ratio
            //);
        }
    }

    /// 重置每批次的增量統計，在 Store 重用時於 parse() 前呼叫。
    /// `wasm_mem_peak` 不重置：它追蹤 Store 生命週期最高水位，
    /// 跨批次重用後不會再觸發 memory.grow，本欄反映第一批的峰值即可。
    pub fn reset_batch_stats(&mut self) {
        self.grow_count = 0;
        self.grow_total_delta_bytes = 0;
        self.max_allocation = 0;
    }
}

impl ResourceLimiter for MyLimiter {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.wasm_mem_peak {
            self.wasm_mem_peak = desired;
        }
        let grow = desired <= self.mem_limit_bytes;
        if desired > self.max_allocation {
            self.max_allocation = desired;
        }
        self.grow_count += 1;
        self.grow_total_delta_bytes += (desired.saturating_sub(current)) as u64;
        Ok(grow)
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

/// wasi:http 出站請求的白名單檢查：(host, port) → 是否放行。
/// 只有 transport store 會設定；由 runtime.rs 依 endpoint map 建立（支援熱重載）。
pub type HttpGuard = Arc<dyn Fn(&str, u16) -> bool + Send + Sync>;

pub struct MyState {
    pub ctx: WasiCtx,
    pub table: ResourceTable,
    pub limiter: MyLimiter,
    /// HTTP context — only populated for transport plugin stores;
    /// parse/format stores carry a default WasiHttpCtx but never link HTTP functions.
    pub http: WasiHttpCtx,
    /// wasi:http 出站白名單；None 代表此 store 沒有連結 wasi:http（parse/filter/format）。
    pub http_guard: Option<HttpGuard>,
}

/// parse／filter／format 共用的 WasiCtx：只繼承 stdout／stderr（plugin 除錯輸出），
/// 不繼承 stdin、不注入任何環境變數、不給任何 preopen、socket 位址檢查維持預設全拒。
pub fn plugin_wasi_ctx() -> WasiCtx {
    WasiCtxBuilder::new().inherit_stdout().inherit_stderr().build()
}

impl WasiView for MyState {
    fn ctx(&mut self) -> wasmtime_wasi::WasiCtxView<'_> {
        wasmtime_wasi::WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

impl wasmtime_wasi_http::WasiHttpView for MyState {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }

    /// 覆寫預設的出站請求：wasi:http 不經過 WasiCtx 的 socket_addr_check，
    /// 舊版的 endpoint 白名單因此只擋得住 raw sockets。這裡在送出前比對
    /// request 的 host:port，不在 endpoint map 中就回傳 HTTP-request-denied。
    fn send_request(
        &mut self,
        request: hyper::Request<HyperOutgoingBody>,
        config: OutgoingRequestConfig,
    ) -> wasmtime_wasi_http::HttpResult<HostFutureIncomingResponse> {
        let uri = request.uri();
        let host = uri.host().unwrap_or("").to_string();
        let default_port = if uri.scheme_str() == Some("https") { 443 } else { 80 };
        let port = uri.port_u16().unwrap_or(default_port);
        let allowed = match &self.http_guard {
            Some(guard) => guard(&host, port),
            // 沒有設定白名單的 store 不應該能送 HTTP：一律拒絕
            None => false,
        };
        if !allowed {
            eprintln!(
                "[SECURITY][transport] 拒絕 wasi:http 請求：{}:{} 不在 forwarder.yaml 的 endpoint map 中",
                host, port
            );
            return Err(ErrorCode::HttpRequestDenied.into());
        }
        Ok(default_send_request(request, config))
    }
}

// ── Shared plugin runtime slot ────────────────────────────────────────────────
//
// The config watcher increments `version` each time it hot-swaps the underlying WASM component.
// Pipeline threads snapshot the version before each batch; when it changes they rebuild their
// local Store/plugin from the new Engine+Component+Linker.

pub struct PluginRuntime {
    pub engine: Engine,
    pub component: Component,
    pub linker: Linker<MyState>,
    pub version: u64,
}

pub type SharedPlugin = Arc<RwLock<PluginRuntime>>;

/// Compile a parse / filter / format plugin and wrap it in a shared slot.
/// 載入時依 `stage` 與 `grant` 檢查 import，未授權直接回傳 `[capability]` 錯誤。
pub fn new_shared_runtime(wasm_path: &str, stage: Stage, grant: StageGrant) -> wasmtime::Result<SharedPlugin> {
    let (engine, component, linker) = build_runtime(wasm_path.to_string(), stage, grant)?;
    Ok(Arc::new(RwLock::new(PluginRuntime { engine, component, linker, version: 0 })))
}

/// Compile a transport plugin (async + WASI HTTP) and wrap it in a shared slot.
pub fn new_shared_transport_runtime(wasm_path: &str, grant: StageGrant) -> wasmtime::Result<SharedPlugin> {
    let (engine, component, linker) = build_transport_runtime(wasm_path.to_string(), grant)?;
    Ok(Arc::new(RwLock::new(PluginRuntime { engine, component, linker, version: 0 })))
}

/// Recompile a plugin from `new_path` and swap it into an existing shared slot.
/// Returns true if the swap succeeded, false if compilation failed (old plugin is retained).
/// 熱更換同樣經過能力檢查：新 plugin 若 import 了未授權介面，保留舊版並印出 `[capability]` 錯誤。
pub fn rebuild_shared_slot(slot: &SharedPlugin, new_path: &str, stage: Stage, grant: StageGrant, label: &str) -> bool {
    let result = if stage == Stage::Transport {
        build_transport_runtime(new_path.to_string(), grant)
    } else {
        build_runtime(new_path.to_string(), stage, grant)
    };
    match result {
        Ok((engine, component, linker)) => {
            let mut s = slot.write().unwrap();
            s.engine = engine;
            s.component = component;
            s.linker = linker;
            s.version += 1;
            eprintln!("[config] {} plugin hot-swapped → v{} ({})", label, s.version, new_path);
            true
        }
        Err(e) => {
            eprintln!("[config] {} plugin rebuild FAILED — keeping old plugin: {}", label, e);
            false
        }
    }
}

// ── Memory guard(RSS 水位線保險絲)────────────────────────────────────────────
//
// 問題:各元件(WASM mem_limit、buffer_size_mb、channel)各自守自己的配額,
// 但 process 總 RSS 沒有任何機制保證低於容器 cgroup 上限;超標時被 OOM kill,
// 對上游表現為 connection reset(掉資料),而不是 backpressure。
//
// 做法:背景執行緒每 100ms 讀 cgroup v2 的 memory.current / memory.max,
// 超過高水位(80%)就停止「讀取」輸入 socket/檔案——不斷線、不丟資料,
// kernel buffer 滿了以後上游自然被 TCP 流控卡住;降回低水位(65%)才恢復。
// 讀不到 cgroup 檔案(非容器環境或 memory.max = "max")時自動停用。

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

static INPUT_PAUSED: AtomicBool = AtomicBool::new(false);
static MEM_GUARD_INIT: OnceLock<()> = OnceLock::new();

// 預設水位偏保守:暫停輸入後,已在管線內的資料仍會經 parse/format 膨脹(2~4 倍),
// 需要足夠餘裕吸收;可用環境變數 MEM_GUARD_HIGH / MEM_GUARD_LOW 覆寫(0~1 小數)。
const MEM_GUARD_HIGH_DEFAULT: f64 = 0.70;
const MEM_GUARD_LOW_DEFAULT: f64 = 0.55;
const MEM_GUARD_POLL_MS: u64 = 100;

fn guard_level(env_key: &str, default: f64) -> f64 {
    std::env::var(env_key)
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| *v > 0.0 && *v < 1.0)
        .unwrap_or(default)
}

fn read_cgroup_mem() -> Option<(u64, u64)> {
    let cur: u64 = std::fs::read_to_string("/sys/fs/cgroup/memory.current")
        .ok()?
        .trim()
        .parse()
        .ok()?;
    // memory.max 內容為 "max" 代表無上限 → parse 失敗 → guard 停用
    let max: u64 = std::fs::read_to_string("/sys/fs/cgroup/memory.max")
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some((cur, max))
}

/// 啟動 memory guard(冪等;由各 input reader 呼叫)。
pub fn init_mem_guard() {
    MEM_GUARD_INIT.get_or_init(|| {
        let Some((cur, max)) = read_cgroup_mem() else {
            eprintln!("[mem-guard] disabled: cgroup memory.current/memory.max 不可讀或無上限");
            return;
        };
        let high = guard_level("MEM_GUARD_HIGH", MEM_GUARD_HIGH_DEFAULT);
        let low = guard_level("MEM_GUARD_LOW", MEM_GUARD_LOW_DEFAULT).min(high - 0.05);
        eprintln!(
            "[mem-guard] enabled: limit={}MB high={:.0}% low={:.0}% current={}MB",
            max / 1048576,
            high * 100.0,
            low * 100.0,
            cur / 1048576
        );
        thread::spawn(move || {
            loop {
                // 暫停期間強制把 allocator 留存的空閒 heap 還給 OS:
                // glibc 預設不歸還,memory.current 會停在歷史高點,
                // 不 trim 的話低水位永遠到不了 → 輸入永久暫停(活鎖)。
                if INPUT_PAUSED.load(Ordering::Relaxed) {
                    unsafe {
                        libc::malloc_trim(0);
                    }
                }
                if let Some((cur, max)) = read_cgroup_mem() {
                    let ratio = cur as f64 / max as f64;
                    let paused = INPUT_PAUSED.load(Ordering::Relaxed);
                    if !paused && ratio >= high {
                        INPUT_PAUSED.store(true, Ordering::Relaxed);
                        eprintln!(
                            "[mem-guard] PAUSE input: mem={}MB ({:.0}% of {}MB)",
                            cur / 1048576,
                            ratio * 100.0,
                            max / 1048576
                        );
                    } else if paused && ratio <= low {
                        INPUT_PAUSED.store(false, Ordering::Relaxed);
                        eprintln!(
                            "[mem-guard] RESUME input: mem={}MB ({:.0}%)",
                            cur / 1048576,
                            ratio * 100.0
                        );
                    }
                }
                thread::sleep(Duration::from_millis(MEM_GUARD_POLL_MS));
            }
        });
    });
}

#[inline]
pub fn input_paused() -> bool {
    INPUT_PAUSED.load(Ordering::Relaxed)
}

/// Listen on a TCP port; each connected client's lines are forwarded into the channel.
pub fn spawn_tcp_reader(host: String, port: u16, tx: SyncSender<String>) {
    init_mem_guard();
    thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("[tcp-input] tokio runtime build failed");
        rt.block_on(async move {
            let addr = format!("{}:{}", host, port);
            let listener = tokio::net::TcpListener::bind(&addr)
                .await
                .unwrap_or_else(|e| panic!("[tcp-input] cannot bind {}: {}", addr, e));
            eprintln!("[tcp-input] listening on {}", addr);
            loop {
                match listener.accept().await {
                    Ok((socket, peer)) => {
                        eprintln!("[tcp-input] accepted {}", peer);
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            use tokio::io::{AsyncBufReadExt, BufReader};
                            let mut lines = BufReader::new(socket).lines();
                            loop {
                                // mem-guard 高水位:暫停讀取(不斷線),讓 TCP 流控回壓上游
                                while input_paused() {
                                    tokio::time::sleep(Duration::from_millis(50)).await;
                                }
                                match lines.next_line().await {
                                    Ok(Some(line)) => {
                                        if tx.send(line).is_err() {
                                            break;
                                        }
                                    }
                                    _ => break,
                                }
                            }
                            eprintln!("[tcp-input] {} disconnected", peer);
                        });
                    }
                    Err(e) => eprintln!("[tcp-input] accept error: {}", e),
                }
            }
        });
    });
}

/// Tail a file (like `tail -f`): seek to end on startup, then stream new lines as they arrive.
pub fn spawn_tail_reader(path: String, tx: SyncSender<String>) {
    use std::fs::File;
    use std::io::{BufReader, Seek, SeekFrom};

    init_mem_guard();
    thread::spawn(move || {
        let mut file = loop {
            match File::open(&path) {
                Ok(f) => break f,
                Err(e) => {
                    eprintln!("[tail-input] waiting for '{}': {}", path, e);
                    thread::sleep(Duration::from_secs(1));
                }
            }
        };
        let _ = file.seek(SeekFrom::End(0));
        eprintln!("[tail-input] tailing '{}'", path);

        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            // mem-guard 高水位:暫停讀取,檔案 offset 停在原地不丟資料
            while input_paused() {
                thread::sleep(Duration::from_millis(50));
            }
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => thread::sleep(Duration::from_millis(50)),
                Ok(_) => {
                    if line.ends_with('\n') {
                        line.pop();
                        if line.ends_with('\r') {
                            line.pop();
                        }
                    }
                    if !line.is_empty() && tx.send(line.clone()).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    eprintln!("[tail-input] read error: {}", e);
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
    });
}

// 負責把log一條條變成LineItem後塞入channel
pub fn spawn_stdin_reader(tx: SyncSender<String>) {
    thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            if let Ok(content) = line {
                if tx.send(content).is_err() {
                    break;
                }
            }
        }
    });
}

// ── epoch interruption 開關（batch.epoch_interruption，預設 true）──────────────
// 啟動時由 main.rs 依 YAML 設定一次；build_runtime／build_transport_runtime（含熱抽換重建）
// 與 runtime.rs 的 deadline／ticker 都讀這個值，因此整個 process 一致。
static EPOCH_INTERRUPTION: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// 設定是否以 epoch interruption 作為 stage watchdog（只在啟動時呼叫）。
pub fn set_epoch_interruption(on: bool) {
    EPOCH_INTERRUPTION.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// 目前是否啟用 epoch interruption。
pub fn epoch_interruption_enabled() -> bool {
    EPOCH_INTERRUPTION.load(std::sync::atomic::Ordering::Relaxed)
}

/// 設定 store 的 epoch deadline；epoch interruption 停用時不設任何 deadline。
pub fn arm_epoch_deadline(store: &mut wasmtime::Store<MyState>, ticks: u64) {
    if epoch_interruption_enabled() {
        store.set_epoch_deadline(ticks);
    }
}

/// Build an engine/component/linker for parse or format plugins (WASI only, no HTTP).
///
/// 舊版以 `add_to_linker_sync` 連結整套 WASI p2；現在改為：
///   1. 載入後先以 component_type().imports() 比對 stage allowlist（capability.rs），
///      未授權 import 直接拒絕載入；
///   2. linker 只連結該 stage 授權的介面（逐介面 add_to_linker）。
pub fn build_runtime(
    wasm_path: String,
    stage: Stage,
    grant: StageGrant,
) -> wasmtime::Result<(Engine, Component, Linker<MyState>)> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.strategy(Strategy::Cranelift);
    config.cranelift_opt_level(OptLevel::Speed);
    config.epoch_interruption(epoch_interruption_enabled());

    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, &wasm_path)?;
    capability::check_component(stage, grant, &wasm_path, &engine, &component)?;

    let mut linker: Linker<MyState> = Linker::new(&engine);
    capability::add_wasi_to_linker_sync(&mut linker, grant)?;

    Ok((engine, component, linker))
}

/// `--check-plugin` 用：把載入流程拆成「編譯／能力檢查／連結／instantiate_pre」四段計時（毫秒），
/// 用來量測載入期能力檢查相對於整體載入的開銷。與 build_runtime／build_transport_runtime
/// 使用相同的 engine 設定與步驟；能力檢查失敗時回傳 Err（含已量到的段落）。
pub fn timed_load_breakdown(wasm_path: &str, stage: Stage, grant: StageGrant) -> Result<[f64; 4], String> {
    let ms = |t: std::time::Instant| t.elapsed().as_secs_f64() * 1000.0;
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.strategy(Strategy::Cranelift);
    config.cranelift_opt_level(OptLevel::Speed);
    config.epoch_interruption(epoch_interruption_enabled());
    let engine = Engine::new(&config).map_err(|e| e.to_string())?;
    let t = std::time::Instant::now();
    let component = Component::from_file(&engine, wasm_path).map_err(|e| format!("compile: {e}"))?;
    let compile_ms = ms(t);
    let t = std::time::Instant::now();
    let checked = capability::check_component(stage, grant, wasm_path, &engine, &component);
    let check_ms = ms(t);
    if let Err(e) = checked {
        return Err(format!("compile_ms={compile_ms:.3} check_ms={check_ms:.3} refused: {e}"));
    }
    let t = std::time::Instant::now();
    let mut linker: Linker<MyState> = Linker::new(&engine);
    if stage == Stage::Transport {
        capability::add_wasi_to_linker_async(&mut linker, grant).map_err(|e| e.to_string())?;
        wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker).map_err(|e| e.to_string())?;
    } else {
        capability::add_wasi_to_linker_sync(&mut linker, grant).map_err(|e| e.to_string())?;
    }
    let link_ms = ms(t);
    let t = std::time::Instant::now();
    linker.instantiate_pre(&component).map_err(|e| e.to_string())?;
    let pre_ms = ms(t);
    Ok([compile_ms, check_ms, link_ms, pre_ms])
}

/// Build an engine/component/linker for the transport plugin (WASI + HTTP, async).
///
/// Async WASI 介面（capability::add_wasi_to_linker_async）+ `add_only_http_to_linker_async`
/// enable non-blocking I/O so that `Func::call_async` can drive HTTP requests without
/// the 4096 B write limit that exists in sync WASI.
///
/// 能力限制：transport 只連結基礎介面 + wasi:http；raw sockets 與 filesystem
/// 預設不連結（需 YAML 明確授權），載入時同樣先做 import 檢查。
pub fn build_transport_runtime(
    wasm_path: String,
    grant: StageGrant,
) -> wasmtime::Result<(Engine, Component, Linker<MyState>)> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.strategy(Strategy::Cranelift);
    config.cranelift_opt_level(OptLevel::Speed);
    config.epoch_interruption(epoch_interruption_enabled());

    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, &wasm_path)?;
    capability::check_component(Stage::Transport, grant, &wasm_path, &engine, &component)?;

    let mut linker: Linker<MyState> = Linker::new(&engine);
    capability::add_wasi_to_linker_async(&mut linker, grant)?;
    wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker)?;

    Ok((engine, component, linker))
}
