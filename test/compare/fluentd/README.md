# Fluentd 比較項目

## 製造 Fluentd 自定義邏輯出現問題的情況

### Ruby plugin 出問題
- [ ] 進入無限迴圈
- [ ] I/O blocking
- [ ] CPU exhaustion
- [ ] Memory exhaustion
- [ ] 單筆解析錯誤

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
- **Log 生成**：`../tools/loggen.py`，透過 TCP forward input 推送給 Fluentd（port 24224）
- **HTTP sink**：`../tools/sink_server.py`，監聽 port 8080，記錄每秒接收量與接收時間戳
- **指標收集**：`pidstat -u -r 1 -p <PID> > metrics.csv`

### 量測步驟
1. 啟動 sink server：`python3 ../tools/sink_server.py`
2. 啟動 Fluentd，記錄 PID
3. 背景啟動監控：`pidstat -u -r 1 -p <PID> > fluentd_metrics.csv &`
4. 執行流量腳本：`python3 ../tools/loggen.py --mode steady --target tcp`
5. 結束後計算：
   - `throughput = total_lines_received / elapsed_s`
   - `latency = avg(sink_recv_ts - log_write_ts)`

### 故障注入方式
| 場景 | Ruby plugin 做法 |
|------|-----------------|
| Infinite loop   | `loop do; end` |
| I/O blocking    | `sleep(Float::INFINITY)` |
| CPU exhaustion  | `while true; Math.sqrt(rand); end` |
| Memory exhaustion | `a = []; loop { a << 'x' * 1024 }` |
| 單筆解析錯誤    | 輸入含 `{broken` 的行 |

**觀察重點**：問題發生後 worker 是否 crash、是否影響其他 worker、GIL 對並發的影響。
