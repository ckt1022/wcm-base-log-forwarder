# tools/

測試與基準測試工具集。

## 目錄結構

```
tools/
├── gen/
│   └── main.go        # Log 產生器
└── bench/
    ├── run.sh         # Benchmark 腳本（輸出 CSV）
    ├── analyze.py     # 結果分析與摘要
    └── results/       # run.sh 的輸出目錄（git ignored）
```

---

## gen — Log 產生器

從 stdin pipe 餵資料給 forwarder 使用。

### 參數

| 參數 | 預設 | 說明 |
|------|------|------|
| `-rate` | 5000 | 每秒行數 |
| `-duration` | 30 | 執行秒數（0 = 無限） |
| `-mode` | simple | `simple` / `complex` / `mixed` / `invalid` |
| `-invalid-rate` | 0.05 | mode=invalid 時，混入無效行的比例 |
| `-buffer` | 1MB | stdout buffer 大小 |
| `-flush-ms` | 100 | stdout flush 間隔（ms） |
| `-seed` | 0 | 亂數種子（0 = 用時間） |

### 模式說明

- **simple**：少量 att 欄位，固定訊息，最低 parsing 負擔
- **complex**：多個 att 欄位 + 較長訊息，模擬真實日誌
- **mixed**：50% simple + 50% complex
- **invalid**：混入 `-invalid-rate` 比例的格式錯誤行，測試 parser 錯誤處理

### 使用範例

```bash
# 基本測試（確認系統能跑）
go run tools/gen/main.go -rate 1000 -duration 10 | ./target/debug/wcm-base-log-forwarder

# 複雜格式壓力測試
go run tools/gen/main.go -rate 10000 -duration 60 -mode complex | ./target/release/wcm-base-log-forwarder

# 測試錯誤處理（5% 無效行）
go run tools/gen/main.go -rate 5000 -duration 30 -mode invalid -invalid-rate 0.05 | ./target/debug/wcm-base-log-forwarder

# 固定 seed 讓輸入可重現（方便對比實驗）
go run tools/gen/main.go -rate 5000 -duration 30 -seed 42 | ./target/release/wcm-base-log-forwarder
```

---

## bench — Benchmark 腳本

### 執行

```bash
# 在 project root 執行
bash tools/bench/run.sh
```

腳本會：
1. 自動 `cargo build --release`（如果 binary 不存在）
2. 跑完整實驗矩陣（rates × modes × 5 runs）
3. 每組先做 warmup，再正式量測
4. 輸出 CSV 到 `tools/bench/results/results_<timestamp>.csv`

### 分析結果

```bash
python3 tools/bench/analyze.py tools/bench/results/results_<timestamp>.csv
```

輸出：
- 每組 (system × mode × rate) 的平均吞吐量、標準差、Peak RSS、執行時間
- Throughput Ceiling 偵測（飽和點估算）

### 自訂比較對象

未來加入 Fluent Bit 對照組時，在 `run.sh` 的主迴圈中新增一個 `SYSTEM` 維度即可，CSV 格式已包含 `system` 欄位。
