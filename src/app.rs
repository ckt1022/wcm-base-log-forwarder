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

wasmtime::component::bindgen!({
    world: "parser-plugin",
    path: "wit/log_plugin.wit",
});

/// WASM 線性記憶體追蹤器，實作 ResourceLimiter 介面。
///
/// Wasmtime 在每次 WASM `memory.grow` 指令時回呼 `memory_growing()`，
/// 因此可以精確記錄 WASM 線性記憶體的峰值用量。
///
/// 這是測量 WASM 記憶體最準確的方式：
///   - 涵蓋 Go runtime stack、GC metadata、WASM globals 等所有非 heap 記憶體
///   - 不受 GC stop-the-world 的採樣時機影響
///   - 與 plugin 內部的 HeapInuse（僅 Go heap）互補，兩者合起來提供完整視角
pub struct MyLimiter {
    mem_limit_bytes: usize,
    /// 本次 Store 生命週期內觀測到的 WASM 線性記憶體峰值（bytes）。
    /// 每次呼叫 process_batch 建立新 Store 時重置為 0。
    pub wasm_mem_peak: usize,
}

impl MyLimiter {
    pub fn new(mem_limit_bytes: usize) -> Self {
        Self {
            mem_limit_bytes,
            wasm_mem_peak: 0,
        }
    }
}

impl ResourceLimiter for MyLimiter {
    /// 每次 WASM memory.grow 時觸發：記錄峰值並強制執行上限。
    fn memory_growing(
        &mut self,
        _current: usize,
        desired: usize,
        _maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        // 更新峰值（desired 為本次 grow 後的總大小）
        if desired > self.wasm_mem_peak {
            self.wasm_mem_peak = desired;
        }
        // 超過設定上限時拒絕 grow，Wasmtime 會回傳 OOM trap
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
    // 開條新的thread來接收資料
    thread::spawn(move || {
        let stdin = io::stdin();
        for line in stdin.lock().lines() {
            if let Ok(content) = line {
                if tx.send(LineItem {
                    bytes: content.into_bytes(),
                })
                .is_err()
                {
                    break;
                }
            }
        }
    });
}

pub fn build_runtime_parse(
    mem_limit_bytes: usize,parse_path: String
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

    let component = Component::from_file(
        &engine,
        parse_path,
    )?;

    Ok((engine, linker, component))
}