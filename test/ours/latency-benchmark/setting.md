# Latency Benchmark — 實驗設定文件

> 更新日期：2026-07-05

---

## 1. 實驗目標

量測 WCM Log Forwarder 的 **E2E 延遲分佈（p50 / p95 / p99 / max）**，
並建立可與 Fluent Bit、Vector 等工具公平比較的基準。

---

## 2. 延遲定義

```
latency = server_receive_time − log_ts（log 產生時間戳）
```

- **起點**：log generator 寫入每筆 log 時嵌入的 `ts` 欄位（RFC3339Nano）
- **終點**：HTTP server（`tools/test/server.go`）接收到包含該筆 log 的 POST request 的 Unix nanosecond 時間
- **精度**：generator 與 server 均使用 `time.Now().UnixNano()` / `time.RFC3339Nano`，無 float64 精度損失
- **粒度**：batch 層級——同一批次內所有行共用相同 `server_receive_time`；
  批次最前面的行延遲最高，最後面的行延遲最低，反映的是完整管線等待成本（正確行為）
- **時鐘**：generator 與 server 同機器，無 clock skew 問題

---

## 3. 固定變因

### 3.1 輸入格式

| 項目 | 設定值 | 說明 |
|------|--------|------|
| Log format | `json-fixed5` | 5 條固定內容循環（`_slot=1~5`），排除內容差異對延遲的影響 |
| Traffic pattern | `flat` | 固定速率，不用 wave/bursty（避免突發排隊汙染 p99） |
| 傳送方式 | `-output tcp -send-unit line` | 每條 log 產生後立即送出（直連 TCP，無 nc 中介緩衝） |

> **json-fixed5 設計說明**：5 種模板（info/warn/error/debug/info）循環輸出，`_slot` 欄位標識種類。
> Filter plugin 只放行 `warn`（slot 2）和 `error`（slot 3），60% 的 log 在 filter stage 被丟棄，
> 實際進入 transport 的吞吐量為送入量的 40%。

### 3.2 輸入模式

| 項目 | 設定值 | 說明 |
|------|--------|------|
| 模式 | **TCP** | 排除 tail polling 不確定性 |
| 端口 | `5140` | WCM TCP input |
| 傳送 | 直連 TCP（`-output tcp`） | 避免 nc 中介緩衝在 backpressure 下累積舊時間戳記的 log |
| 比較工具 | 各工具同樣從 TCP 讀入 | 需確認 |

### 3.3 批次參數（**所有比較工具必須對齊**）

| 參數 | WCM 設定 | Fluent Bit 對應 | Vector 對應 |
|------|----------|-----------------|-------------|
| Batch timeout | `max_wait_ms: 100` | `Flush 0.1` | `batch.timeout_secs: 0.1` |
| Batch max lines | `max_batch_lines: 1000` | chunk line limit | `batch.max_events: 1000` |
| Transport chunk | `max_transport_bytes: 131072` | `storage.Chunk_Size_Limit 128k` | `batch.max_bytes: 131072` |

> `max_wait_ms=100ms` 直接疊加在 p50 上（約佔 p50 的 40%），比較時各工具必須設定相同值，
> 否則 p50 差異反映的是 batch timeout 而非 pipeline 處理速度。

### 3.4 速率策略

**不固定絕對 lines/s，改用各工具飽和點的比例。**

實驗流程：
1. 先跑爬坡測試找出 WCM 的飽和吞吐量（p99 開始暴增的轉折點）
2. 對各比較工具做相同的爬坡
3. 延遲實驗在 **30% / 50% / 80%** 飽和點各跑一次

現階段低流量測試：**500 lines/s**（filter 後實際 ~200 lines/s 進入 transport）

### 3.5 環境與系統

| 項目 | 設定 |
|------|------|
| Build mode | `cargo build --release` |
| WASM 插件 | 全部 C 語言，使用 `-O2` 編譯 |
| Transport plugin | C plugin 使用 `wasi:http/outgoing-handler`（HTTP POST）|
| config_reload_secs | `3600`（避免 watcher 干擾量測） |
| parse_noop | **停用**（`parse_noop: ~`） |
| transport_workers | `2` |
| 系統背景程序 | 量測期間關閉無關程序 |
| Warm-up | 前 **30 秒**資料丟棄，不計入分佈（前 30s spike rate 明顯偏高）|
| 量測窗口 | warm-up 後取穩態資料（duration=180s，有效量測窗口 T=30~180s）|

### 3.6 Transport sink

- 使用 `tools/test/server.go`（編譯後直接執行 binary，SIGTERM 觸發 shutdown 並寫出 CSV）
- 單一 endpoint，tag = `A`（不使用 fan-out）
- CSV 格式：`ts, receive_unix, latency_ms`（`receive_unix` 為 nanosecond 精度的 Unix 時間轉 float64）

---

## 4. WCM 設定值（forwarder.yaml）

```yaml
plugins:
  parse:      "latency_c_parser.wasm"
  parse_noop: ~                          # 停用 noop diff
  filter:     "latency_c_filter.wasm"    # level >= warn 通過，丟棄 info/debug（約 60%）
  format:     "latency_c_format.wasm"    # JSON flat 格式輸出
  transport:  "transport_http.wasm"      # wasi:http/outgoing-handler HTTP POST

stages:
  filter:    true
  format:    true
  transport: true

input:
  mode: tcp
  tcp:
    host: "0.0.0.0"
    port: 5140

batch:
  mem_limit_mb:               256
  safe_data_ratio:            0.5
  max_wait_ms:                100       # 決定 p50 基線的關鍵參數，比較時需對齊
  max_batch_lines:            1000
  channel_capacity:           150000
  max_format_chunk:           50000
  max_transport_bytes:        131072    # 128 KB
  transport_workers:          2
  pipeline_channel_capacity:  20000
  stage_timeout_ms:           5000

endpoint:
  A: "http://127.0.0.1:8080/ingest"

config_reload_secs: 3600
```

---

## 5. 執行緒說明（比較公平性）

### 低流量（目前階段）

WCM 在 500 lines/s（filter 後 ~200/s）下實際活躍 thread：
- Input reader: 1 thread（block on TCP read）
- Parse dispatcher: 1 thread
- Parse worker: 1 thread（PARSE_WORKERS hardcoded）
- Transport workers: 2 threads（`transport_workers: 2`）

**低流量時各 thread 幾乎全在等待，多執行緒帶來的吞吐優勢未被啟動。**
延遲差異主要由批次 timeout（100ms）與 WASM 處理時間決定。

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

## 7. 執行指令

### 低流量測試（500 lines/s，180s）

使用 `latency.sh` 一鍵執行（編譯 server binary、啟動 server、編譯並啟動 forwarder、執行 gen）：

```bash
cd test/ours/latency-benchmark
./latency.sh
```

手動執行 generator（供調試）：

```bash
go run tools/gen/main.go \
  -rate 500 \
  -duration 180 \
  -traffic flat \
  -mode json-fixed5 \
  -output tcp \
  -send-unit line \
  -tcp-addr 127.0.0.1:5140 \
  -log-file /tmp/gen.log
```

### 飽和點爬坡測試（未來）

```bash
for rate in 1000 2000 5000 10000 20000 50000; do
  go run tools/gen/main.go -rate $rate -duration 30 -traffic flat \
    -mode json-fixed5 -output tcp -send-unit line -tcp-addr 127.0.0.1:5140
  sleep 5
done
```

---

## 8. 目前測試結果（供參考）

測試條件：500 lines/s，flat，json-fixed5，duration=300s

| 指標 | 全量 | 穩態（T≥30s） |
|------|------|--------------|
| n | 60,001 | ~54,000 |
| p50 | 242.7ms | 241.4ms |
| p95 | 568.8ms | 561.3ms |
| p99 | 815.9ms | 818.3ms |
| max | 2304.8ms | 2304.8ms |
| >800ms | 1.07% | 1.08% |
| 負值 | 0 | 0 |

**max 說明**：全部 >2000ms（61 筆）集中在 T+264.8s 的 1.7ms 內，為單次 batch hold 事件（非網路問題、非系統性問題）。

**Filter 行為**：只有 `warn`（slot 2）和 `error`（slot 3）通過 filter，`info`/`debug`（slot 1/4/5）被丟棄。兩種通過的 log 延遲幾乎相同（p50 差 < 0.2ms），確認 log 內容不是延遲差異來源。

---

## 9. 待辦事項

- [ ] **Fluent Bit / Vector 的批次設定對齊**：確認 `Flush 0.1` / `batch.timeout_secs: 0.1` 設為 100ms
- [ ] **確認 C 插件是否經過 wasm-opt**：檢查 Makefile，補上 `wasm-opt -O2` 步驟
- [ ] **讓 PARSE_WORKERS 可由 YAML 控制**（目前 hardcoded in runtime.rs）
- [ ] **output.rs per-batch eprintln 靜音**：高吞吐測試時 stderr 輸出會增加 p99，需提供關閉選項
- [ ] **飽和點爬坡測試**：找出各工具的飽和吞吐量轉折點，作為比較基準的速率選擇依據

---

## 10. 已排除的變因（不需控制原因）

| 排除項目 | 原因 |
|----------|------|
| json-mixed / json-complex | 線長 variance 過大，保留為次要測試 |
| wave / bursty traffic | 突發會造成排隊延遲，與處理延遲混淆 |
| parse_noop（diff measurement） | 啟用時每批多跑一次 WASM，使吞吐量減半 |
| Fan-out（多 endpoint） | 造成額外 clone 分配，不是通用基準 |
| Go / C# 插件 | GC jitter 汙染延遲分佈，用於語言相容性驗證，非效能基準 |
| nc 管線傳送 | backpressure 下會在 pipe buffer 累積舊時間戳記的 log，造成假性大延遲 |
| float64 時間戳記 | float64 在 Unix 秒數（~1.7×10⁹）下有精度損失，改用 int64 nanoseconds |
