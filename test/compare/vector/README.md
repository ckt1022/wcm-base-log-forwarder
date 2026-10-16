# Vector 比較項目

## 製造 Vector 自定義邏輯出現問題的情況

### DSL 邏輯出問題
- [ ] 進入無限迴圈
- [ ] I/O blocking
- [ ] CPU exhaustion
- [ ] Memory exhaustion
- [ ] 單筆解析錯誤
- [ ] 錯誤後的處理能力

## 測試指標（加入過濾）
- [ ] 吞吐量
- [ ] 延遲
- [ ] Memory usage
- [ ] CPU usage

## 測試方法

### 資料集
JSON mix：每行一個 JSON object，欄位含 `level`、`msg`、`ts`（Unix ms）、`host`、`payload`（隨機字串）。

### 流量模式
| 模式 | 說明 | 參數 |
|------|------|------|
| Steady  | 固定速率持續輸入 | 5,000 lines/s × 120s |
| Burst   | 基線 → 峰值 → 基線，重複 3 次 | 2,000 → 20,000 → 2,000 lines/s，各維持 20s |
| Ramp-up | 逐步爬升 | 每 30s +2,000 lines/s，從 1,000 升至 20,000 |

### 工具
- **Log 生成**：`../tools/loggen.py`，寫入 `/tmp/test-logs.log`，Vector 以 `file` source 讀取
- **HTTP sink**：`../tools/sink_server.py`，監聽 port 8080，Vector 以 `http` sink 轉發
- **指標收集**：`pidstat -u -r 1 -p <PID> > metrics.csv`，或使用 Vector 內建 `internal_metrics` source

### 量測步驟
1. 啟動 sink server：`python3 ../tools/sink_server.py`
2. 啟動 Vector，記錄 PID
3. 背景啟動監控：`pidstat -u -r 1 -p <PID> > vector_metrics.csv &`
4. 執行流量腳本：`python3 ../tools/loggen.py --mode steady`
5. 結束後計算：
   - `throughput = total_lines_received / elapsed_s`
   - `latency = avg(sink_recv_ts - log_write_ts)`

### 故障注入方式（VRL）
| 場景 | VRL / 設定做法 |
|------|---------------|
| Infinite loop   | VRL 不支援迴圈，改用 Lua transform：`while true do end` |
| CPU exhaustion  | Lua transform 緊密計算迴圈 |
| Memory exhaustion | Lua transform：`a = {}; while true do a[#a+1] = string.rep('x',1024) end` |
| 單筆解析錯誤    | 輸入含 `{broken` 的行，觀察 `dropped_events` 指標 |
| 錯誤後的處理能力 | 在 VRL 加 `abort`，觀察後續 event 是否繼續流動 |

**注意**：VRL 本身有執行時間限制，無限迴圈場景需改用 Lua transform。
**觀察重點**：Vector 的 `on_error` 行為、`dropped_events_total` 指標、是否影響其他 pipeline。
