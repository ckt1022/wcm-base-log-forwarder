# transport — WASM Transport Plugin (Rust)

A WebAssembly Component Model (WCM) plugin that implements the `transport-plugin` world defined in the shared WIT interface. It receives a batch of formatted log bytes and HTTP POSTs them to a configurable endpoint.

---

## WIT Interface

```wit
world transport-plugin {
    export init: func(config: transport-config) -> result<_, plugin-error>;
    export send: func(output-data: list<list<u8>>) -> result<_, plugin-error>;
    export report-usage: func() -> u64;
}
```

| Export | Description |
|---|---|
| `init` | Called once before any `send`. Stores config and validates the endpoint. Re-calling resets state. |
| `send` | Accepts `list<list<u8>>` (one `list<u8>` per formatted log line) and HTTP POSTs them to the configured endpoint. Splits into multiple requests when `max-batch-bytes > 0`. |
| `report-usage` | Returns total bytes successfully sent since last `init`. |

The plugin imports WASI HTTP (`wasi:http/outgoing-handler`) for network access, provided by the host runtime.

---

## Features

- **Auth methods**: `none`, `bearer-token`, `basic-auth`, `api-key`
- **Retry with exponential backoff**: configurable via `retry-config`
- **Batch splitting**: respects `max-batch-bytes` (0 = unlimited)
- **Connect / request timeouts** in milliseconds
- **Custom HTTP headers** via `extra-headers`
- **Usage tracking**: `report-usage()` returns total bytes sent

---

## Build

### Prerequisites

```bash
# Rust toolchain with wasm32-unknown-unknown target
rustup target add wasm32-unknown-unknown

# wasm-tools (for lifting core module → WASM Component)
cargo install wasm-tools
```

### Step 1 — Compile to WASM core module

```bash
cargo build --target wasm32-unknown-unknown --release
```

Output: `target/wasm32-unknown-unknown/release/transport.wasm` (core WASM module)

### Step 2 — Lift to WASM Component

```bash
wasm-tools component new \
  target/wasm32-unknown-unknown/release/transport.wasm \
  -o target/wasm32-unknown-unknown/release/transport_component.wasm
```

Output: `target/wasm32-unknown-unknown/release/transport_component.wasm` (WASM Component)

### Why two steps?

`wit_bindgen::generate!` on `wasm32-unknown-unknown` generates a core WASM module with component-type annotations. `wasm-tools component new` wraps it into a proper WASM Component binary (magic bytes `\0asm\r\0\1\0`) that wasmtime can load via `Component::from_file`.

> **Note on `wasm32-wasip2`**: The `wasm32-wasip2` target collides the WIT `send` export name with POSIX libc's `send()` socket function at link time. `wasm32-unknown-unknown` avoids this because it ships no POSIX libc.

### Verify the component

```bash
wasm-tools component wit target/wasm32-unknown-unknown/release/transport_component.wasm
```

---

## Integration Test

A minimal test setup lives in `/test/` at the repo root.

### 1. Start the Python receiver

```bash
python3 test/server.py
# [server] Listening on http://127.0.0.1:8080/ingest
```

The server accepts `POST /ingest` and prints each received log line.

### 2. Build the Rust test host

```bash
cd test/host
cargo build
```

### 3. Run the host

```bash
./test/host/target/debug/transport-test-host \
  test-plugins/rust-plugin/transport/target/wasm32-unknown-unknown/release/transport_component.wasm \
  http://127.0.0.1:8080/ingest
```

Expected output:

```
[host] Loading component: ...
[host] Calling init(http://127.0.0.1:8080/ingest)...
[host] init() -> Ok

[host] Calling send() with 3 log entries...
[host] send() -> Ok

[host] report-usage() -> 239 bytes sent

[host] All tests passed!
```

Server side:

```
[server] --- Received batch #1 ---
[server] Content-Type: application/octet-stream
[server] Bytes: 239
[server] Line 1: {"level":"info",...}
[server] Line 2: {"level":"warn",...}
[server] Line 3: {"level":"error",...}
```

---

## Host Requirements

The host runtime must provide:

| WASI import | Purpose |
|---|---|
| `wasi:http/outgoing-handler@0.2.0` | Outgoing HTTP requests |
| `wasi:http/types@0.2.0` | HTTP request/response types |
| `wasi:io/streams@0.2.0` | Body write stream |
| `wasi:io/poll@0.2.0` | Blocking poll for response |
| `wasi:clocks/monotonic-clock@0.2.0` | Retry sleep |

Add to your host's `Linker`:

```rust
wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
wasmtime_wasi_http::add_only_http_to_linker_sync(&mut linker)?;
```

---

## 踩坑紀錄（Rust Transport Plugin 實作過程）

### 1. `generate!` 找不到 WASI 依賴套件

**問題**：`path: "wit/log_plugin.wit"` 指向單一檔案，`wit_bindgen` 無法自動發現 `wit/deps/` 子目錄下的 WASI 套件，導致 `package wasi:http` 等無法解析。

**解法**：改成 `path: "wit"`（指向目錄），`wit_bindgen` 會自動掃描整個目錄樹，包含 `deps/` 子目錄。

---

### 2. WASI 型別未生成 — `PluginError`、`TransportConfig` 找不到

**問題**：`generate!` 預設只生成直接在 `world` 宣告的 interface，WASI import 的 binding（`wasi::http`、`wasi::io` 等）不會自動產生。

**解法**：在 `generate!` macro 加入 `generate_all` 選項，強制對所有引用到的 interface 都產生 Rust binding。

---

### 3. 型別重複定義 — `use` 造成衝突

**問題**：`generate_all` 會把出現在 export 函數簽名中的型別（`PluginError`、`TransportConfig`）直接放在 crate root，若再額外 `use local::log_process::pipeline_process::PluginError`，會產生重複定義的 compile error。

**解法**：只針對 **不在 export 函數簽名中出現的型別**（如 `AuthMethod`、`RetryConfig`）加 `use local::log_process::transport_types::{AuthMethod, RetryConfig}`；`PluginError` 和 `TransportConfig` 直接用，不需要額外 `use`。

---

### 4. WASI OutputStream `write()` 回傳值誤解

**問題**：誤以為 `stream.write(bytes)` 回傳 `Result<u64, StreamError>`（已寫入位元組數），實際上回傳 `Result<(), StreamError>`，且 `write()` 是非同步的（不保證全部寫入）。

**解法**：改用 `stream.blocking_write_and_flush(body_bytes)` 一次把整個 body 寫入並 flush，這是同步阻塞版本，不需要自行處理部分寫入。

---

### 5. `wasm32-wasip2` 目標的 `send` symbol 衝突

**問題**：WIT export 函數命名為 `send`，在 `wasm32-wasip2` target 下，POSIX libc 也有 `send(fd, buf, len, flags)` socket 函數，link 時兩者名稱相同導致錯誤，且即使用 `--allow-multiple-definition` 也會選到錯誤的定義，component export 失效。

**解法**：改用 `wasm32-unknown-unknown` target。此 target 不帶 POSIX libc，不存在 `send` symbol，衝突消失。

---

### 6. `wasm32-unknown-unknown` 編譯出 core module，不是 WASM Component

**問題**：`wasmtime` 的 `Component::from_file` 需要 WASM Component 二進位格式（magic bytes `\0asm\r\0\1\0`），但 `cargo build --target wasm32-unknown-unknown` 只產生 core WASM module，直接載入會失敗。

**解法**：加第二步驟：
```bash
wasm-tools component new transport.wasm -o transport_component.wasm
```
`wasm-tools` 把 core module 包裝成合法的 WASM Component，wasmtime 才能正常載入。

---

### 7. `[package.metadata.component]` 引入不相容依賴

**問題**：在 `Cargo.toml` 加入 `[package.metadata.component]` 設定時，`cargo component` 工具鏈會自動拉入 `errno`、`getrandom`、`rustix` 等依賴，這些 crate 與 `wasm32-unknown-unknown` target 不相容（假設有 POSIX 環境），導致編譯失敗。

**解法**：完全移除 `[package.metadata.component]` 區塊。本專案不需要 `cargo component`，直接用 `cargo build` + `wasm-tools component new` 兩步完成。

---

### 8. HTTP body 未帶 `Content-Length`，server 收到 0 bytes

**問題**：建立 HTTP request 時忘記在 headers 設定 `Content-Length`，某些 server 或 WASI HTTP 實作不會自動計算，導致 server 端讀到空 body。

**解法**：在 `build_headers()` 函數中明確加入：
```rust
set_header(&fields, "content-length", body_len.to_string().as_bytes())?;
```
並在呼叫 `build_headers` 時傳入 `body_bytes.len()`。

---

### 9. wasmtime 42 的 `WasiView` trait 簽名不同

**問題**：舊版 wasmtime 的 `WasiView::ctx()` 回傳 `&mut WasiCtx`；wasmtime 42 改成回傳 `WasiCtxView<'_>` 結構體（同時持有 ctx 和 table 的可變參考），直接套舊版寫法會 compile error。

**解法**：按 wasmtime 42 的正確簽名實作：
```rust
fn ctx(&mut self) -> WasiCtxView<'_> {
    WasiCtxView { ctx: &mut self.ctx, table: &mut self.table }
}
```

---

### 10. `wasmtime::Error` 無法直接用 `.with_context()`

**問題**：wasmtime 的 `Error` 型別沒有實作 `std::error::Error` trait（anyhow 需要），所以 `.with_context(|| ...)` 無法使用。

**解法**：改用 `.map_err(|e| anyhow::anyhow!("failed to load {}: {}", path, e))?` 手動包裝錯誤訊息。

---

## Directory Layout

```
transport/
├── Cargo.toml            # cdylib, wit-bindgen = "0.37"
├── README.md             # this file
├── src/
│   └── lib.rs            # plugin implementation
├── wit/
│   ├── log_plugin.wit    # WIT world + interface definitions
│   └── deps/             # fetched by `wkg wit fetch`
│       ├── wasi-http-0.2.0/
│       ├── wasi-io-0.2.0/
│       └── wasi-clocks-0.2.0/
└── wkg.lock              # locked WIT dependency digests
```
