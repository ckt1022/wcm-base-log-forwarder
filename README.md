# wcm-base-log-forwarder

A simple log forwarder tool built with Rust, designed to process and forward streamed logs efficiently.

---

## 🚀 Getting Started

### 1. Build the Project

```bash
cargo build
```

---

### 2. Run with Log Generator

Use the following command to pipe logs into the forwarder:

```bash
go run tools/gen/main.go -rate 5000 -duration 30 | ./target/debug/wcm-base-log-forwarder
```

---

## ⚙️ Parameters

### Log Generator (`tools/gen/main.go`)

#### 基本參數

| Flag | 預設 | 說明 |
|------|------|------|
| `-rate int` | `5000` | 目標輸出速率（行/秒），為各流量模式的平均基準 |
| `-duration int` | `30` | 執行秒數，`0` 表示不限時 |
| `-mode string` | `json-simple` | 輸出格式（見下表） |
| `-invalid-rate float` | `0.05` | mode=`invalid` 時混入非法行的比例 |
| `-buffer int` | `1048576` | stdout 緩衝區大小（byte） |
| `-flush-ms int` | `100` | stdout flush 間隔（毫秒） |
| `-seed int` | `0` | 隨機種子，`0` = 用當前時間 |
| `-log-file string` | `gen.log` | 診斷訊息輸出檔，`-` 表示 stderr |

#### 流量波形參數（`-traffic`）

| Flag | 預設 | 說明 |
|------|------|------|
| `-traffic string` | `flat` | 流量模式：`flat`、`wave`、`bursty` |
| `-wave-amp float` | `0.6` | 正弦波振幅（0.0–0.9），速率在 `rate×(1-amp)` 到 `rate×(1+amp)` 之間 |
| `-wave-period float` | `60.0` | 正弦波週期（秒） |
| `-spike-mult float` | `3.0` | `bursty` 突發期間疊加在波形上的倍率 |
| `-spike-freq float` | `2.0` | `bursty` 平均每分鐘突發次數 |
| `-spike-dur float` | `5.0` | `bursty` 每次突發持續秒數 |

**流量模式說明：**

- `flat`：固定速率，長期精確等於 `-rate`（預設，向後相容）
- `wave`：正弦波，高峰低谷交替，長期平均**精確等於** `-rate`；適合可控實驗
- `bursty`：正弦波底層加隨機突發尖峰，模擬真實流量；長期平均近似 `-rate`；適合壓力測試

診斷訊息（寫入 `-log-file`）會顯示當前倍率，突發期間加註 `[SPIKE]`：
```
[gen] total=56422 inst=2109/s avg=2169/s mult=1.21x [SPIKE]
```

#### 日誌格式（`-mode`）

| 模式 | 說明 |
|------|------|
| `json-simple` | 固定格式 JSON，少量欄位（基本效能測試） |
| `json-complex` | JSON 含多欄位與較長訊息（壓力測試） |
| `json-mixed` | simple + complex 混合（接近真實情況） |
| `invalid` | 混入無法解析的行（測試 parser 錯誤處理） |
| `logfmt-simple` | 扁平 key=value 格式 |
| `logfmt-complex` | logfmt 含較多欄位與較長訊息 |
| `logfmt-mixed` | logfmt simple + complex 混合 |
| `syslog-simple` | RFC5424 風格 syslog，基本欄位 |
| `syslog-complex` | RFC5424 syslog，附帶延伸欄位 |
| `syslog-mixed` | syslog simple + complex 混合 |

#### 使用範例

```bash
# 固定速率（基準測試）
go run tools/gen/main.go -rate 5000 -duration 60 | ./target/debug/wcm-base-log-forwarder

# 正弦波流量（60s 週期，±60% 振幅）
go run tools/gen/main.go -rate 5000 -duration 120 -traffic wave | ./target/debug/wcm-base-log-forwarder

# 突發流量（每 30s 一次週期，加上每分鐘 2 次突發至 3x）
go run tools/gen/main.go -rate 5000 -duration 120 -traffic bursty -wave-period 30 | ./target/debug/wcm-base-log-forwarder

# 短週期突發，用於快速驗證背壓處理
go run tools/gen/main.go -rate 5000 -duration 30 -traffic bursty -wave-period 10 -spike-freq 4 -spike-dur 3 | ./target/debug/wcm-base-log-forwarder

# 把診斷訊息印到 stderr，方便即時觀察
go run tools/gen/main.go -rate 5000 -traffic wave -log-file - | ./target/debug/wcm-base-log-forwarder
```

---

## 📌 Requirements

* Rust (https://www.rust-lang.org/)
* Go (https://golang.org/)

---

## 📂 Project Structure

```
.
├── src/
├── target/
├── README.md
```

---

## 🧪 Use Cases

This tool is suitable for:

* High-throughput log testing
* Pipeline validation
* Streaming performance benchmarking

---

## ⚠️ Notes

* The compiled binary will be located at:

  ```
  target/debug/wasm-base-log-forwarder
  ```
* Adjust `rate` and `duration` based on your testing requirements.
* Make sure the `abc.go` file exists in the specified relative path.

---

## 注意編譯細節與錯誤

### WIT 介面版本管理

- `wit/log_plugin.wit` 是唯一的正規來源（canonical source），所有 plugin 目錄下的 `wit/log_plugin.wit` 都必須與此完全一致，修改後需手動同步複製（`cp wit/log_plugin.wit test-plugins/.../wit/log_plugin.wit`）
- WIT package 宣告加入版本號（例如 `package local:log-process@0.1.0`）後，`wkg wit build` 輸出的檔名會帶版本（`local:log-process@0.1.0.wasm`），對應的 tinygo `--wit-package` 參數與 README 範例也必須同步更新，否則找不到套件
- `wit-bindgen-go generate` 的 `--package-root` 必須設為 `example.com/internal`（而非 `example.com`），否則生成的 import 路徑缺少 `internal/` 前綴，導致編譯錯誤

### TinyGo 版本與 Go 工具鏈相容性

- TinyGo 0.40.x 僅支援 Go **1.19–1.25**，若系統預設 Go 版本 ≥ 1.26，編譯時會報錯 `requires go version 1.19 through 1.25, got go1.26`
- 設定 `GOROOT` 環境變數單獨使用無效，TinyGo 檢查版本時走的是 `PATH` 裡的 `go` 二進位，必須**同時設定 `GOROOT` 和 `PATH`** 才能生效：
  ```bash
  GO125="$(GOTOOLCHAIN=go1.25.0 go env GOROOT)"
  GOROOT="$GO125" PATH="$GO125/bin:$PATH" tinygo build ...
  ```
- 若系統沒有 go1.25，可用 Go 工具鏈管理自動下載：`GOTOOLCHAIN=go1.25.0 go version`（首次執行會自動下載至 `$GOPATH/pkg/mod/golang.org/toolchain@v0.0.1-go1.25.0.linux-amd64`）
- `go.mod` 的 `go` 指令版本必須 ≤ 1.25（例如 `go 1.25.0`），否則 TinyGo 直接拒絕編譯，與 GOROOT 設定無關

### Go 綁定重新生成（wit-bindgen-go）

- 修改 WIT 後必須先 `wkg wit build` 重建 `.wasm` 套件，再執行 `wit-bindgen-go generate`，否則生成的型別與新 WIT 不一致
- 刪除舊的 `internal/local/` 目錄再重新生成，避免殘留舊介面名稱（例如舊的 `parse-process/` 目錄在改名為 `pipeline-process/` 後若不清除，會同時存在兩個版本造成混亂）
- format plugin 的 `go.mod` 原本沒有 `wit-bindgen-go` 工具依賴，需先執行 `go get go.bytecodealliance.org/cmd/wit-bindgen-go@latest` 或直接呼叫已安裝的 `$GOPATH/bin/wit-bindgen-go` 二進位

### WIT 介面升級（parsed-entry 與 log-entry id）

- 新版 WIT 將 parser 的回傳型別從 `list<log-entry>` 改為 `list<parsed-entry>`（不含 `id` 欄位），**id 由 host 在 parse 後統一分配**，plugin 不應也不能設定 id
- Host 端（`runtime.rs`）需在收到 `Vec<ParsedEntry>` 後將每筆資料加上 id 轉換為 `Vec<LogEntry>` 再傳入後續 plugin；id 命名規則建議使用 `seq * MAX_BATCH + index` 確保批次間全域唯一
- format plugin 的 `LogEntry` 多了 `id: u64` 欄位，但 format 邏輯本身不需要使用 id，不填入格式化輸出是正確行為

### Rust host bindgen（wasmtime::component::bindgen!）

- 刪除 `wit/log_host.wit` 並改用整個 `wit/` 目錄（`path: "wit"`）後，`format_bindings` 的 `with` 區塊必須使用正確的 Rust 模組路徑 `local::log_process::pipeline_process`（底線分隔，對應 WIT 的 kebab-case `local:log-process/pipeline-process`）
- 兩個 bindgen（parser 與 format）共用同一份 `pipeline-process` 型別的前提是 `format_bindings` 的 `with` 設定正確指向 parser bindgen 生成的模組，否則 channel 傳遞 `Vec<LogEntry>` 時會出現型別不相容錯誤

---

## Transport Plugin 整合 — 遇到的問題與解決方法

### 問題一：transport.wasm 非 component 格式

- **現象**：載入 `transport.wasm` 時報錯 `attempted to parse a wasm module with a component parser`
- **原因**：`wasm-compose` 輸出兩個檔案：`transport.wasm`（原始 module）與 `transport_component.wasm`（已組合的 component）；pipeline 需要的是後者
- **解決方法**：`src/main.rs` 的路徑改為 `transport_component.wasm`

---

### 問題二：wasmtime sync WASI blocking-write 限制 4096 B

- **現象**：第一批次呼叫 `send()` 時 trap：`Buffer too large for blocking-write-and-flush (expected at most 4096)`；後續所有批次繼續 trap：`cannot enter component instance`
- **原因**：
  - wasmtime sync 模式下 `blocking-write-and-flush` 每次最多寫入 4096 B
  - transport plugin（Rust）將整個 batch 的資料一次性寫入 HTTP request body，大批次（~260 KB）遠超限制
  - 第一次 trap 後 component instance 進入損壞狀態，後續呼叫全部失敗
- **解決方法**：在 host 端（`transport_loop`）對 `FormattedBatch.lines` 分片，每片最多 `max_transport_chunk`（預設 20 行 ≈ 2–4 KB）呼叫一次 `send()`；每次呼叫觸發一個獨立 HTTP POST 請求

---

### 問題三：`parse_handle` 超出作用域

- **現象**：`parse_handle` 在 `match` block 內定義，離開 block 後無法 `.join()`，導致編譯失敗
- **解決方法**：將 `parse_handle` 宣告為 `Option<JoinHandle<...>>`，在 `if let Some(...)` 外部持有

---

### 問題四：`BatchConfig` 移入 closure 後無法再使用

- **現象**：`cfg` 被 parse closure 的 `move` 捕獲後，後續取 `cfg.transport_endpoint` 時編譯報錯 `borrow of moved value`
- **原因**：`BatchConfig` 原本實作 `Copy`（`#[derive(Clone, Copy)]`），加入 `transport_endpoint: String` 後 `String` 不能 `Copy`，trait 自動失效
- **解決方法**：改為只 `#[derive(Clone)]`，在 spawn 前預先 `clone()` 或個別 `clone` 需要的欄位，再傳入各 closure

---

### 問題五：`post_return()` deprecated

- **現象**：呼叫 `Func::post_return()` 出現編譯警告 `use of deprecated method: no longer needs to be called`
- **原因**：wasmtime 42.x 已將 `post_return` 改為 no-op 並標記 deprecated
- **解決方法**：移除所有 `post_return()` 呼叫

---

## 重大改動

### Pipeline 架構：三段並行 Thread

- **改動前**：parse 在獨立 thread，format 在主 thread（串行）
- **改動後**：parse / format / transport 各自在獨立 thread，以 `sync_channel` 背壓串聯

```
stdin → [reader thread]
             ↓  LineItem channel (cap 5000)
        [parse thread]
             ↓  ParsedBatch channel (cap 32)
        [format thread]
             ↓  FormattedBatch channel (cap 16)
        [transport thread]
```

- 任一階段停用時自動 spawn drain thread 排空 channel，確保上游不 block

---

### `MyState` 加入 HTTP 支援

- 新增 `http: WasiHttpCtx` 欄位，實作 `wasmtime_wasi_http::WasiHttpView`
- parse / format 的 store 帶著預設 `WasiHttpCtx::new()` 但 linker 不加入 HTTP 函式，無額外開銷
- `build_transport_runtime()` 在 linker 額外呼叫 `add_only_http_to_linker_sync()`，使 transport component 能執行 HTTP 請求

---

### Transport Loop 設計

- 使用 **Val API**（動態型別）呼叫 transport component，避免 bindgen 需要映射複雜 wasi:http 型別
- 單一長存活 Store：`init()` 只呼叫一次，`send()` 對每個 `FormattedBatch` 的每個 chunk 各呼叫一次
- 結束後呼叫 `report-usage()` 取得 plugin 自行累計的傳送 byte 數
- 新增 `max_transport_chunk`（預設 20 行）控制每次 `send()` 的資料大小，規避 wasmtime sync WASI 4096 B 寫入限制

---

### Transport 統計數據（TransportStats）

| 欄位 | 說明 |
|---|---|
| `total_batches` | 成功傳送的批次數 |
| `total_input_lines` | 傳送的格式化行數 |
| `total_input_bytes` | Host 計算的傳送 byte 數 |
| `total_bytes_reported` | Plugin `report-usage()` 回報的累計 byte 數 |
| `total_elapsed` | Transport 累計耗時 |
| `wasm_mem_peak_max` | WASM 線性記憶體峰值 |

Pipeline summary 末尾新增 Trans 區塊，與 Parse / Format 格式一致。


## 觀察
### 2026/05/02
1. 不使用GO就不會有OOM的問題，C都沒有出現，我的推測是C沒有GC，因此就算重新用實例也是直接覆蓋。
2. 如果把實例用完後，清除並放回，重複使用，GO就會出現OOM，C則不會，若GO也是用完後丟棄，則也沒有OOM的問題。
3. 利用C寫的插件速度比GO快太多

- 透過在GO plugin中斷開已經使用過的結構指標，讓GC能正確清理已經不用的記憶體，大幅減少OOM的情況
僅剩下syslog simple偶爾會出現OOM
- 若沒有重置結構，則syslog每次重用都會OOM，幾乎無法重用 -> 目前OOM次數很少。


## 工作日記
### 2026/05/02
1. 把log產生器改成有流量變化，並把用法寫在README.md
2. 做好了C的Format plugin
3. 用重置pointer的方式讓GO的GC能順利完成，減少OOM發生
4. 能夠重用實例，每個實例用到OOM為止被刪除。
5. 論文的方向為測量實例啟動、ABI複製到component與回傳到host各自的花費時間
   比較flat(length+data)與結構化的傳送方式的時間差異
   強調安全性。

## 接下來的工作
1. 用有GC的語言的插件作為對照組，判斷斷開pointer是否真的有用，
    觀察C#、Python用原本寫法(不預先分配、不斷開)是否會有GC問題
    改用現在的寫法是否能改善。

    預先分配可能能處理因為頻繁mem.grow產生的OOM。
    斷開是要處理重複使用實例的情況下GC的問題。

    另一種結合方法，讓指標在不用的情況下就斷開，增加一次批次GC的清理量(尚未驗證)，預期能夠提高單個批次的量。
2. 提高transport的吞吐量。
3. 比較結構化傳送與flat傳送的花費時間差異
4. 新增route與filter
5. 如何保證wasm安全、如何保證log處裡後的對應。
6. enrich加在parse(optional,最後再做)
