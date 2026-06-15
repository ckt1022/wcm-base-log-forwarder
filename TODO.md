# WCM Log Forwarder — 進度追蹤

> 依據 `project_proposal.pdf` 整理，對照目前原始碼（2026-06-15）

---

## 已完成 (Done)

### 核心框架（第 4 節：實作內容）

- [x] **Rust + Wasmtime 宿主實作**：以 Rust 語言搭配 Wasmtime 實作 Host，協調各 Component 間的資料傳遞
- [x] **WIT 介面定義**：定義 `parser-plugin`、`reduction-plugin`（filter）、`format-plugin`、`transport-plugin` 四個 world，版本 v0.3.0
- [x] **四階段管線**：Parse → Filter → Format → Transport，各階段獨立 thread 並以 `sync_channel` 背壓串聯
- [x] **階段可選啟用**：透過 `stages.filter/format/transport` YAML 設定，停用時自動橋接不阻塞上游

### 插件系統

- [x] **WCM 插件熱更換（Hot Swap）**：`spawn_config_watcher()` 週期性監測 WASM 路徑與 mtime，變更時自動重新編譯並替換插件槽（`Arc<RwLock<PluginRuntime>>`），pipeline 在下一批次自動切換，不需重啟
- [x] **Parse 實例池（ParsePool）**：預熱 3 個備援 WASM 實例，支援用後即丟策略，解決 GC 語言（Go）重用實例 OOM 問題
- [x] **OOM 重試機制**：Parse 階段最多 3 次重試；超過上限寫入 `error.txt` 並跳過此批次
- [x] **記憶體限制（MyLimiter）**：`ResourceLimiter` 在 `memory.grow` 時強制上限，超過即 trap 插件，宿主不受影響

### 輸入

- [x] **TCP 輸入模式**：Tokio async TcpListener，支援多條並發連線
- [x] **File Tail 輸入模式**：Polling 追蹤新增日誌行（50ms 間隔）；檔案不存在時每秒重試

### 路由（Routing）

- [x] **ParsedEntry.targettag 路由欄位**：Parser 插件回傳 `targettag`，host 以每個字元映射 endpoint key
- [x] **多端點 Fan-out**：單條日誌可複製到多個 endpoint（targettag="AB" → buffer_A 和 buffer_B）
- [x] **YAML endpoint map**：`endpoint.<key>: <url>` 動態設定，支援熱重載

### Transport

- [x] **非同步 HTTP Transport**：使用 WASI async + WASI HTTP，繞過 sync 模式 4096B 限制
- [x] **N 個 Transport Workers**：共用工作佇列，平行進行多個 HTTP POST
- [x] **Transport Router**：依 targettag 分流，buffer 達 `max_transport_bytes` 或 timeout 後 flush

### 量測與觀測

- [x] **Diff 量測（no-op parser）**：可選 `parse_noop` 插件，估算 copy-in / guest 邏輯 / copy-out 各自的時間
- [x] **WASM 記憶體追蹤**：每批次記錄峰值 `wasm_mem_peak`、`grow_count`、`grow_delta_bytes`
- [x] **`report-usage()` 介面**：插件回報內部邏輯執行時間（ns）
- [x] **Pipeline Summary 輸出**：結束時印出 Parse / Filter / Format / Transport / E2E 完整彙總表格

### 配置

- [x] **YAML 設定檔**：所有 batch 參數、插件路徑、輸入模式、endpoint map 均可由 YAML 控制
- [x] **參數驗證（`validate_and_describe`）**：啟動時驗證各參數合理性，印出對應程式碼路徑說明

### 比較實驗（第 5 節：比較指標與對象）

- [x] **Fluent Bit Lua 插件實驗**：已完成（`test/compare/fluentbit/lua/`），資料有 loop / cpu / mem / io CSV
- [x] **Fluent Bit WASM filter 實驗**：已完成（`test/compare/fluentbit/wasm_filter/`），資料有各項 CSV
- [x] **Native Go 解析基準**：`benchmark/go/main.go` 提供 JSON / logfmt / syslog 三種格式的純 Go 解析吞吐量
- [x] **Log 產生器**：`tools/gen/main.go` 支援多種格式、flat/wave/bursty 流量模式

### 語言跨語言驗證

- [x] **C 插件**：Parse（JSON）、Format（JSON flat）
- [x] **C# 插件**：Parse、Filter
- [x] **Go 插件**：Parse（JSON/sys/fmt）、Format、no-op parser
- [x] **Rust 插件**：Transport

---

## 未完成 / 待辦 (TODO)

### 效能量測（第 3.1 節：WCM 效能成本量化）

- [ ] **原生 Rust pipeline 基準**（最重要的對照組）：實作同程序內的 Rust 函式呼叫 pipeline（無 WASM），量測「無沙箱成本的天花板」吞吐量
- [ ] **裸 core-wasm + C-ABI 對照**：以 offset+length 的 C-ABI 傳遞資料的 core-wasm 版本，隔離出 Component Model / Canonical ABI 的「額外」成本
- [ ] **延遲分佈量測**：per-batch p50 / p95 / p99 / p999 尾延遲（目前只有平均值）
- [ ] **CPU cycles/record 量測**：在固定輸入速率下量測 cycles/record（比 CPU% 更可比較）
- [ ] **實例化時間基準（冷啟動）**：量測 ms/component 的冷啟動時間，評估熱抽換與水平擴充的成本
- [ ] **記憶體隨階段數成長曲線**：多實例 / 多階段下 RSS 與 linear memory 的成長曲線
- [ ] **複製次數 / bytes 插樁計數**：在 host 內部加入計數器，量化 Canonical ABI 邊界複製量

### 效能比較（第 3.2 節：與市面工具比較）

- [ ] **與 Vector VRL 比較**：量測 Vector 使用 VRL 做相同日誌處理的吞吐量與延遲
- [ ] **subprocess + pipe/IPC 對照**：量測以「fork 獨立程序 + pipe」達成隔離的成本，回答「為何選 WASM 而非 fork」

### 安全性驗證（第 3.3 節 / 第 5.1 節）

- [ ] **能力阻擋測試（parser 嘗試開 socket）**：製作一個嘗試呼叫網路函式的惡意 parse 插件，驗證 linker 是否在 `instantiate()` 或 runtime 正確阻擋
- [ ] **故障隔離測試（plugin panic）**：製作會 trap / panic 的插件，驗證宿主程序不崩潰、其他批次繼續處理
- [ ] **越界存取測試**：製作越界記憶體存取的插件，驗證 Wasmtime 沙箱攔截
- [ ] **記憶體超限測試**：驗證 `MyLimiter` 阻擋機制在高負載下的行為
- [ ] **不可信插件整合測試**：整合上述案例，系統化記錄「攻擊被擋下」的案例研究

### 功能補全

- [ ] **CPU fuel / timeout 機制**：加入 Wasmtime fuel 或 thread timeout，防止插件無限迴圈讓管線靜止
- [ ] **Transport 重試機制（HTTP backoff）**：`TransportConfig.retry`（max-retries / backoff）的 HTTP 層級重試尚未實作；已實作 WASM 呼叫層級超時偵測與 2 次重試
- [x] **Format / Filter 錯誤寫入 error file**：format 與 filter 階段失敗批次現在參考 parse 的 `write_error_file()` 寫入 error.txt
- [ ] **stdin 輸入模式接入**：`spawn_stdin_reader()` 已實作，需在 `main.rs` 的 `InputMode` match 加入 stdin 分支


### 語言跨語言補全

- [ ] **C++ 插件**：提案提及 C++ 作為語言之一，目前未有 C++ 實作的插件

### 效能調優（README 工作日記提到）

- [ ] **提高 Transport 吞吐量**：目前每個 SendTask 各自建立新 WASM 實例，有初始化開銷；考慮 transport 也採用 instance pool
- [ ] **結構化傳送 vs Flat 傳送時間比較**：量測 WCM Canonical ABI（結構化）與 core-wasm C-ABI（flat offset+length）的傳送時間差異

### 工程品質

- [x] **parse→filter / filter→format / format→transport channel 容量加入 YAML 控制**：新增 `batch.pipeline_channel_capacity` 欄位，預設 20,000
- [ ] **`PARSE_WORKERS` 加入 YAML 控制**：目前硬編碼為 1（`runtime.rs:54`）
- [ ] **`print_transport_batch()` 接入**：`output.rs:160` 已定義但 `transport_worker` 未呼叫
- [ ] **`mem_limit_mb` 和 `transport_workers` 熱重載支援**：目前這兩個值在 `run_pipeline()` 啟動時一次性讀取，無法熱重載

---

## 進度摘要

| 類別 | 完成 | 待辦 |
|------|------|------|
| 核心框架與管線 | ✅ | — |
| 插件熱更換 | ✅ | — |
| 輸入模式 | TCP + Tail ✅ | stdin 未接入 |
| 路由 | ✅ | — |
| Transport 非同步 HTTP | ✅ | 重試機制未實作 |
| 差分量測 | ✅ | — |
| 效能基準（原生 Rust、core-wasm + C-ABI） | ❌ | 最關鍵的對照組未做 |
| 延遲分佈（p50/p95/p99）| ❌ | 未實作 |
| CPU cycles/record | ❌ | 未實作 |
| Vector VRL 比較 | ❌ | 未做 |
| 安全性驗證 | ❌ | 所有案例均未完成 |
| CPU fuel / timeout | ❌ | 無限迴圈無保護 |
| Enrich 階段 | ❌ | WIT 已定義，host 未實作 |
| C++ 插件 | ❌ | 未實作 |
