//! WCM component runtime、host 狀態及輸入來源的建立工具。
//!
//! 本檔案負責建立 Wasmtime Component Model linker、管理可熱更新的共享 plugin、限制
//! Wasm 記憶體、提供 H1/H2 計時標記，並從 TCP、檔案或 stdin 將 log 送入 pipeline。

use std::cell::RefCell;
use std::io::{self, BufRead};
use std::sync::{Arc, RwLock, mpsc::SyncSender};
use std::thread;
use std::time::{Duration, Instant};

use wasmtime::{
    Config, Engine, OptLevel, ResourceLimiter, Strategy,
    component::{Component, Linker},
};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiView};
use wasmtime_wasi_http::WasiHttpCtx;

thread_local! {
    static FILTER_COPY_MARKERS: RefCell<(Option<Instant>, Option<Instant>, u8)> =
        const { RefCell::new((None, None, 0)) };
}

/// 清除目前執行緒保存的 WCM component H1/H2 邊界時間標記。
pub fn reset_copy_markers() {
    FILTER_COPY_MARKERS.with(|markers| *markers.borrow_mut() = (None, None, 0));
}

/// 取出目前執行緒記錄的 H1、H2 時間與 component 呼叫標記次數。
pub fn take_copy_markers() -> (Option<Instant>, Option<Instant>, u8) {
    FILTER_COPY_MARKERS.with(|markers| *markers.borrow())
}

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
    /// 建立具有指定線性記憶體上限的 Wasm 資源限制器。
    pub fn new(mem_limit_bytes: usize) -> Self {
        Self {
            mem_limit_bytes,
            wasm_mem_peak: 0,
            max_allocation: 0,
            grow_count: 0,
            grow_total_delta_bytes: 0,
        }
    }
    /// 計算本次最大配置量相對於上限的比例，供指定階段的除錯輸出使用。
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
    /// 記錄 Wasm 線性記憶體成長統計，並拒絕超過設定上限的成長要求。
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

    /// 允許 Wasm 資料表成長；此限制器只限制線性記憶體。
    fn table_growing(
        &mut self,
        _current: usize,
        _desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        Ok(true)
    }
}

pub struct MyState {
    pub ctx: WasiCtx,
    pub table: ResourceTable,
    pub limiter: MyLimiter,
    /// HTTP context — only populated for transport plugin stores;
    /// parse/format stores carry a default WasiHttpCtx but never link HTTP functions.
    pub http: WasiHttpCtx,
}

impl WasiView for MyState {
    /// 將 host state 的 WASI context 與 resource table 暴露給 Wasmtime WASI。
    fn ctx(&mut self) -> wasmtime_wasi::WasiCtxView<'_> {
        wasmtime_wasi::WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

impl wasmtime_wasi_http::WasiHttpView for MyState {
    /// 回傳 transport component 使用的 WASI HTTP context。
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }
    /// 回傳 transport component 共用的 WASI resource table。
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
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
pub fn new_shared_runtime(wasm_path: &str) -> wasmtime::Result<SharedPlugin> {
    let (engine, component, linker) = build_runtime(wasm_path.to_string())?;
    Ok(Arc::new(RwLock::new(PluginRuntime {
        engine,
        component,
        linker,
        version: 0,
    })))
}

/// Compile a transport plugin (async + WASI HTTP) and wrap it in a shared slot.
pub fn new_shared_transport_runtime(wasm_path: &str) -> wasmtime::Result<SharedPlugin> {
    let (engine, component, linker) = build_transport_runtime(wasm_path.to_string())?;
    Ok(Arc::new(RwLock::new(PluginRuntime {
        engine,
        component,
        linker,
        version: 0,
    })))
}

/// Recompile a plugin from `new_path` and swap it into an existing shared slot.
/// Returns true if the swap succeeded, false if compilation failed (old plugin is retained).
pub fn rebuild_shared_slot(
    slot: &SharedPlugin,
    new_path: &str,
    is_transport: bool,
    label: &str,
) -> bool {
    let result = if is_transport {
        build_transport_runtime(new_path.to_string())
    } else {
        build_runtime(new_path.to_string())
    };
    match result {
        Ok((engine, component, linker)) => {
            let mut s = slot.write().unwrap();
            s.engine = engine;
            s.component = component;
            s.linker = linker;
            s.version += 1;
            eprintln!(
                "[config] {} plugin hot-swapped → v{} ({})",
                label, s.version, new_path
            );
            true
        }
        Err(e) => {
            eprintln!(
                "[config] {} plugin rebuild FAILED — keeping old plugin: {}",
                label, e
            );
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

/// 從環境變數讀取介於 0 與 1 之間的記憶體水位，無有效設定時使用預設值。
fn guard_level(env_key: &str, default: f64) -> f64 {
    std::env::var(env_key)
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|v| *v > 0.0 && *v < 1.0)
        .unwrap_or(default)
}

/// 讀取 cgroup v2 的目前記憶體用量與上限；不可用或無上限時回傳 `None`。
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
/// 回傳 memory guard 目前是否要求輸入 reader 暫停讀取。
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

/// 逐行讀取標準輸入，並將每筆 log 送入 pipeline 的輸入 channel。
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

/// Build an engine/component/linker for parse or format plugins (WASI only, no HTTP).
pub fn build_runtime(wasm_path: String) -> wasmtime::Result<(Engine, Component, Linker<MyState>)> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.strategy(Strategy::Cranelift);
    config.cranelift_opt_level(OptLevel::Speed);
    config.epoch_interruption(true);

    let engine = Engine::new(&config)?;
    let mut linker: Linker<MyState> = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    linker.root().func_wrap("time-sign", |_store, (): ()| {
        let timestamp = Instant::now();
        FILTER_COPY_MARKERS.with(|markers| {
            let mut markers = markers.borrow_mut();
            match markers.2 {
                0 => markers.0 = Some(timestamp),
                1 => markers.1 = Some(timestamp),
                _ => {}
            }
            markers.2 = markers.2.saturating_add(1);
        });
        Ok(())
    })?;

    let component = Component::from_file(&engine, wasm_path)?;

    Ok((engine, component, linker))
}

/// Build an engine/component/linker for the transport plugin (WASI + HTTP, async).
///
/// `add_to_linker_async` + `add_only_http_to_linker_async` enable non-blocking I/O
/// so that `Func::call_async` can drive HTTP requests without the 4096 B write limit
/// that exists in sync WASI.
pub fn build_transport_runtime(
    wasm_path: String,
) -> wasmtime::Result<(Engine, Component, Linker<MyState>)> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.strategy(Strategy::Cranelift);
    config.cranelift_opt_level(OptLevel::Speed);
    config.epoch_interruption(true);

    let engine = Engine::new(&config)?;
    let mut linker: Linker<MyState> = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker)?;

    let component = Component::from_file(&engine, wasm_path)?;

    Ok((engine, component, linker))
}
