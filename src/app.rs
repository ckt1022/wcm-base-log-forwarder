use std::io::{self, BufRead};
use std::sync::{Arc, RwLock, mpsc::SyncSender};
use std::thread;
use std::time::Duration;

use wasmtime::{
    Config, Engine, OptLevel, ResourceLimiter, Strategy,
    component::{Component, Linker},
};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiView};
use wasmtime_wasi_http::WasiHttpCtx;

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

pub struct MyState {
    pub ctx: WasiCtx,
    pub table: ResourceTable,
    pub limiter: MyLimiter,
    /// HTTP context — only populated for transport plugin stores;
    /// parse/format stores carry a default WasiHttpCtx but never link HTTP functions.
    pub http: WasiHttpCtx,
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
    Ok(Arc::new(RwLock::new(PluginRuntime { engine, component, linker, version: 0 })))
}

/// Compile a transport plugin (async + WASI HTTP) and wrap it in a shared slot.
pub fn new_shared_transport_runtime(wasm_path: &str) -> wasmtime::Result<SharedPlugin> {
    let (engine, component, linker) = build_transport_runtime(wasm_path.to_string())?;
    Ok(Arc::new(RwLock::new(PluginRuntime { engine, component, linker, version: 0 })))
}

/// Recompile a plugin from `new_path` and swap it into an existing shared slot.
/// Returns true if the swap succeeded, false if compilation failed (old plugin is retained).
pub fn rebuild_shared_slot(slot: &SharedPlugin, new_path: &str, is_transport: bool, label: &str) -> bool {
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
            eprintln!("[config] {} plugin hot-swapped → v{} ({})", label, s.version, new_path);
            true
        }
        Err(e) => {
            eprintln!("[config] {} plugin rebuild FAILED — keeping old plugin: {}", label, e);
            false
        }
    }
}

/// Listen on a TCP port; each connected client's lines are forwarded into the channel.
pub fn spawn_tcp_reader(host: String, port: u16, tx: SyncSender<String>) {
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
                            while let Ok(Some(line)) = lines.next_line().await {
                                if tx.send(line).is_err() {
                                    break;
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

/// Build an engine/component/linker for parse or format plugins (WASI only, no HTTP).
pub fn build_runtime(wasm_path: String) -> wasmtime::Result<(Engine, Component, Linker<MyState>)> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.strategy(Strategy::Cranelift);
    config.cranelift_opt_level(OptLevel::Speed);

    let engine = Engine::new(&config)?;
    let mut linker: Linker<MyState> = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;

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

    let engine = Engine::new(&config)?;
    let mut linker: Linker<MyState> = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    wasmtime_wasi_http::add_only_http_to_linker_async(&mut linker)?;

    let component = Component::from_file(&engine, wasm_path)?;

    Ok((engine, component, linker))
}
