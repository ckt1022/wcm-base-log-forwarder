# Rust Plugin – WASM Component 建立過程

本文件記錄如何以 Rust 建立實作 `copy_overhead.wit` 中兩個 world 的 WASM Component：

- **`parse/`** — `parser-plugin` world：
  - 匯出 `parse`：解析 JSON log（`ts` / `level` / `msg` / `att` 與其他欄位），填入 `parsed-entry` 回傳
  - 匯出 `report-usage`：回傳上次 `parse` 的執行耗時（奈秒）
  - 在 `parse` 主函數**最開頭與最結尾**各呼叫一次 host 匯入的 `time-sign()`
  - 解析邏輯參考 `test-plugins/c-plugin/parse/main.c`
- **`filter/`** — `reduction-plugin` world：
  - 匯出 `filter`：保留 `level >= warn` 的 log（丟棄 debug / info），逐筆回傳 `filter-result { id, keep }`
  - 匯出 `report-usage`：回傳上次 `filter` 的執行耗時（奈秒）
  - 在 `filter` 主函數**最開頭與最結尾**各呼叫一次 `time-sign()`
  - 過濾邏輯參考 `test-plugins/c-plugin/filter/filter_impl.c`

以下先以 `parse` 完整走一遍建立流程，`filter` 的差異在文末獨立一節說明。

## 前置需求

本次建立時使用的工具版本：

```console
$ rustc --version
rustc 1.93.1 (01f6ddf75 2026-02-11)

$ cargo --version
cargo 1.93.1 (083ac5135 2025-12-15)

$ wasm-tools --version
wasm-tools 1.253.0

$ rustup target list --installed | grep wasip2
wasm32-wasip2
```

若缺少 `wasm32-wasip2` target 或 `wasm-tools`，先安裝：

```bash
rustup target add wasm32-wasip2
cargo install wasm-tools
```

> 與 C plugin 不同：C 需要編到 `wasm32-wasip1` 再用 `wasm-tools component new`
> 搭配 `wasi_snapshot_preview1.reactor.wasm` adapter 封裝成 component；
> Rust 的 `wasm32-wasip2` target 會**直接產出 component**，不需要 adapter 步驟。

## Step 1：建立專案結構並複製 WIT 定義

```bash
cd test/ours/copy_overhead/rust_plugin
mkdir -p parse/src parse/wit
cp ../wit/copy_overhead.wit parse/wit/
# wasi 0.2.0 的介面定義（clocks/io/cli/random...），從既有 rust transport plugin 複製
cp -r ../../../../test-plugins/rust-plugin/transport/wit/deps parse/wit/deps
```

結果：

```console
$ ls -R parse/wit | head
parse/wit:
copy_overhead.wit
deps

parse/wit/deps:
wasi-cli-0.2.0
wasi-clocks-0.2.0
wasi-filesystem-0.2.0
wasi-http-0.2.0
wasi-io-0.2.0
wasi-random-0.2.0
wasi-sockets-0.2.0
```

`wit/deps/` 必須包含 world 中 `import wasi:...@0.2.0` 用到的所有 package 定義，
否則 `wit_bindgen::generate!` 巨集會在編譯期解析 WIT 失敗。

## Step 2：撰寫 `Cargo.toml`

`parse/Cargo.toml`：

```toml
[package]
name = "parse"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]          # component 必須編成動態庫（reactor 模式，無 main）

[dependencies]
wit-bindgen = { version = "0.37", default-features = false, features = ["macros", "realloc"] }
serde_json = "1"                 # JSON 解析（對應 C 版的 cJSON）

[profile.release]
opt-level = "s"                  # 以體積優先
strip = true
```

## Step 3：撰寫 `src/lib.rs`

完整程式碼見 `parse/src/lib.rs`。重點結構：

```rust
wit_bindgen::generate!({
    path: "wit",                 // 指向 Step 1 的 wit/ 目錄
    world: "parser-plugin",      // copy_overhead.wit 中的 world 名稱
    generate_all,                // 一併產生 wasi:clocks 等 import 的 bindings
});

/// 實際解析邏輯抽成獨立函數，讓 parse 有單一出口，
/// 錯誤路徑（early return）也能走到結尾的 time-sign
fn do_parse(raw_data: &[String]) -> Result<Vec<ParsedEntry>, ParseError> {
    // ... 逐筆 serde_json 解析，填入 ParsedEntry ...
}

struct Component;

impl Guest for Component {       // Guest trait = world 的所有 export
    fn parse(raw_data: Vec<String>) -> Result<Vec<ParsedEntry>, ParseError> {
        time_sign();             // ← 主函數最開頭呼叫 host 匯入的 time-sign
        let start_ns = monotonic_clock::now();
        let result = do_parse(&raw_data);
        LAST_EXEC_NS.set(monotonic_clock::now().saturating_sub(start_ns));
        time_sign();             // ← 主函數最結尾再呼叫一次（含錯誤路徑）
        result
    }

    fn report_usage() -> u64 { LAST_EXEC_NS.get() }
}

export!(Component);              // 註冊 export，產生實際的 wasm 匯出符號
```

型別對應（由 `generate!` 巨集產生）：

| WIT | Rust |
|---|---|
| `parsed-entry` | `ParsedEntry`（world 有 `use`，直接在 crate 根） |
| `parse-error` | `ParseError`（同上） |
| `log-level` | `local::log_process::pipeline_process::LogLevel` |
| `import time-sign: func()` | 根模組的 `time_sign()` |
| `wasi:clocks/monotonic-clock` | `wasi::clocks::monotonic_clock::now()` |

解析邏輯（同 `test-plugins/c-plugin/parse/main.c`）：

1. `ts` → `timestamp`（非字串則為 `""`）
2. `level` → 數字直接映射 0–6；字串做不分大小寫的 debug/info/warn/error 比對，其餘為 `info`
3. `msg` → `message`
4. `tags`：第一個為 `("lang", "Rust")`，接著收集根層非保留欄位
   （`ts`/`level`/`msg`/`att` 以外）與 `att` 物件內所有欄位；非字串值以 compact JSON 字串表示
5. `targettag`：依 level 決定 route 標籤（照 C 版邏輯，目前一律 `"A"`）
6. 任一筆 JSON 解析失敗 → 回傳 `parse-error::invalid-format("JSON Parse Error")`
7. 執行耗時以 `monotonic_clock::now()` 前後相減，存入全域狀態供 `report-usage` 回傳

踩雷紀錄：world 中 `use pipeline-process.{parsed-entry, parse-error}` 會讓
`generate!` 把這兩個型別**直接匯出到 crate 根模組**，若再手動
`use local::log_process::pipeline_process::{ParsedEntry, ParseError}` 會出現
E0255 重複定義錯誤——只需 `use` 沒被 world re-export 的 `LogLevel`。

## Step 4：編譯

```console
$ cd parse
$ cargo build --target wasm32-wasip2 --release
   Compiling parse v0.1.0 (.../test/ours/copy_overhead/rust_plugin/parse)
    Finished `release` profile [optimized] target(s) in 0.52s
```

產物即為 component（不需再跑 `wasm-tools component new`）：

```console
$ ls -lh target/wasm32-wasip2/release/parse.wasm
-rw-r--r-- 2 ckt1022 ckt1022 115K ... target/wasm32-wasip2/release/parse.wasm
```

## Step 5：驗證

### 5.1 二進位驗證

```console
$ wasm-tools validate target/wasm32-wasip2/release/parse.wasm && echo "Validated OK"
Validated OK
```

### 5.2 檢查 component 實際的 imports / exports

```console
$ wasm-tools component wit target/wasm32-wasip2/release/parse.wasm
package root:component;

world root {
  import local:log-process/pipeline-process@0.3.0;
  import wasi:clocks/monotonic-clock@0.2.6;
  import wasi:io/error@0.2.6;
  import wasi:io/streams@0.2.6;
  import wasi:cli/environment@0.2.6;
  import wasi:cli/exit@0.2.6;
  import wasi:cli/stderr@0.2.6;
  use local:log-process/pipeline-process@0.3.0.{parsed-entry, parse-error};
  import time-sign: func();

  export parse: func(raw-data: list<string>) -> result<list<parsed-entry>, parse-error>;
  export report-usage: func() -> u64;
}
```

確認重點：

- ✅ `export parse` / `export report-usage` 簽名與 world 一致
- ✅ `import time-sign: func()` 存在（host 需提供）
- ✅ **沒有** `wasi:filesystem` 與 `wasi:sockets` import ——
  符合 `parser-plugin` world 刻意排除檔案系統與網路能力的安全限制；
  Rust std 只在實際用到時才會拉入這些 import，本 plugin 未使用故不出現
- ℹ️ 實際 import 是 world 允許集合的**子集**（std 只用到 clocks/io/cli 的部分介面）
- ℹ️ wasi 版本為 `0.2.6`（rustc 1.93 std 內建版本），與 world 宣告的 `0.2.0`
  屬同一 semver 相容線，wasmtime 的 `wasmtime-wasi` linker 可直接滿足

## filter（reduction-plugin world）

`filter/` 的建立流程與 `parse` 完全相同（Step 1–5），僅列出差異：

### 專案結構與 WIT

```bash
cd test/ours/copy_overhead/rust_plugin
mkdir -p filter/src filter/wit
cp ../wit/copy_overhead.wit filter/wit/
cp -r parse/wit/deps filter/wit/deps
```

`reduction-plugin` world 使用 `include wasi:cli/imports@0.2.0`（完整 wasi:cli 集合，
含 filesystem / sockets 的介面定義），deps 目錄同樣要齊全。

### Cargo.toml 差異

- `name = "filter"`，其餘與 parse 相同
- 不需要 `serde_json`（filter 收到的是已解析的 `log-entry` 結構，不再碰 JSON）
- edition 需用 **2021**：edition 2024 下 `unsafe_op_in_unsafe_fn` 預設開啟，
  wit-bindgen 0.37 的 `export!` 巨集產生的程式碼會冒出 E0133 警告

### src/lib.rs 重點

```rust
wit_bindgen::generate!({
    path: "wit",
    world: "reduction-plugin",   // 注意：world 名稱是 reduction-plugin，不是 filter-plugin
    generate_all,
});

/// 保留 level >= warn 的 log（同 C plugin 的 MIN_KEEP_LEVEL）
const MIN_KEEP_LEVEL: LogLevel = LogLevel::Warn;

fn do_filter(struct_data: &[LogEntry]) -> Result<Vec<FilterResult>, PluginError> {
    Ok(struct_data
        .iter()
        .map(|entry| FilterResult {
            id: entry.id,
            keep: entry.level as u8 >= MIN_KEEP_LEVEL as u8,
        })
        .collect())
}

impl Guest for Component {
    fn filter(struct_data: Vec<LogEntry>) -> Result<Vec<FilterResult>, PluginError> {
        time_sign();             // ← 最開頭
        let start_ns = monotonic_clock::now();
        let result = do_filter(&struct_data);
        LAST_EXEC_NS.set(monotonic_clock::now().saturating_sub(start_ns));
        time_sign();             // ← 最結尾
        result
    }
    fn report_usage() -> u64 { LAST_EXEC_NS.get() }
}
```

過濾邏輯（同 `test-plugins/c-plugin/filter/filter_impl.c`）：逐筆回傳
`filter-result { id, keep }`，`keep = level >= warn`；level 比較利用
wit-bindgen 產生的 enum 是 `#[repr(u8)]`（debug=0 … emerg=6），以 `as u8` 轉數值比大小。

### 編譯與驗證

```console
$ cd filter
$ cargo build --target wasm32-wasip2 --release
   Compiling filter v0.1.0 (.../test/ours/copy_overhead/rust_plugin/filter)
    Finished `release` profile [optimized] target(s) in 0.63s

$ wasm-tools validate target/wasm32-wasip2/release/filter.wasm && echo "filter Validated OK"
filter Validated OK

$ wasm-tools component wit target/wasm32-wasip2/release/filter.wasm
package root:component;

world root {
  import local:log-process/pipeline-process@0.3.0;
  import wasi:clocks/monotonic-clock@0.2.6;
  import wasi:io/error@0.2.6;
  import wasi:io/streams@0.2.6;
  import wasi:cli/environment@0.2.6;
  import wasi:cli/exit@0.2.6;
  import wasi:cli/stderr@0.2.6;
  use local:log-process/pipeline-process@0.3.0.{log-entry, filter-result, plugin-error};
  import time-sign: func();

  export filter: func(struct-data: list<log-entry>) -> result<list<filter-result>, plugin-error>;
  export report-usage: func() -> u64;
}
```

✅ `export filter` / `report-usage` 簽名與 world 一致、`import time-sign` 存在；
實際 import 為 world 允許集合的子集（雖然 world `include wasi:cli/imports`，
但程式未用到 filesystem / sockets，最終 component 不會 import 它們）。

## 產物

| 檔案 | 說明 |
|---|---|
| `parse/target/wasm32-wasip2/release/parse.wasm` | parser component（約 115 KB） |
| `filter/target/wasm32-wasip2/release/filter.wasm` | reduction component（約 59 KB） |

> 注意：這些 component 需要 host 提供 `time-sign` import，因此無法用
> `wasmtime run` 直接執行，必須由實作了該 import 的自訂 host 載入。
