use std::io::{self, BufRead};
use std::sync::mpsc::SyncSender;
use std::thread;

use wasmtime::{
    component::{Component, Linker},
    Config, Engine, InstanceAllocationStrategy, OptLevel, PoolingAllocationConfig, ResourceLimiter,
    Strategy,
};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiView};

use crate::config::LineItem;

// parser-plugin world：產生 ParserPlugin 及 pipeline-process 所有型別
wasmtime::component::bindgen!({
    world: "parser-plugin",
    path: "wit/log_host.wit",
});

// format-plugin world：用 `with` 重用上面已生成的 LogEntry / LogLevel，
// 使兩個 bindgen 共用同一 Rust 型別，channel 傳遞時無需轉換。
pub mod format_bindings {
    wasmtime::component::bindgen!({
        world: "format-plugin",
        path: "wit/log_host.wit",
        with: {
            "local:log-process/pipeline-process":
                super::local::log_process::pipeline_process,
        }
    });
}

/// WASM 線性記憶體追蹤器，實作 ResourceLimiter 介面。
pub struct MyLimiter {
    mem_limit_bytes: usize,
    pub wasm_mem_peak: usize,
}

impl MyLimiter {
    pub fn new(mem_limit_bytes: usize) -> Self {
        Self { mem_limit_bytes, wasm_mem_peak: 0 }
    }
}

impl ResourceLimiter for MyLimiter {
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        if desired > self.wasm_mem_peak {
            self.wasm_mem_peak = desired;
        }
        Ok(desired <= self.mem_limit_bytes)
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
}

impl WasiView for MyState {
    fn ctx(&mut self) -> wasmtime_wasi::WasiCtxView<'_> {
        wasmtime_wasi::WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

pub fn spawn_stdin_reader(tx: SyncSender<LineItem>) {
    thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            if let Ok(content) = line {
                if tx.send(LineItem { bytes: content.into_bytes() }).is_err() {
                    break;
                }
            }
        }
    });
}

pub fn build_runtime(
    mem_limit_bytes: usize,
    wasm_path: String,
) -> wasmtime::Result<(Engine, Linker<MyState>, Component)> {
    let mut config = Config::new();
    config.wasm_component_model(true);
    config.strategy(Strategy::Cranelift);
    config.cranelift_opt_level(OptLevel::Speed);

    let mut pooling_config = PoolingAllocationConfig::new();
    pooling_config.total_memories(20);
    pooling_config.max_memory_size(mem_limit_bytes);
    config.allocation_strategy(InstanceAllocationStrategy::Pooling(pooling_config));

    let engine = Engine::new(&config)?;
    let mut linker: Linker<MyState> = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;

    let component = Component::from_file(&engine, wasm_path)?;

    Ok((engine, linker, component))
}
