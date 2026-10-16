# WCM Log Forwarder — 專案技術文件

## 1. 整體架構

本專案是以 **Rust + Wasmtime** 實作的日誌轉發框架，核心概念是將各處理階段封裝成可熱替換的 WebAssembly Component Model (WCM) 插件。

### 管線拓撲

```
Input
  ┌─────────────────────────────────┐
  │ TCP Listener (Tokio async)      │   ← mode=tcp
  │ File Tail Reader (polling 50ms) │   ← mode=tail
  │ Stdin Reader                   │   ← 定義在 app.rs，目前未接入 main.rs
  └───────────────┬─────────────────┘
                  ↓  sync_channel (channel_capacity=150000)
            [parse dispatcher thread]
                  ↓  sync_channel (64 WorkBatch)
            [parse worker thread × PARSE_WORKERS=1]
                  ↓  sync_channel (20000 ParsedBatch)  ← 硬編碼
            [filter thread] (可選，stages.filter)
                  ↓  sync_channel (20000 ParsedBatch)  ← 硬編碼
            [format thread] (可選，stages.format)
                  ↓  sync_channel (20000 FormattedBatch) ← 硬編碼
            [transport router thread]
                  ↓  sync_channel (1024 SendTask)
            [transport worker thread × transport_workers=5] (可選，stages.transport)
```

### 並行執行緒數量

| 階段 | Thread 數 | 說明 |
|------|-----------|------|
| Input | 1 (TCP 為 Tokio 事件迴圈) | TCP 接受多連線但同在一個 OS thread |
| Config watcher | 1 | 週期性重載設定與插件 |
| Parse dispatcher | 1 | 累積行、觸發 flush 條件 |
| Parse worker | 1 (PARSE_WORKERS 常數) | 持有 ParsePool，執行 WASM parse |
| Filter | 1 | 長存活 Store，逐批次呼叫 filter() |
| Format | 1 | 每個 chunk 建立新 Store，呼叫 format() |
| Transport router | 1 | 依 targettag 分流到 per-endpoint buffer |
| Transport worker | 5 (transport_workers) | 各自建立獨立 WASM 實例，發送 HTTP POST |

**各階段並不並行**：各自在獨立 thread，以有背壓的 `sync_channel` 串聯，前後兩階段是生產者-消費者關係。同一階段內只有一個 thread 在運算（transport 例外，有 N 個 worker 搶工作佇列）。

---

## 2. 檔案與函數說明

### `src/main.rs`

程式進入點，負責組裝整個系統。

| 函數 | 作用 |
|------|------|
| `main()` | 讀取 YAML 設定、編譯各插件 WASM、啟動 config watcher、啟動 input reader、呼叫 `run_pipeline()` |

**互動流程**：
`main()` 呼叫 `config::load_app_config()` → 呼叫 `app::new_shared_runtime()` / `new_shared_transport_runtime()` 建立插件槽 → 呼叫 `config::spawn_config_watcher()` 啟動熱更換監視 → 呼叫 `app::spawn_tcp_reader()` 或 `spawn_tail_reader()` 啟動輸入 → 呼叫 `runtime::run_pipeline()` 進入主管線。

---

### `src/config.rs`

設定資料結構、熱重載邏輯、統計結構、Batch 工具類別。

#### 主要型別

| 型別 | 說明 |
|------|------|
| `AppConfig` | YAML 頂層結構（plugins / stages / input / batch / endpoint / config_reload_secs） |
| `PluginsConfig` | 各插件的 WASM 路徑（parse / parse_noop / filter / format / transport） |
| `PipelineStages` | filter / format / transport 是否啟用（bool） |
| `InputConfig` / `InputMode` | 輸入來源設定（tcp 或 tail） |
| `TcpInputConfig` | host + port |
| `TailInputConfig` | 要追蹤的檔案路徑 |
| `BatchConfigRaw` | YAML 原始 batch 參數（Duration 用毫秒表示） |
| `BatchConfig` | 轉換後的 batch 參數（Duration 已轉換） |
| `EndpointSet` | `HashMap<String, String>`：標籤 → URL |
| `PluginSlots` | 持有各階段 `SharedPlugin` 的容器，傳給 config watcher 進行熱替換 |
| `ChannelStats` | Atomic 計數器，追蹤 channel 積壓（條數與 bytes）— 已定義但目前未主動使用 |
| `Batch` | 累積原始行、bytes 總量、建立時間，提供 push / clear / elapsed 方法 |
| `FlushReason` | 觸發 flush 的原因（size / time / line_count / eof） |
| `ParseDiffTiming` | 差分量測結果：noop 時間、copy-in 時間、guest 時間、copy-out 時間 |
| `ParseStats` | Parse 階段的批次統計：批次數、行數、byte 數、WASM 記憶體峰值、grow 次數等 |
| `FormatStats` | Format 階段的批次統計 |
| `TransportStats` | Transport 階段的批次統計 |
| `FilterStats` | Filter 階段的批次統計 |

#### 主要函數

| 函數 | 作用 | 被誰呼叫 |
|------|------|----------|
| `load_app_config(path)` | 讀取並解析 YAML 設定檔 | `main()`、`spawn_config_watcher` 迴圈 |
| `spawn_config_watcher(path, shared, slots)` | 啟動背景 thread，週期性重載設定；若插件路徑或 mtime 改變則觸發熱替換 | `main()` |
| `BatchConfig::validate_and_describe()` | 驗證各 batch 參數合理性，印出每個欄位對應的程式碼路徑 | `main()` |
| `BatchConfig::print_config_table()` | 以表格形式印出每個參數值、用途、是否為預設值 | 內部使用 |
| `Batch::push(line)` | 新增一行，累積 bytes | `parse_dispatcher` |
| `Batch::clear()` | 清空並重置計時 | `parse_dispatcher`、`worker_flush_batch` |

---

### `src/app.rs`

WASM 運行時設定、插件編譯、輸入讀取器、熱替換機制。

#### WIT Bindgen 宣告

```
parser-plugin world  → ParserPlugin（call_parse, call_report_usage）
format-plugin world  → format_bindings::FormatPlugin（call_format, call_report_usage）
reduction-plugin world → reduction_bindings::ReductionPlugin（call_filter, call_report_usage）
transport-plugin world → transport_bindings::TransportPlugin（call_init, call_send, call_report_usage）
```

四個 world 共用 `pipeline-process` 介面的 Rust 型別（透過 `with` 區塊重用），避免 channel 傳遞時型別轉換。

#### 主要型別

| 型別 | 說明 |
|------|------|
| `MyLimiter` | 實作 `ResourceLimiter`，追蹤 WASM 線性記憶體用量，超過 `mem_limit_bytes` 時回傳 `Ok(false)` 讓 Wasmtime trap |
| `MyState` | WASM Store 的狀態：WASI context (`WasiCtx`)、資源表 (`ResourceTable`)、記憶體限制器 (`MyLimiter`)、HTTP context (`WasiHttpCtx`) |
| `PluginRuntime` | 已編譯的插件：engine + component + linker + version 號 |
| `SharedPlugin` | `Arc<RwLock<PluginRuntime>>`，跨 thread 共享、可熱替換的插件槽 |

#### 主要函數

| 函數 | 作用 | 被誰呼叫 |
|------|------|----------|
| `new_shared_runtime(wasm_path)` | 編譯 parse/filter/format 插件（同步 WASI），包裝成 SharedPlugin | `main()` |
| `new_shared_transport_runtime(wasm_path)` | 編譯 transport 插件（非同步 WASI + HTTP），包裝成 SharedPlugin | `main()` |
| `rebuild_shared_slot(slot, new_path, is_transport, label)` | 重新編譯插件，替換 slot 內容並遞增 version；失敗時保留舊插件 | `spawn_config_watcher` |
| `spawn_tcp_reader(host, port, tx)` | 在獨立 OS thread 跑 Tokio 事件迴圈，接受多條 TCP 連線，逐行送入 channel | `main()` |
| `spawn_tail_reader(path, tx)` | 在獨立 thread 追蹤檔案，polling 間隔 50ms，逐行送入 channel | `main()` |
| `spawn_stdin_reader(tx)` | 讀取 stdin 逐行送入 channel（定義在此但目前 main.rs 未呼叫） | 未接入 |
| `build_runtime(wasm_path)` | 建立 Cranelift 引擎、編譯 WASM component、建立同步 WASI linker | `new_shared_runtime` |
| `build_transport_runtime(wasm_path)` | 建立 Cranelift 引擎、編譯 WASM component、建立非同步 WASI + HTTP linker | `new_shared_transport_runtime` |
| `MyLimiter::new(mem_limit_bytes)` | 建立記憶體限制器 | 各 Store 建立時 |
| `MyLimiter::reset_batch_stats()` | 重置 grow_count / grow_total_delta_bytes / max_allocation（跨批次重用 Store 時呼叫）| `parse_worker` |
| `MyLimiter::memory_growing(...)` | Wasmtime 在 memory.grow 前呼叫；超過限制回傳 Ok(false) 觸發 trap | Wasmtime 內部呼叫 |

---

### `src/runtime.rs`

管線各階段的執行邏輯。

#### 內部結構

| 結構 | 說明 |
|------|------|
| `ParsedBatch` | parse→filter 傳遞的批次（entries + targettags + seq） |
| `FormattedBatch` | format→transport 傳遞的批次（格式化 bytes + targettags + seq + total_bytes） |
| `SendTask` | transport router→workers 的工作單元（url + lines） |
| `WorkBatch` | parse dispatcher→worker 的工作單元（lines + bytes + seq + reason + created_at） |
| `ParseInstance` | 單一 WASM parse 實例（id + store + plugin） |
| `ParsePool` | parse worker 持有的實例池（POOL_TARGET_SIZE=3 個備援） |

#### ParsePool 方法

| 方法 | 作用 |
|------|------|
| `new(shared, mem_limit_bytes, target_size)` | 建立並預熱池（create_one × target_size） |
| `create_one()` | 建立一個 ParseInstance（WasiCtx + MyLimiter + instantiate） |
| `check_reload()` | 偵測 version 差異（config watcher 已更新），清空重建整個池 |
| `replenish()` | 補滿至 target_size |
| `acquire()` | 從池頭取出實例；池空時建立緊急實例 |
| `release(inst)` | 歸還實例並 replenish |
| `release_without_reset(inst)` | 歸還但不 replenish（noop pool 用） |
| `discard_and_replenish(inst)` | 丟棄實例（OOM 後）並補新實例 |

#### 主要函數

| 函數 | 作用 | 被誰呼叫 |
|------|------|----------|
| `run_pipeline(...)` | 建立所有 channel，spawn 各階段 thread，等待結束並印出 summary | `main()` |
| `parse_loop(rx, tx, shared, parse_noop, cfg, mem)` | Parse 協調者：建立 dispatcher + N workers，彙總統計 | `run_pipeline` |
| `parse_dispatcher(rx, tx, cfg)` | 累積行，根據 size / time / line_count 觸發 flush，把 WorkBatch 送給 workers；熱重載 batch params（每次 recv 前讀 cfg） | `parse_loop` |
| `send_work_batch(batch, seq, reason, tx)` | 將 Batch 轉為 WorkBatch 送入 channel | `parse_dispatcher` |
| `parse_worker(id, rx, tx, shared, noop, mem)` | 持有 ParsePool，逐一處理 WorkBatch，呼叫 `worker_flush_batch` | `parse_loop` |
| `worker_flush_batch(...)` | 從池取實例、呼叫 `do_parse_batch`；OOM 時重試最多 3 次；超過上限寫 error.txt 並跳過 | `parse_worker` |
| `do_parse_batch(...)` | 實際呼叫 `call_parse()`；成功後呼叫 noop 差分量測；組裝 ParsedBatch；更新 stats | `worker_flush_batch` |
| `measure_noop_parse(pool, lines, guest_ns, elapsed)` | 用 noop parser 對同批次資料量測，估算 copy-in / copy-out 時間 | `do_parse_batch` |
| `write_error_file(header, lines)` | 將失敗批次寫入 `error.txt` | `worker_flush_batch` |
| `filter_loop(rx, tx, shared, mem)` | 單一長存活 Store，逐批次呼叫 `call_filter()`；插件錯誤時透傳原始批次（不丟資料） | `run_pipeline` |
| `format_loop(rx, tx, shared, mem, max_chunk)` | 逐批次依 max_format_chunk 分塊，每塊建新 Store 呼叫 `call_format()`；解析 4-byte LE length 幀格式 | `run_pipeline` |
| `transport_router(rx, tx_work, cfg, max_bytes)` | 依 targettag 中每個字元路由到對應 endpoint buffer；buffer 達 max_transport_bytes 或 timeout 時 flush 成 SendTask | `run_pipeline` |
| `build_transport_store(shared, mem)` | 非同步建立 transport WASM 實例（每個 SendTask 各自建立獨立實例） | `transport_worker` |
| `transport_worker(rx, shared, mem, max_chunk, id)` | 搶 SendTask，分塊呼叫 `call_init()` + `call_send()`；非同步 HTTP 傳輸 | `run_pipeline` |

---

### `src/output.rs`

所有終端輸出格式化函數（全部寫到 stderr）。

| 函數 | 輸出內容 |
|------|---------|
| `print_startup(cfg, budget, stages)` | 啟動時印出啟用的階段和主要參數 |
| `print_flush_header(seq, batch, reason)` | 每批次 flush 的標題行（批次號、原因、行數、大小、age） |
| `print_parse_batch(...)` | Parse 批次指標：In/Out 行數、WASM 記憶體、Time、吞吐量、grow 次數 |
| `print_filter_batch(...)` | Filter 批次指標：In/kept/dropped、記憶體、Time |
| `print_format_batch(...)` | Format 批次指標：In/Out 行數、記憶體、logic ms、copy-in ms |
| `print_transport_batch(...)` | Transport 批次指標（**定義但目前未被 runtime.rs 呼叫**） |
| `print_parse_aggregate(stats, workers, wall, errors)` | Parse 階段彙總：wall-clock 吞吐量、diff 量測平均值 |
| `print_pipeline_summary(p, fi, f, t, wall, mem)` | 全管線結束摘要表格（Parse / Filter / Format / Transport / E2E） |

---

## 3. 已實作功能詳述

### 3.1 熱更換 (Hot Swap)

`spawn_config_watcher()` 在背景 thread 每 `config_reload_secs`（預設 10 秒）重讀 `forwarder.yaml`。

觸發熱替換的條件（任一成立）：
- 插件路徑字串改變
- 同路徑檔案的 mtime 改變（即替換了同名 .wasm 檔）

替換流程：
1. 重新編譯 WASM component（`rebuild_shared_slot()`）
2. 寫入 `SharedPlugin`（`Arc<RwLock<PluginRuntime>>`），遞增 `version`
3. 管線各 thread 在下一批次前讀取 version，偵測到差異後重建本地 Store

**Batch 參數**（max_wait_ms 等）也在每次重載時同步更新，parse dispatcher 在每次 recv 前從 `Arc<RwLock<AppConfig>>` 讀取最新值，因此 batch 參數無需重啟即可生效。

失敗保護：若新 WASM 編譯失敗，保留舊插件繼續運作，並印出錯誤訊息到 stderr。

### 3.2 路由 (Routing)

Parse 插件為每條日誌回傳一個 `targettag` 字串（定義在 WIT `parsed-entry.targettag`）。

Transport router 的分流邏輯：
- 將 `targettag` 中的**每個字元**當作 endpoint key
- 對應到 `endpoint` map 查找 URL
- **Fan-out**：同一條日誌若 targettag = "AB" → clone 後同時推入 buffer_A 和 buffer_B
- buffer 達到 `max_transport_bytes` 或等待超過 `max_wait_ms` → flush 成 SendTask 送給 workers

`endpoint` map 在 `forwarder.yaml` 中定義（支援多個字母鍵對應多個後端 URL）。

### 3.3 輸入模式

| 模式 | 機制 | 說明 |
|------|------|------|
| `tcp` | Tokio async TcpListener | 支援多條並發連線，每條連線獨立 task 讀行 |
| `tail` | Polling 50ms | 開啟時 seek 到 EOF，只讀新增內容；檔案不存在則每秒重試 |
| `stdin` | 逐行讀取 | 已在 app.rs 實作，但 main.rs 未接入此模式選擇 |

### 3.4 管線階段控制

透過 `stages.*` 可以選擇性啟用各階段：
- `stages.filter: false` → filter thread 改為橋接器直接透傳 ParsedBatch
- `stages.format: false` → format thread 改為 drain（丟棄），tx_formatted 被 drop
- `stages.transport: false` → transport 改為 drain

### 3.5 Parse 實例池 (ParsePool) 與 OOM 重試

每個 parse worker 持有一個 `ParsePool`（預熱 3 個備援實例）。

目前採用**用後即丟**策略（`discard_and_replenish`），每批次用完後補新實例，避免 GC 語言（Go）在重用實例時因指標未斷開而累積無法回收的記憶體。

OOM 重試流程（最多 3 次，共 4 次嘗試）：
1. 取實例、執行 parse
2. Wasmtime 觸發 `memory_growing()` 回傳 `Ok(false)` → trap
3. 捕獲 `Err(e)` → discard 實例、補新實例
4. 重試直到成功或超過上限
5. 超過上限：將失敗批次寫入 `error.txt`，跳過此批次，繼續處理下一批

### 3.6 Diff 量測（no-op parser）

若設定 `plugins.parse_noop`，parse worker 會在主 parser 完成後，用 noop parser 對**相同資料**再跑一次，量測：
- `copy_in_ns`：估算 host→WASM 資料複製時間
- `guest_ns`：主 parser 的 guest 邏輯時間（來自 `report_usage()`）
- `copy_out_ns`：估算 WASM→host 資料複製時間

### 3.7 Format 分塊呼叫

Format loop 將每個 ParsedBatch 依 `max_format_chunk` 分塊，每塊**建立全新 Store** 呼叫 `call_format()`，目的是讓 TinyGo GC 能在批次結束後回收中間字串緩衝區，避免 WASM OOM。

Format 插件輸出格式：`[4-byte LE length][data]...` 幀序列，host 端解析並重組為 `Vec<Vec<u8>>`。

### 3.8 Transport 非同步 HTTP（多 Worker）

Transport 插件使用非同步 WASI（`add_to_linker_async`）+ WASI HTTP（`add_only_http_to_linker_async`），繞過同步模式 4096 B 寫入限制。

每個 SendTask 建立獨立 WASM 實例（stateless），避免跨請求狀態污染。N 個 workers 共用 `Arc<Mutex<Receiver<SendTask>>>` 搶任務，允許同時進行多個 HTTP POST。

---

## 4. YAML 可調整的參數

### 頂層參數

| 參數 | 預設值 | 說明 |
|------|--------|------|
| `config_reload_secs` | `10` | Config watcher 重讀設定檔的間隔（秒） |

### `plugins.*`（所有為路徑，可相對或絕對）

| 參數 | 必填 | 說明 |
|------|------|------|
| `plugins.parse` | 是 | Parse WASM 插件路徑 |
| `plugins.parse_noop` | 否 | No-op parser 路徑（用於 diff 量測，null 表示停用） |
| `plugins.filter` | 是（即使 stages.filter=false 也需設路徑） | Filter WASM 插件路徑 |
| `plugins.format` | 同上 | Format WASM 插件路徑 |
| `plugins.transport` | 同上 | Transport WASM 插件路徑 |

### `stages.*`

| 參數 | 類型 | 預設 | 說明 |
|------|------|------|------|
| `stages.filter` | bool | false | 是否啟用 filter 階段 |
| `stages.format` | bool | false | 是否啟用 format 階段 |
| `stages.transport` | bool | false | 是否啟用 transport 階段 |

### `input.*`

| 參數 | 說明 |
|------|------|
| `input.mode` | `"tcp"` 或 `"tail"` |
| `input.tcp.host` | TCP 監聽位址（mode=tcp 時必填） |
| `input.tcp.port` | TCP 監聽 port（mode=tcp 時必填） |
| `input.tail.path` | 追蹤的日誌檔路徑（mode=tail 時必填） |

### `batch.*`（所有參數支援熱重載，但 transport_workers 和 mem_limit_mb 只在啟動時生效）

| 參數 | 預設值 | 影響的程式碼路徑 | 說明 |
|------|--------|-----------------|------|
| `mem_limit_mb` | 256 | `MyLimiter::new()` | 每個 WASM Store 的線性記憶體上限（MB） |
| `safe_data_ratio` | 0.5 | `parse_dispatcher` size trigger | batch bytes 超過 `mem_limit × ratio` 時提前 flush |
| `max_wait_ms` | 5000 | `parse_dispatcher` recv_timeout | 超過此時間無新行則 time-based flush（ms） |
| `max_batch_lines` | 50000 | `parse_dispatcher` line_count trigger | 批次達到此行數時 flush |
| `channel_capacity` | 150000 | `main()` sync_channel | stdin→parse channel 容量（parse→filter 等硬編碼為 20000） |
| `max_format_chunk` | 50000 | `format_loop` chunks() | format 插件每次呼叫的最大 entry 數 |
| `transport_endpoint` | None | transport_worker init | 已由 endpoint map 取代，保留相容性 |
| `max_transport_bytes` | 131072 | `transport_router` buffer 上限 + `transport_worker` 分塊 | 每次 send() 傳送的最大 bytes（一次 HTTP POST）|
| `transport_workers` | 5 | `run_pipeline` | 平行 transport worker 數量（啟動時決定） |

### `endpoint.*`

| 格式 | 說明 |
|------|------|
| `endpoint.<key>: <url>` | key 為單一字元標籤（如 A、B、C），url 為 HTTP endpoint |

---

## 5. 各階段並行性詳解

| 階段 | 並行方式 | Thread 數 | 備註 |
|------|---------|-----------|------|
| Parse dispatcher | 串行 | 1 | 只做 batching，不碰 WASM |
| Parse worker | 串行（可擴展） | `PARSE_WORKERS = 1`（常數） | 架構已支援 N workers 搶 WorkBatch channel；增加此常數即可擴展 |
| Filter | 串行 | 1 | 長存活 Store，逐批次呼叫 |
| Format | 串行 | 1 | 每 chunk 建新 Store（Store 建立成本在 chunk 迴圈內，非 thread 並行） |
| Transport router | 串行 | 1 | 純 routing 邏輯，不碰 WASM |
| Transport worker | **並行** | `transport_workers`（預設 5） | 共用 Mutex<Receiver>，各自獨立 WASM 實例、Tokio runtime |

**階段間並行**：各階段 thread 同時在跑，以 `sync_channel` 背壓控制速率。Format 在處理批次 N+1 時，Transport 同時在發送批次 N。

---

## 6. 錯誤處理機制

### Parse OOM 重試

- 最多 3 次重試（共 4 次嘗試）
- 每次重試前 `discard_and_replenish()`：丟掉損壞實例、補充新實例
- 超過重試上限：`write_error_file("以下這批是OOM", &batch.lines)` → 寫入 `error.txt`，跳過此批次
- **只有 WASM trap（`Err(e)`）才重試**；插件邏輯錯誤（`Ok(Err(plugin_error))`）不重試，直接跳過

### Filter 錯誤

- 插件回傳 `Ok(Err(plugin_error))` 或 trap（`Err(e)`）：**透傳原始 ParsedBatch**，不丟失資料
- 印出錯誤訊息到 stderr 後繼續處理下一批

### Format 錯誤

- 插件 error 或 OOM：`batch_ok = false`，整批次跳過（資料丟失）
- 不寫 error file

### Transport 錯誤

- `call_init()` 失敗：`continue`，跳過此 SendTask
- `call_send()` 失敗：`batch_ok = false`，後續 chunk 跳過，此 task 資料丟失
- **無重試機制**：`TransportConfig.retry = None`；WIT 介面有定義 `retry-config` 但目前 host 設為 None

---

## 7. Plugin 異常行為的後果

### 7.1 Infinite Loop（無限迴圈）

目前**沒有** CPU 燃油（fuel）或 timeout 機制。

若插件進入無限迴圈：
1. 該階段 thread 永久阻塞在 `call_parse()` / `call_filter()` / `call_format()` / `call_send()`
2. 下游 channel 無新資料，下游 thread 阻塞在 `rx.recv()`
3. 上游 channel 持續被填滿，觸發 `sync_channel` 背壓（`tx.send()` 阻塞）
4. 最終 input channel 也填滿，input reader 阻塞
5. **整個管線靜止**：不崩潰但不再處理任何日誌，直到手動 kill 程序

### 7.2 記憶體過量

`MyLimiter::memory_growing()` 在 WASM 嘗試 `memory.grow` 時被呼叫：
- 若 `desired > mem_limit_bytes`，回傳 `Ok(false)` → Wasmtime **立即 trap** 插件
- Host 捕獲此 `Err(e)` 作為 OOM，進入重試流程（parse）或跳過批次（format/transport）
- 宿主程序本身**不受影響**，繼續運作

### 7.3 CPU 過量

**無保護機制**（未設定 Wasmtime fuel）。CPU 密集型無限迴圈的後果同 7.1。

### 7.4 Plugin Panic / 越界存取

Wasmtime 沙箱捕獲 WASM trap（如 unreachable 指令、越界記憶體存取）並回傳 `Err`。
- Host 的 `Err(e)` 處理路徑（各階段均有）印出錯誤訊息，繼續處理下一批次
- **不影響宿主程序**，符合提案中的故障隔離設計目標

### 7.5 未授予能力（如 parser 嘗試開 socket）

- Parse / filter / format 使用 `build_runtime()`：linker 只加入同步 WASI CLI imports，**不含 HTTP 能力**
- 插件呼叫任何網路相關函式 → Wasmtime linker 找不到函式 → `instantiate()` 即失敗
- Transport 使用 `build_transport_runtime()`：額外授予 HTTP 能力（`add_only_http_to_linker_async()`），其他能力仍受限

---

## 8. WIT 介面定義（v0.3.0）

### 共用型別（`pipeline-process` interface）

| 型別 | 說明 |
|------|------|
| `parsed-entry` | Parser 回傳值（無 id）：timestamp, level, message, tags, targettag |
| `log-entry` | Host 分配 id 後的完整條目：id + parsed-entry 的欄位（不含 targettag） |
| `log-level` | enum：debug / info / warn / error / crit / alert / emerg（7 級） |
| `parse-error` | variant：invalid-format / unsupported-version / corrupted-data |
| `plugin-error` | variant：invalid-input / processing-failed / config-error / unknown-id / internal-error |
| `filter-result` | `{ id: u64, keep: bool }` |
| `enrich-result` | `{ id: u64, additional-tags: list<tuple<string,string>> }`（WIT 已定義，但 enrich 階段未實作） |
| `route-result` | `{ id: u64, urls: list<string> }`（WIT 已定義，但 route-plugin world 未實作） |

### Transport 專用型別（`transport-types` interface）

`auth-method`（none / bearer-token / basic-auth / api-key）、`retry-config`（max-retries / initial-backoff-ms / max-backoff-ms）、`tls-config`（verify-peer / ca-pem / mTLS cert）、`transport-config`

### Plugin Worlds 介面

| World | 輸出函數 |
|-------|---------|
| `parser-plugin` | `parse(list<string>) → result<list<parsed-entry>, parse-error>` + `report-usage() → u64` |
| `reduction-plugin` | `filter(list<log-entry>) → result<list<filter-result>, plugin-error>` + `report-usage()` |
| `format-plugin` | `format(list<log-entry>) → result<list<u8>, plugin-error>` + `report-usage()` |
| `transport-plugin` | `init(transport-config) → result<_, plugin-error>` + `send(list<list<u8>>) → result<_, plugin-error>` + `report-usage()` |

---

## 9. 已有的測試插件

| 語言 | 階段 | 路徑 |
|------|------|------|
| C | parse | `test-plugins/c-plugin/parse/parser_c_json.wasm` |
| C | format | `test-plugins/c-plugin/format/format_json-flat.wasm` |
| C# | parse | `test-plugins/csharp-plugin/parse/` |
| C# | filter | `test-plugins/csharp-plugin/filter/filter_csharp.wasm` |
| Go | parse | `test-plugins/go-plugin/parse/`（json / sys / fmt 三種格式） |
| Go | format | `test-plugins/go-plugin/format/` |
| Go | parse_noop | `test-plugins/go-plugin/noop-parser/`（差分量測用） |
| Rust | transport | `test-plugins/rust-plugin/transport/` |

---

## 10. 工具與測試基礎設施

### Log 產生器（`tools/gen/main.go`）

輸出格式：json-simple / json-complex / json-mixed / invalid / logfmt-simple / logfmt-complex / logfmt-mixed / syslog-simple / syslog-complex / syslog-mixed

流量模式：`flat`（固定速率）/ `wave`（正弦波，長期平均精確等於 rate）/ `bursty`（正弦波 + 隨機突發尖峰）

### Native Go 基準測試（`benchmark/go/main.go`）

純 Go 實作 JSON / logfmt / syslog 解析器，用於量測「無 WASM 的天花板吞吐量」。支援 chunk 分段計時、repeat 重複測試、記憶體分配統計。

### Sink Server（`test/tools/sink_server.py`）

接收 transport 送出的 HTTP POST，用於驗證 transport 端到端。

### 比較測試（`test/compare/`）

已完成 Fluent Bit Lua 插件和 Fluent Bit WASM filter 的實驗，資料存為 CSV（loop / cpu / mem / io / sink 各項指標）。

---

## 11. 已知設計缺陷與注意事項

1. **`channel_capacity` 只控制 stdin→parse channel**；parse→filter、filter→format、format→transport router 三個 channel 容量硬編碼為 20,000（`runtime.rs:224-227`），不受 YAML 控制。

2. **`PARSE_WORKERS = 1` 硬編碼**：架構已支援多 parse worker，只需修改 `runtime.rs:54` 的常數即可擴展，但目前只有 1 個。

3. **無 CPU fuel / timeout**：Plugin 無限迴圈會讓整個管線靜止，無自動恢復機制。

4. **Transport 無重試**：`TransportConfig.retry = None`，HTTP 失敗即丟棄此 SendTask 資料。

5. **Format 錯誤不寫 error file**：Format 錯誤批次直接跳過，資料無法回收。

6. **`print_transport_batch()` 定義但未呼叫**：`output.rs:160` 定義了此函數，但 `transport_worker` 使用個別 eprintln!，未呼叫此函數。

7. **stdin input 未接入**：`spawn_stdin_reader()` 已在 `app.rs:289` 實作，但 `main.rs` 的 `InputMode` match 沒有 stdin 分支。

8. **`mem_limit_mb` 和 `transport_workers` 熱重載無效**：這兩個值在 `run_pipeline()` 啟動時一次性讀取，config watcher 雖然更新 `AppConfig`，但 runtime 中的本地 copy 不會跟著更新。
