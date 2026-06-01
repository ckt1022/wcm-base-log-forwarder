use anyhow::{bail, Context, Result};
use wasmtime::component::{Component, Linker, Val};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxView};
use wasmtime_wasi_http::WasiHttpCtx;

// ── Host state ─────────────────────────────────────────────────────────────

struct HostState {
    ctx: WasiCtx,
    table: ResourceTable,
    http: WasiHttpCtx,
}

impl wasmtime_wasi::WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.ctx,
            table: &mut self.table,
        }
    }
}

impl wasmtime_wasi_http::WasiHttpView for HostState {
    fn ctx(&mut self) -> &mut WasiHttpCtx {
        &mut self.http
    }
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

// ── Main ──────────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let wasm_path = args.get(1).map(String::as_str).unwrap_or(
        "../../test-plugins/rust-plugin/transport/target/wasm32-unknown-unknown/release/transport.wasm",
    );
    let endpoint = args
        .get(2)
        .map(String::as_str)
        .unwrap_or("http://127.0.0.1:8080/ingest");

    println!("[host] Loading component: {}", wasm_path);
    println!("[host] Target endpoint:   {}", endpoint);

    // ── Engine ────────────────────────────────────────────────────────────
    let mut config = Config::new();
    config.wasm_component_model(true);
    let engine = Engine::new(&config)?;

    // ── Component ─────────────────────────────────────────────────────────
    let component = Component::from_file(&engine, wasm_path)
        .map_err(|e| anyhow::anyhow!("failed to load {}: {}", wasm_path, e))?;

    // ── Linker ────────────────────────────────────────────────────────────
    let mut linker: Linker<HostState> = Linker::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    wasmtime_wasi_http::add_only_http_to_linker_sync(&mut linker)?;

    // ── Store ─────────────────────────────────────────────────────────────
    let wasi_ctx = wasmtime_wasi::WasiCtxBuilder::new()
        .inherit_stdio()
        .build();
    let state = HostState {
        ctx: wasi_ctx,
        table: ResourceTable::new(),
        http: WasiHttpCtx::new(),
    };
    let mut store = Store::new(&engine, state);

    // ── Instantiate ───────────────────────────────────────────────────────
    let instance = linker.instantiate(&mut store, &component)?;

    // ── Call: init ────────────────────────────────────────────────────────
    println!("\n[host] Calling init({})...", endpoint);

    let init_fn = instance
        .get_func(&mut store, "init")
        .context("component has no export 'init'")?;

    // Build transport-config record
    // WIT variant auth-method::none → Val::Variant("none", None)
    let config_val = Val::Record(vec![
        ("endpoint".into(), Val::String(endpoint.into())),
        ("auth".into(), Val::Variant("none".into(), None)),
        ("connect-timeout-ms".into(), Val::U32(5_000)),
        ("request-timeout-ms".into(), Val::U32(10_000)),
        ("retry".into(), Val::Option(None)),
        ("tls".into(), Val::Option(None)),
        ("extra-headers".into(), Val::List(vec![])),
        ("max-batch-bytes".into(), Val::U32(0)),
    ]);

    let mut init_results = vec![Val::Bool(false)];
    init_fn.call(&mut store, &[config_val], &mut init_results)?;
    init_fn.post_return(&mut store)?;

    check_result("init", &init_results[0])?;

    for i in 0..5{
    // ── Call: send ────────────────────────────────────────────────────────
    let test_logs: &[&str] = &[
        "{\"level\":\"info\",\"ts\":\"2026-04-05T10:00:00Z\",\"msg\":\"server started\",\"port\":8080}",
        "{\"level\":\"warn\",\"ts\":\"2026-04-05T10:00:01Z\",\"msg\":\"high memory\",\"pct\":82}",
        "{\"level\":\"error\",\"ts\":\"2026-04-05T10:00:02Z\",\"msg\":\"connection refused\",\"host\":\"db\"}",
    ];

    println!("\n[host] Calling send() with {} log entries...", test_logs.len());

    let send_fn = instance
        .get_func(&mut store, "send")
        .context("component has no export 'send'")?;

    // list<list<u8>>
    let output_data = Val::List(
        test_logs
            .iter()
            .map(|s| Val::List(s.bytes().map(Val::U8).collect()))
            .collect(),
    );

    let mut send_results = vec![Val::Bool(false)];

    
        send_fn.call(&mut store, &[output_data], &mut send_results)?;
        send_fn.post_return(&mut store)?;

        check_result("send", &send_results[0])?;
    }
    // ── Call: report-usage ────────────────────────────────────────────────
    let usage_fn = instance
        .get_func(&mut store, "report-usage")
        .context("component has no export 'report-usage'")?;

    let mut usage_results = vec![Val::U64(0)];
    usage_fn.call(&mut store, &[], &mut usage_results)?;
    usage_fn.post_return(&mut store)?;

    if let Val::U64(bytes) = usage_results[0] {
        println!("\n[host] report-usage() -> {} bytes sent", bytes);
    }

    println!("\n[host] All tests passed!");
    Ok(())
}

fn check_result(fn_name: &str, val: &Val) -> Result<()> {
    match val {
        Val::Result(Ok(_)) => {
            println!("[host] {}() -> Ok", fn_name);
            Ok(())
        }
        Val::Result(Err(Some(err))) => {
            bail!("[host] {}() failed: {:?}", fn_name, err)
        }
        Val::Result(Err(None)) => {
            bail!("[host] {}() failed: (no error detail)", fn_name)
        }
        other => bail!("[host] {}() unexpected result type: {:?}", fn_name, other),
    }
}
