# WCM Base Log Forwarder — 研究規劃

> 方向 A（系統設計 + 效能評估）為主，保留方向 C（Vector 失敗案例對比）的可能性

---

## 整體時程概覽

| Phase | 內容 | 週次 |
|-------|------|------|
| Phase 1 | 系統建構 | 第 1–8 週 |
| Phase 2 | 工程強化 | 第 9–13 週 |
| Phase 3 | 實驗設計 | 第 14–16 週 |
| Phase 4 | 量化實驗 | 第 17–22 週 |
| Phase 5 | 論文撰寫 | 第 23–30 週 |

---

## Phase 1：系統建構（第 1–8 週）

> 目標：讓完整 pipeline 跑起來，能接收日誌並輸出

### 週 1–2：Transport + Formatter（最優先）

~~- [ ] 實作 `transport-plugin` WIT 介面（stdout / HTTP POST / file）~~
~~- [ ] 實作 `formatter-plugin`（JSON 格式輸出）~~
~~- [ ] 驗證 parser → formatter → transport 的端到端流程~~

### 週 3–4：Enricher + Reduction

- [ ] 實作 `enricher-plugin`（加入 hostname、timestamp、source tag）
- [ ] 實作 `reduction-plugin`（依 log level 過濾，例如只保留 ERROR 以上）
- [ ] 用 Go 寫出對應的 WASM 插件範例

### 週 5–6：Router + Masking

- [ ] 實作 `route-plugin`（依 tag 決定走哪個 transport）
- [ ] 實作 `masking-plugin`（regex 遮蔽 IP、email、token）
- [ ] 讓整條 pipeline 串接完成

### 週 7–8：多語言插件驗證（論文重要貢獻點）

- [ ] 用 **Rust** 寫一個 parser plugin（對比 Go 版）
- [ ] 驗證不同語言的插件在同一 host 上能正確運作
- [ ] 整理插件開發流程文件（本身是論文貢獻之一）

---

## Phase 2：工程強化（第 9–13 週）

> 目標：處理生產環境必要問題，同時為實驗製造可觀測點

### 週 9–10：Buffer 與背壓（Backpressure）

- [ ] 設計 disk-based buffer（當下游 transport 壅塞時，不丟資料）
- [ ] 實作背壓機制：當 channel 快滿時，通知 stdin reader 減速或阻塞
- [ ] 量化 buffer 大小對吞吐量/延遲的影響（實驗數據）

**背壓架構方向：**

```
目前: stdin → channel(25000) → batch → WASM
改為: stdin → channel → [壓力感測] → adaptive batch size
     當 channel 使用率 > 80% 時，觸發提前 flush 或 slow-read
```

### 週 11–12：錯誤隔離與恢復

~~- [ ] 插件 crash 的 recovery 機制（不能讓一個壞插件搞垮整個 forwarder）~~
- [ ] Poison message 處理（無法解析的 log：skip / dead-letter queue）
- [ ] 加入結構化的 error log 輸出

### 週 13：熱重啟（Hot Reload）

- [ ] 設計插件替換機制（新版 .wasm 上線，不停機切換）
- [ ] 跟傳統 native plugin（需要重啟 process）做對比
- [ ] 作為論文的 qualitative 討論點

---

## Phase 3：實驗設計（第 14–16 週）

> 目標：在跑實驗之前，先設計好「要量什麼、怎麼量、怎麼比」

### 週 14：建立比較對象的環境

| 系統 | 插件方式 | 代表意義 |
|------|----------|----------|
| **本系統** | WASM Component Model | 論文主角 |
| **Fluent Bit + WASM filter** | 舊式 WASM (C ABI) | 主要對照組 |
| **Fluent Bit + Lua filter** | 原生腳本 | 傳統方案基準 |
| **本系統（無 WASM）** | Rust 原生硬編碼 parser | 理論效能上限 |

- [ ] 安裝並設定 Fluent Bit，配置同等功能（stdin → 解析 → 輸出）
- [ ] 寫 Fluent Bit 的 WASM filter 版本（相同的 parsing 邏輯）
- [ ] 確保四組系統做的事情**功能等價**

### 週 15：設計實驗矩陣

**輸入變數（Independent Variables）：**

```
├── 輸入速率：1k / 5k / 10k / 20k logs/sec
├── Log 格式複雜度：簡單(空白分隔) / 中等(JSON) / 複雜(多 regex)
├── Batch 大小：小(1k lines) / 中(10k) / 大(50k)
└── Pipeline 深度：1 插件 / 3 插件 / 6 插件全開
```

**量測目標（Dependent Variables）：**

```
├── 吞吐量 (logs/second)
├── 端到端延遲 P50 / P95 / P99 (ms)
├── 記憶體用量 Peak RSS (MB)
├── CPU 使用率 (%)
└── 插件錯誤下的系統穩定性
```

- [ ] 寫實驗腳本，確保每組測試重複 **5 次以上**（取平均 + 標準差）
- [ ] 設計 **ablation study**：單獨量 Component Model 本身的 overhead

### 週 16：測試日誌生成器

- [ ] 擴充現有的 Go log generator，支援：
  - 可調整的格式複雜度
  - 可設定 burst pattern（突發流量）
  - 輸出 nginx / syslog / k8s 格式日誌
- [ ] 準備真實日誌資料集（公開資料集：HDFS log、OpenStack log）

---

## Phase 4：量化實驗（第 17–22 週）

### 週 17–18：基線效能測試

- [ ] 量測 4 組系統在穩定負載下的基礎效能
- [ ] 畫出吞吐量曲線（x 軸：輸入速率，y 軸：輸出速率）
- [ ] 找到每個系統的**飽和點**（throughput ceiling）

### 週 19–20：壓力與邊界測試

- [ ] Burst traffic 實驗：瞬間 10x 流量，觀察各系統表現
- [ ] 記憶體壓力實驗：大量複雜 log，觀察 OOM 行為
- [ ] 插件錯誤注入：讓插件故意 panic / 無限迴圈，觀察隔離效果（安全性論點的核心實驗）

### 週 21：插件開發成本比較（定性 + 定量）

- [ ] 記錄「寫一個新的 parser 插件」需要多少行程式碼
- [ ] 記錄編譯到 WASM 需要的步驟數
- [ ] 跟 Fluent Bit 的插件開發流程做比較（Developer Experience 貢獻）

### 週 22：數據整理與視覺化

- [ ] 將所有實驗結果整理成圖表
- [ ] 計算統計顯著性（t-test 或 Mann-Whitney U test）
- [ ] 找出不如預期的數據，補充解釋

---

## Phase 5：論文撰寫（第 23–30 週）

- [ ] Abstract + Introduction（問題、貢獻、論文結構）
- [ ] Related Work（Fluent Bit、Vector、WasmEdge/WLF、Component Model 相關論文）
- [ ] System Design（架構設計、WIT 介面設計決策）
- [ ] Implementation（關鍵實作細節：batch 策略、記憶體管理）
- [ ] Evaluation（量化實驗結果與分析）
- [ ] Discussion（限制、方向 C 的 Vector 對比可在此展開）
- [ ] Conclusion

---

## 如何進行定量測試

### 測量工具

```bash
# 記憶體與 CPU（簡易）
/usr/bin/time -v ./target/release/wcm-base-log-forwarder

# CPU 詳細分析
perf stat ./target/release/wcm-base-log-forwarder

# 持續取樣（整個實驗過程）
pidstat -u -r -p <PID> 1 > metrics.csv
```

### 公平比較的原則

**兩個系統要做「功能等價」的事。** 例如，若本系統做：
1. 解析 nginx log → 提取 IP、status code、url
2. 過濾掉 status < 400 的 log
3. 輸出為 JSON 到 stdout

則 Fluent Bit 的設定也要做完全相同的事。

### 實驗腳本範例

```bash
#!/bin/bash
# run_benchmark.sh

RATES=(1000 5000 10000 20000)
RUNS=5
DURATION=60  # 秒

for RATE in "${RATES[@]}"; do
  for RUN in $(seq 1 $RUNS); do
    echo "=== Rate: $RATE, Run: $RUN ==="

    # 本系統
    go run log_generator.go -rate $RATE -duration $DURATION | \
      /usr/bin/time -v ./target/release/wcm-base-log-forwarder \
      2> results/your_system_${RATE}_${RUN}.txt

    # Fluent Bit (WASM)
    go run log_generator.go -rate $RATE -duration $DURATION | \
      /usr/bin/time -v fluent-bit -c fluent-bit-wasm.conf \
      2> results/fluentbit_wasm_${RATE}_${RUN}.txt

    sleep 5  # 冷卻時間
  done
done
```

### 統計顯著性分析

```python
# analyze.py
import scipy.stats as stats

your_system = [結果1, 結果2, 結果3, 結果4, 結果5]
fluent_bit  = [結果1, 結果2, 結果3, 結果4, 結果5]

t_stat, p_value = stats.ttest_ind(your_system, fluent_bit)
# p < 0.05 → 差異具統計顯著性，可在論文中聲稱「顯著優於/劣於」
```

### 預期論文圖表

| 圖表 | 呈現內容 |
|------|----------|
| Figure 1 | 系統架構圖 |
| Figure 2 | 吞吐量 vs 輸入速率（4 條線：4 個系統） |
| Figure 3 | P99 延遲 vs 輸入速率 |
| Figure 4 | 記憶體用量 vs Batch 大小 |
| Figure 5 | 插件崩潰隔離實驗（穩定性對比） |
| Table 1 | 插件開發成本比較（LOC、步驟數） |

---

## 研究面必要工作（常被忽略）

- [ ] **閱讀並整理 Related Work**：至少 15 篇論文（Fluent Bit、Vector、WASM 效能、Plugin 架構設計）
- [ ] **定義正式的 Research Questions**：論文需明確回答 2–3 個問題
- [ ] **威脅模型（Threat Model）**：說明安全隔離的假設前提（例如：插件程式碼不受信任）
- [ ] **Reproducibility**：提供 Docker Compose + 一鍵重現實驗的腳本
- [ ] **開源準備**：論文投出前整理 repo，補充 README 和實驗腳本
