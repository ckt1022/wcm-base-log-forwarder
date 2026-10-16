# 原生 WASM 比較項目

建立與目前 WCM 設計類似的結構，但以 **offset + length** 傳遞資料（而非複製），比較兩者在記憶體複製與 instance 啟動上的開銷差異。同樣為 4 階段與 wasmtime 作為 runtime。

## 自定義邏輯出現問題的情況

### 比較兩者在開銷的差異
- [ ] Copy in 時間
- [ ] Copy out 時間
- [ ] Instance 啟動時間

## 測試指標
- [ ] 吞吐量
- [ ] 延遲
- [ ] Memory usage
- [ ] CPU usage

## 測試方法

### 兩種模式定義
| 模式 | 資料傳遞方式 |
|------|------------|
| Copy-based（現行 WCM）| host 將資料複製進 WASM linear memory，call function，再複製出結果 |
| Offset+Length | host 直接傳 shared buffer 的 offset + length，WASM 讀取後回寫同一塊 memory |

### 資料集
JSON mix：每行一個 JSON object，欄位含 `level`、`msg`、`ts`（Unix ms）、`host`、`payload`（隨機字串）。

### 流量模式
| 模式 | 說明 | 參數 |
|------|------|------|
| Steady  | 固定速率持續輸入 | 5,000 lines/s × 120s |
| Burst   | 基線 → 峰值 → 基線，重複 3 次 | 2,000 → 20,000 → 2,000 lines/s，各維持 20s |
| Ramp-up | 逐步爬升 | 每 30s +2,000 lines/s，從 1,000 升至 20,000 |

### 工具
- **Log 生成**：`../tools/loggen.py`，寫入 `/tmp/test-logs.log`
- **HTTP sink**：`../tools/sink_server.py`，監聽 port 8080
- **指標收集**：`pidstat -u -r 1 -p <PID> > metrics.csv`
- **微觀計時**：在 Rust host 端用 `std::time::Instant` 分別計時 copy-in、copy-out、instance init，輸出到 CSV

### 量測步驟
1. 啟動 sink server：`python3 ../tools/sink_server.py`
2. 分別執行 copy-based 與 offset+length 兩個版本，各記錄 PID
3. 背景啟動監控：`pidstat -u -r 1 -p <PID> > wasm_metrics.csv &`
4. 執行流量腳本：`python3 ../tools/loggen.py --mode steady`
5. 比較兩份 metrics CSV 的差異

### 關鍵量測點（需在 host 端埋點）
```rust
let t0 = Instant::now();
// copy data into WASM memory
let copy_in_us = t0.elapsed().as_micros();

let t1 = Instant::now();
// call WASM function
let exec_us = t1.elapsed().as_micros();

let t2 = Instant::now();
// copy result out
let copy_out_us = t2.elapsed().as_micros();
```

**觀察重點**：大 payload（>4KB）時兩種方式的差距是否顯著、instance 複用（pre-instantiation pool）對啟動時間的影響。
