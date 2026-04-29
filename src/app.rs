use std::io::{self, BufRead};
use std::sync::mpsc::SyncSender;
use std::thread;

use wasmtime::{
    component::{Component, Linker},
    Config, Engine, OptLevel, ResourceLimiter, Strategy,
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
    pub fn print_max(&self,type_of_max:&str){
        let ratio = self.max_allocation as f64 / self.mem_limit_bytes as f64;
        if type_of_max == "parse"{
            println!("{} 該次實例最大memoey用量:{},佔比: {}",type_of_max,self.max_allocation,ratio);
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
pub fn build_runtime(
    wasm_path: String,
) -> wasmtime::Result<(Engine, Component, Linker<MyState>)> {
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
