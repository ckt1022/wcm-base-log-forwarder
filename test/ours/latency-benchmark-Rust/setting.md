# 延遲實驗固定變因（WCM Forwarder vs Fluent Bit + Lua filter）

實驗變因只有一個：**forwarder 本體**（WCM base log forwarder ↔ Fluent Bit）。
以下所有項目在兩組實驗中必須完全相同。

## A. 負載端（gen）— 指令與參數完全固定

```bash
go run tools/gen/main.go \
    -rate 500 -duration 300 -traffic flat \
    -mode json-fixed5 -output tcp -send-unit line \
    -tcp-addr 127.0.0.1:5140
```

- 500 行/秒、flat、300 秒，共 150,000 行，逐行寫入 TCP。
- `json-fixed5`：5 條固定內容輪流，`ts` 每行即時刷新（RFC3339Nano）：

| slot | level | service | code | 過 warn filter？ |
|------|-------|---------|------|------------------|
| 1 | info  | api-gateway | 200 | ✗ |
| 2 | warn  | payment-svc | 503 | ✓ |
| 3 | error | auth-service | 401 | ✓ |
| 4 | debug | notify-svc | 200 | ✗ |
| 5 | info  | order-svc | 200 | ✗ |

- 沒有 invalid 行（`-invalid-rate` 只在 `mode=invalid` 時生效）。
- t=15s 會額外注入一行 `{"level":"warn","msg":"LOOP trigger","loop":"true"}`，
  level=warn 會通過 filter 抵達 server（CSV 中的單行 batch）。

## B. 管線語意 — Fluent Bit 端必須對齊的行為

| 階段 | 本 forwarder 的行為 | Fluent Bit 對應 |
|------|--------------------|-----------------|
| Input | TCP :5140，line-delimited JSON | `in_tcp`（format json） |
| Parse | JSON 解析，`ts` 原字串搬到 `timestamp` | in_tcp json parser + Lua |
| Filter | **只保留 level ≥ warn**（丟 debug/info），保留率 40% = 200 行/秒 | **Lua filter 實作同一規則** |
| Format | NDJSON，每行含欄位：`id, timestamp, level, message, lang, _slot, service, code` | Lua 重組 record |
| Transport | HTTP POST `http://127.0.0.1:8080/ingest`，body = NDJSON | `out_http`（format json_lines） |

關鍵約束：

1. **`timestamp` 欄位必須是 gen 原始 `ts` 字串原封不動**（RFC3339Nano）。
   server 靠它計算延遲；Fluent Bit 預設會用自己的時間戳/`date` 欄位，必須在
   Lua 裡把原始 `ts` 改名為 `timestamp` 保留下來，並確認 out_http 沒有覆蓋。
2. **批次節奏對齊**：本 forwarder `max_wait_ms=100`（每 100ms 切一批），
   Fluent Bit 必須設 `Flush 0.1`。預設 Flush 1s 會直接讓延遲差一個數量級，
   比較就失去意義。
3. payload 欄位集合盡量一致（影響每行 bytes 數）：本 forwarder 輸出約
   `{"id":N,"timestamp":"...","level":"warn","message":"...","lang":"Rust","_slot":"2","service":"payment-svc","code":"503"}`。

## C. 量測端 — 完全固定

- `tools/test/server.go`（Go，port 8080，`/ingest`）。
- 延遲定義：server **讀完整個 HTTP body 後**的 `time.Now()` − 每行 `timestamp`。
- 輸出 `server_log/latency_worker_1.csv`（SIGTERM 時寫出）。
- 流程與時序（照 latency.sh）：server 啟動 → DUT 啟動 → **等 5s 暖機** →
  gen 跑 300s → **drain 10s** → 依序 SIGTERM。

## D. 環境 — 每次跑之前檢查

- 同一台 WSL2 機器，所有流量走 127.0.0.1 loopback。
- **時鐘修正必須生效**（本實驗曾因時鐘被 NTP 每 32s 回撥 ~1.4s 而污染數據）：
  - `cat /sys/devices/system/clocksource/clocksource0/current_clocksource`
    → 必須是 `hyperv_clocksource_tsc_page`
  - `systemctl is-active systemd-timesyncd` → 必須是 `inactive`
  - 開跑前用 70s clock-step 偵測腳本確認安靜；跑完檢查 CSV **不得有負延遲**。
- 兩邊都用 release / 官方發行版建置。
- 背景負載盡量一致（同樣的 VSCode / 服務開著或關著）。

## E. 屬於 DUT 的參數（不是固定變因，但必須記錄）

- 本 forwarder（forwarder.yaml）：mem_limit 256MB、max_batch_lines 1000、
  transport_workers 2、stage_timeout 5000ms、wasi:http（每 POST 新開 TCP 連線）。
- Fluent Bit：mem_buf_limit、workers、retry、`net.keepalive`（預設 on，
  連線行為與本 forwarder 不同——這屬於受測系統差異，記錄下來即可）。

## 已知解讀注意事項

- 本 forwarder 的延遲尖峰（~1s / ~3.2s）疑似 per-request TCP 連線的 SYN 重傳，
  尚待抓包證實；比較 Fluent Bit 結果時注意它的 keep-alive 行為不同。
- 基準延遲（無停頓時 p50 ≈ 150–350ms）主要由 100ms 批次窗構成，兩邊對齊
  Flush 後才可直接比較。
