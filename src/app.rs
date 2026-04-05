use std::io::{self, BufRead};
use std::sync::mpsc::SyncSender;
use std::thread;

use wasmtime::{
    component::{Component, Linker},
    Config, Engine, OptLevel, ResourceLimiter, Strategy,
};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiView};

use crate::config::LineItem;

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

// 負責把log一條條變成LineItem後塞入channel
pub fn spawn_stdin_reader(tx: SyncSender<LineItem>) {
    // 開條thread負責接收log，並放入channel
    thread::spawn(move || {
        let stdin = io::stdin();
        // 因為stdin是全域資源，需要上鎖確保只有一個thread在修改
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

    // 使用預設的 on-demand allocator：每次 instantiate 都取得全新的清零記憶體。
    // Pooling allocator 為了效能重複使用記憶體 slot，但不會重置 TinyGo 的零初始化
    // globals（heap bump pointer 等），導致第二次以後的 instance 繼承舊的 heap 狀態
    // 而觸發 OOM 或 out-of-bounds memory access。
    let engine = Engine::new(&config)?;
    let mut linker: Linker<MyState> = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;

    let component = Component::from_file(&engine, wasm_path)?;

    Ok((engine, linker, component))
}
