use std::io::{self, BufRead};
use std::sync::mpsc::SyncSender;
use std::thread;

use wasmtime::{
    component::{Component, Linker},
    Config, Engine, InstanceAllocationStrategy, OptLevel, PoolingAllocationConfig, Strategy,
};
use wasmtime_wasi::{ResourceTable, WasiCtx, /*WasiCtxBuilder,*/ WasiView};

use crate::config::LineItem;

wasmtime::component::bindgen!({
    world: "parser-plugin",
    path: "/home/ckt1022/wcm-base-log-forwarder/wit/log_plugin.wit",
});

pub struct MyState {
    pub ctx: WasiCtx,
    pub table: ResourceTable,
    pub limiter: wasmtime::StoreLimits,
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