# 實驗結果與清理候選盤點

盤點日期：2026-08-09

本文件只列出目前結果與清理候選，**尚未刪除任何資料**。容量為 `du` 的近似值；重新執行實驗後可能改變。

## 1. 正式結果索引：建議保留

| 類型 | 正式摘要／圖表 | 建議保留的原始結果 |
|---|---|---|
| 所有實驗索引 | `test/compare/EXPERIMENTS.md` | 各節所列腳本與結果 |
| 總結果表 | `實驗結果.csv` | — |
| 128–2048 B 多欄位容量 | `test/compare/results/multifield_capacity_256_2048.csv`、`.svg`、`multifield_capacity_analysis.md` | `test/compare/capacity/runs/{wcm-b1,fluentd,fbit-lua,vector,fbit-wasm}/cap_rate_*_fields*`；1024 B 正式值以 `*_capacity_manual_*` 為準 |
| 1024 B 延遲／CPU／MEM | `test/compare/results/*_fields1k_latency_cgroup_summary.*`、`five_product_latency_charts/` | 下列「正式延遲 raw」目錄 |
| WCM 正確性 | `test/compare/results/experiment_catalog_charts/02_wcm_correctness.svg` | `test/compare/correctness/runs/wcm_correctness_formal_20260804/`、`wcm_correctness_edge_20260804/` |
| 四產品短突發恢復 | `test/compare/resilience/result.md`、各 formal run 的 `summary.*` 與 `figures/` | 下列四個 `*_burst_forward30_formal_*` 目錄 |
| 五分鐘持續過載 | `test/compare/resilience/result.md` | WCM、Vector、Fluentd 各三個 `*_overload_manual_r*`；Fluent Bit WASM 的 `fbit_wasm_overload_150pct_5m_r1_20260808/` |
| WCM 一小時穩定性 | `test/compare/resilience/result.md`、`experiment_catalog_charts/04_wcm_one_hour_stability.svg` | `test/compare/resilience/runs/wcm_stability_20k_1h_manual_r1_20260809/` |

### 1.1 正式延遲 raw：建議保留

| 產品 | 目錄名稱規則 | 數量 | 約略容量 |
|---|---|---:|---:|
| WCM | `capacity/runs/wcm-b1/lat_rate_*_fields1k_latency_cgroup_rerun/` | 13 | 361.0 MiB |
| Fluentd | `capacity/runs/fluentd/lat_rate_*_fields1k_latency_sweep/` | 18 | 747.4 MiB |
| Fluent Bit Lua | `capacity/runs/fbit-lua/lat_rate_*_fields1k_latency_cgroup_sweep/` | 19 | 1093.0 MiB |
| Vector | `capacity/runs/vector/lat_rate_*_fields1k_latency_cgroup_sweep/` | 8 | 153.2 MiB |
| Fluent Bit WASM | `capacity/runs/fbit-wasm/lat_rate_*_fields1k_latency_cgroup_sweep/` | 5 | 45.9 MiB |

### 1.2 正式 resilience／correctness raw：建議保留

```text
test/compare/resilience/runs/wcm_burst_forward30_formal_20260805/
test/compare/resilience/runs/vector_burst_forward30_formal_20260805/
test/compare/resilience/runs/fluentd_burst_forward30_formal_20260805/
test/compare/resilience/runs/fbit_wasm_burst_forward30_formal_20260806/

test/compare/resilience/runs/wcm_overload_manual_r1/
test/compare/resilience/runs/wcm_overload_manual_r2/
test/compare/resilience/runs/wcm_overload_manual_r3/
test/compare/resilience/runs/vector_overload_manual_r1/
test/compare/resilience/runs/vector_overload_manual_r2/
test/compare/resilience/runs/vector_overload_manual_r3/
test/compare/resilience/runs/fluentd_overload_manual_r1/
test/compare/resilience/runs/fluentd_overload_manual_r2/
test/compare/resilience/runs/fluentd_overload_manual_r3/
test/compare/resilience/runs/fbit_wasm_overload_150pct_5m_r1_20260808/

test/compare/resilience/runs/wcm_stability_20k_1h_manual_r1_20260809/

test/compare/correctness/runs/wcm_correctness_formal_20260804/
test/compare/correctness/runs/wcm_correctness_edge_20260804/
```

## 2. 第一級：可直接清理或重建的產物

這一級不包含正式實驗結果。預估可釋放約 **2.52 GiB**，其中幾乎全部來自 Vector 暫存 log。

| 路徑 | 容量 | 原因／處理建議 |
|---|---:|---|
| `test/compare/vector/test-logs.log` | 2.5 GiB | 測試輸入暫存檔，不是結果；`run_crash.sh` 啟動時本來就會清空。確認沒有 Vector 實驗正在執行後，建議清空為 0 bytes，保留檔名最穩妥。 |
| `test/compare/results/__pycache__/` | 56 KiB | Python cache，可重建。 |
| `test/compare/resilience/__pycache__/` | 80 KiB | Python cache，可重建。 |
| `test/compare/correctness/__pycache__/` | 8 KiB | Python cache，可重建。 |
| `test/compare/fluentbit/latency/wasm_file/target/` | 16 MiB | Rust build 產物，可重新編譯。 |
| `test/compare/fluentbit/wasm_filter/wasm_file/target/` | 1.1 MiB | Rust build 產物，可重新編譯。 |

補充：`fluentbit/test-logs.log` 與 `fluentd/test-logs.log` 已是 0 bytes，刪除不會節省空間；建議保留作為腳本預期的空白輸入檔。

## 3. 第二級：smoke、selfcheck、warmcheck 與被取代結果

這些資料不是目前正式比較採用的 raw，但部分舊圖表或文件仍引用它們。若要刪除「被取代結果」，應同步更新 `EXPERIMENTS.md` 或 `resilience/result.md`。

### 3.1 Smoke／pilot：可刪除候選（合計約 0.4 MiB）

```text
test/compare/resilience/runs/fbit_wasm_burst_smoke_20260805/
test/compare/resilience/runs/fluentd_burst_smoke_20260805/
test/compare/resilience/runs/vector_burst_smoke_20260805/
test/compare/resilience/runs/wcm_burst_pilot_20260804/
```

### 3.2 Correctness 自我檢查：可刪除候選（合計約 0.2 MiB）

```text
test/compare/correctness/runs/selfcheck/
test/compare/correctness/runs/selfcheck2/
test/compare/correctness/runs/edge_selfcheck/
```

### 3.3 延遲 warmcheck：可刪除候選（合計約 15 MiB）

```text
test/compare/capacity/runs/wcm-b1/lat_rate_2500_fields1k_latency_cgroup_warmcheck/
test/compare/capacity/runs/vector/lat_rate_2500_fields1k_latency_cgroup_warmcheck/
test/compare/capacity/runs/fbit-wasm/lat_rate_2500_fields1k_latency_cgroup_warmcheck/
```

### 3.4 已被新版取代，但文件仍可能引用（合計約 3.7 MiB）

| 路徑 | 被何者取代／注意事項 |
|---|---|
| `resilience/runs/wcm_burst_formal_20260804/` | 已有 `wcm_burst_forward30_formal_20260805/`；但 `EXPERIMENTS.md` 第 6 節目前仍列出其圖。 |
| `resilience/runs/vector_burst_formal_20260805/` | 已有 forward30 版本。 |
| `resilience/runs/vector_burst_norm150_formal_20260805/` | 是另一版正規化標準，目前正式比較採 forward30 結果。 |
| `resilience/runs/wcm_stability_80pct_1h_20260806/` | 已被手動 20,000 EPS 一小時結果取代；`resilience/result.md` 仍保留歷史說明。 |

### 3.5 WCM 舊版、未對齊 cgroup 的 1024 B 延遲 sweep（約 608 MiB）

```text
test/compare/capacity/runs/wcm-b1/lat_rate_*_fields1k_latency_sweep/
```

共 18 個目錄。正式 WCM 延遲已改採 `*_latency_cgroup_rerun/`，因此這批可列為清理候選；刪除後會失去舊量測法的逐筆延遲 raw。

## 4. 第三級：歷史實驗資料，可封存或確認論文不引用後刪除

這一級不是「垃圾」。其摘要可能仍出現在 `EXPERIMENTS.md`、`實驗結果.csv` 或歷史圖表中；刪除 raw 後，摘要與圖片仍在，但無法從逐筆資料重新分析。

### 4.1 WCM buffer／flush／worker 調校目錄（合計約 2576.4 MiB）

```text
test/compare/capacity/runs/wcm-b2/
test/compare/capacity/runs/wcm-b4/
test/compare/capacity/runs/wcm-b6/
test/compare/capacity/runs/wcm-b8/
test/compare/capacity/runs/wcm-b10/
test/compare/capacity/runs/wcm-b12/
test/compare/capacity/runs/wcm-b16/
test/compare/capacity/runs/wcm-b32/
test/compare/capacity/runs/wcm-b64/
test/compare/capacity/runs/wcm-v2/
test/compare/capacity/runs/wcm-tw128/
test/compare/capacity/runs/wcm-flush10ms/
```

這些是舊 WCM 配置搜尋，不是目前五產品相同配置比較的正式 raw。如果論文不討論 WCM 內部調參，可整批封存或刪除。

### 4.2 舊 160 B／歷史延遲逐筆 raw（合計約 1558.7 MiB）

以下指各產品目錄內只有 `lat_rate_<數字>/`、名稱不含 `fields1k` 的目錄：

| 根目錄 | 數量 | 容量 |
|---|---:|---:|
| `capacity/runs/wcm-b1/` | 2 | 185.1 MiB |
| `capacity/runs/fluentd/` | 8 | 556.4 MiB |
| `capacity/runs/fbit-lua/` | 8 | 633.0 MiB |
| `capacity/runs/vector/` | 5 | 138.1 MiB |
| `capacity/runs/fbit-wasm/` | 3 | 46.1 MiB |

這批支援 `EXPERIMENTS.md` 第 2 節的歷史 160 B 結果。如果論文只使用 1024 B／36 欄位結果，可考慮只保留摘要 CSV/SVG，刪除逐筆 raw。

### 4.3 舊產品專用 latency 目錄（約 416 MiB）

```text
test/compare/fluentd/latency/       # 約 85 MiB
test/compare/fluentbit/latency/     # 約 331 MiB，內含約 17 MiB 可重建 target
```

這些包含較早期的 latency CSV、設定與腳本。不要直接整個刪除：若決定捨棄舊結果，應保留設定／腳本，只挑大型結果 CSV 與 `target/` 清理。

## 5. 容量搜尋資料：目前不建議刪除

128、256、512、768、1024、1536、2048 B 的 `cap_rate_*_fields*` 目錄是容量曲線的通過點、失敗點與邊界證據。它們容量很小：五產品非 manual raw 合計約 **11 MiB**，manual 1024 B raw 合計約 **2.3 MiB**。節省空間有限，保留可讓最大吞吐判定可追溯。

`capacity/runs/` 根目錄的搜尋 log 約 42 個、總計約 142 KiB，也沒有清理效益。

## 6. 建議確認順序

1. 先清空 `vector/test-logs.log`，立即釋放約 2.5 GiB，且不影響正式結果。
2. 清除三個 `__pycache__` 與兩個 Rust `target/`，約 17 MiB。
3. 確認後刪除 smoke、selfcheck、warmcheck。
4. 若確定只採新量測法，刪除 WCM 舊 `*_latency_sweep/`，約 608 MiB。
5. 若論文不使用 WCM 調參與 160 B 歷史實驗，再處理第 4 節；建議先壓縮移到封存區，而非立即永久刪除。

## 7. 注意事項

- 工作樹目前已有大量先前人工結果、修改與已移除舊容量資料；本次盤點沒有改動它們。
- 刪除 raw 前，至少保留 `EXPERIMENTS.md`、`實驗結果.csv`、`test/compare/results/`、`test/compare/resilience/result.md` 及第 1 節列出的正式 run。
- 執行任何清理前先確認沒有實驗程序或 Docker container 正在寫入對應檔案。
