# Latency Benchmark — 實驗設定文件

> 更新日期：2026-07-04

---

## 1. 實驗目標

量測 WCM Log Forwarder 的 **E2E 延遲分佈（p50 / p95 / p99 / p999）**，
並建立可與 Fluent Bit、Vector 等工具公平比較的基準。

---

## 2. 延遲定義

```
latency = server_receive_time − log_ts（log 產生時間戳）
```

- **起點**：log generator 寫入每筆 log 時嵌入的 `ts` 欄位（RFC3339Nano，微秒精度）
- **終點**：HTTP server 接收到包含該筆 log 的 POST request 的時間
- **粒度**：batch 層級——同一批次內所有行共用相同 `server_receive_time`；
  批次最前面的行延遲最高，最後面的行延遲最低，反映的是完整管線等待成本（正確行為）
- **時鐘**：generator 與 server 同機器，無 clock skew 問題

### ⚠️ 前提（待辦）

- Transport stage 必須啟用，server 才能收到資料
- **`tools/test/server.py` 需要修改**，加入：
  - 解析每筆 JSON 的 `ts` 欄位
  - 記錄 request 抵達的 wall-clock time
  - 輸出每筆的 `(receive_time − ts)` latency，寫入 CSV

---

## 3. 固定變因

### 3.1 輸入格式

| 項目 | 設定值 | 說明 |
|------|--------|------|
| Log format | `json-simple` | 每行固定 **172 bytes**（`_p` padding 欄位補齊，已實作）|
| Traffic pattern | `flat` | 固定速率，不用 wave/bursty（避免突發排隊汙染 p99） |
| Generator flush | `-flush-ms 10` | 平滑 TCP 輸入，避免 100ms burst |
| Random seed | `-seed 42` | 固定 seed，確保各工具收到相同內容分佈 |

### 3.2 輸入模式

| 項目 | 設定值 | 說明 |
|------|--------|------|
| 模式 | **TCP** | 排除 tail polling 50ms 不確定性地板 |
| 端口 | `5140` | WCM TCP input |
| 比較工具 | 各工具同樣從 TCP 讀入，或寫到同一個 log file 再 tail | 需確認 |

### 3.3 批次參數（**所有比較工具必須對齊**）

| 參數 | WCM 設定 | Fluent Bit 對應 | Vector 對應 |
|------|----------|-----------------|-------------|
| Batch timeout | `max_wait_ms: 100` | `Flush 0.1` | `batch.timeout_secs: 0.1` |
| Batch max lines | `max_batch_lines: 1000` | chunk line limit | `batch.max_events: 1000` |
| Transport chunk | `max_transport_bytes: 131072` | `storage.Chunk_Size_Limit 128k` | `batch.max_bytes: 131072` |

### 3.4 速率策略

**不固定絕對 lines/s，改用各工具飽和點的比例。**

實驗流程：
1. 先跑爬坡測試找出 WCM 的飽和吞吐量（p99 開始暴增的轉折點）
2. 對各比較工具做相同的爬坡
3. 延遲實驗在 **30% / 50% / 80%** 飽和點各跑一次

初始低流量測試（第一步）：**500 lines/s**
- 100ms window × 500/s = ~50 lines/batch，遠低於 max_batch_lines
- 系統接近 idle，p50 ≈ 50ms（純 batch timeout cost）

### 3.5 環境與系統

| 項目 | 設定 |
|------|------|
| Build mode | `cargo build --release`（非 debug！）；`FORWARDER_BIN` 路徑改為 `target/release/`；`test.sh` 目前使用 debug build，需同步修改 |
| WASM 插件 | 全部 C 語言，使用 `-O2` + `wasm-opt -O2` |
| config_reload_secs | `3600`（避免 watcher 干擾量測） |
| parse_noop | **停用**（`parse_noop: ~`），否則每批多跑一次 WASM call |
| 系統背景程序 | 量測期間關閉無關程序 |
| Warm-up | 前 15 秒資料丟棄，不計入分佈 |
| 量測窗口 | warm-up 後取 **60 秒** 穩態資料 |

### 3.6 Transport sink

- 使用 `tools/test/server.py`（localhost），不使用外部 endpoint
- 單一 endpoint，targettag = `A`（不使用 fan-out）

---

## 4. WCM 設定值（forwarder.yaml）

```yaml
plugins:
  parse: "test-plugins/c-plugin/parse/parser_c_json.wasm"
  parse_noop: ~                          # 停用 noop diff

  filter: "test-plugins/c-plugin/filter/filter_c.wasm"          # ← 待辦：C filter plugin 尚未實作
  format: "test-plugins/c-plugin/format/format_json-flat.wasm"
  transport: "test-plugins/c-plugin/transport/transport_c.wasm"  # ← 待辦：C transport plugin 尚未實作

stages:
  filter: true     # 全開
  format: true
  transport: true

input:
  mode: tcp
  tcp:
    host: "0.0.0.0"
    port: 5140

batch:
  mem_limit_mb: 256
  safe_data_ratio: 0.5
  max_wait_ms: 100           # ← 關鍵：從 1000 改為 100
  max_batch_lines: 1000      # ← 關鍵：從 10000 改為 1000
  channel_capacity: 150000
  max_format_chunk: 50000
  max_transport_bytes: 131072
  transport_workers: 1       # 低流量測試用單 worker
  pipeline_channel_capacity: 20000
  stage_timeout_ms: 5000

endpoint:
  A: "http://127.0.0.1:8080/ingest"

config_reload_secs: 3600     # ← 避免熱重載干擾
```

---

## 5. 執行緒說明（比較公平性）

### 低流量（目前階段）

WCM 在 500 lines/s 下實際活躍 thread：
- Input reader: 1 thread（block on TCP read）
- Parse dispatcher: 1 thread（block on recv_timeout 大部分時間）
- Parse worker: 1 thread（PARSE_WORKERS=1，hardcoded in runtime.rs:55）

**低流量時各 thread 幾乎全在等待，多執行緒帶來的吞吐優勢未被啟動。**
延遲差異主要由批次 timeout 與 WASM 處理時間決定，非執行緒架構。

### 高吞吐比較（未來）

不同工具的執行緒模型：
- WCM: 每 stage 獨立 thread（pipeline 並行）
- Fluent Bit: N worker threads（預設 1），處理全 filter/output 鏈
- Vector: Tokio async（預設全核 thread pool）

**高吞吐比較時的公平方式**：
- 固定各工具 worker=1，比較 throughput
- 或：比較「相同 CPU% 下的 throughput」（lines/s 除以 CPU 使用率）

---

## 6. 比較場景

### 主場景：Full pipeline（全 stage 開啟）

目標：量測完整處理鏈的 E2E 延遲  
stages: `filter=true, format=true, transport=true`  
對應比較：Fluent Bit (input → filter → output) / Vector (source → transform → sink)

### 次場景（可選）：Parse-only（WCM 架構成本隔離）

目標：單獨量測 WASM ABI + Cranelift 基礎成本，排除 filter/format/transport 干擾  
stages: `filter=false, format=false, transport=false`  
對應比較：Fluent Bit Lua filter only / Vector VRL transform only

---

## 7. Log Generator 指令

### 初始低流量測試（500 lines/s）

```bash
go run tools/gen/main.go \
  -rate 500 \
  -duration 90 \
  -traffic flat \
  -mode json-simple \
  -flush-ms 10 \
  -seed 42 \
  | nc 127.0.0.1 5140
```

duration=90：前 15s warm-up + 60s 量測 + 15s 緩衝

### 飽和點爬坡測試

```bash
for rate in 1000 2000 5000 10000 20000 50000; do
  go run tools/gen/main.go -rate $rate -duration 30 -traffic flat \
    -mode json-simple -flush-ms 10 -seed 42 | nc 127.0.0.1 5140
  sleep 5
done
```

---

## 8. 待辦事項（實作前需完成）

- [ ] **撰寫 C filter plugin**（`test-plugins/c-plugin/filter/`）：實作 level-based filter，保留 `level >= Warn`（丟棄 debug / info，保留約 33%）；邏輯參考 `csharp-plugin/filter/Filters.cs`，對照 WIT 介面 `reduction-plugin` world
- [ ] **撰寫 C transport plugin**（`test-plugins/c-plugin/transport/`）：使用 WASI HTTP C bindings（`wasi:http/outgoing-handler`）實作 HTTP POST；注意此為 async Component，編譯需搭配 `wasm-tools component` 和 async lift/lower；複雜度高於 filter，建議先完成 filter 再處理
- [x] **修改 `tools/gen/main.go`**：`json-simple` 每行固定 172 bytes，在 JSON 結尾加入 `"_p":"..."` padding 欄位補齊
- [ ] **修改 `test.sh`**：`cargo build` 改為 `cargo build --release`；`FORWARDER_BIN` 路徑改為 `target/release/wcm-base-log-forwarder`
- [ ] **修改 `tools/test/server.py`**：解析每行 `ts` 欄位，記錄 receive timestamp，輸出 latency CSV
- [ ] **確認 C 插件是否經過 wasm-opt**：檢查 Makefile 或 build script
- [ ] **確認 transport WASM 路徑**：forwarder.yaml 中的 transport 路徑需要指向已編譯的 `.wasm`
- [ ] **讓 PARSE_WORKERS 可由 YAML 控制**（目前 hardcoded runtime.rs:55）
- [ ] **Fluent Bit / Vector 的批次設定對齊**：確認 Fluent Bit `Flush` 和 Vector `batch.timeout_secs` 設為 0.1s
- [ ] **output.rs per-batch eprintln 靜音**：高吞吐測試時，`print_parse_batch` 等函式的 stderr 輸出會增加 p99，需要提供關閉選項

---

## 9. 已排除的變因（不需控制原因）

| 排除項目 | 原因 |
|----------|------|
| json-mixed / json-complex | 線長 variance 過大，保留為次要測試 |
| wave / bursty traffic | 突發會造成排隊延遲，與處理延遲混淆 |
| parse_noop（diff measurement） | 啟用時每批多跑一次 WASM，使吞吐量減半 |
| Fan-out（多 endpoint） | 造成額外 clone 分配，不是通用基準 |
| Go / C# 插件 | GC jitter 汙染延遲分佈，用於語言相容性驗證，非效能基準 |
