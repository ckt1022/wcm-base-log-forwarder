# 突發流量恢復實驗結果

## 1. 實驗目的

本實驗比較 WCM、Vector、Fluentd 與 Fluent Bit WASM 在短期過載後的資料正確性、延遲、backlog、資源使用量與恢復能力。

本結果回答的是：各產品承受相同**相對壓力**後，多久能回到自己的正常穩態。由於各產品的實際 EPS 與 baseline 延遲不同，本結果不能單獨用來判定相同絕對 EPS 下的延遲優劣。

## 2. 實驗條件

- 輸入記錄：1024 B、36 個 JSON 欄位。
- 資源限制：1 CPU、512 MiB、無 swap。
- 輸入：單一 TCP connection。
- 輸出：單一 HTTP worker。
- WCM、Vector、Fluentd：1 MiB batch、1 秒 flush。
- Fluent Bit WASM：1 秒 flush；使用 Fluent Bit 原生約 2 MiB soft chunk，無法精確設為 1 MiB。
- 每個產品的 baseline 為其最大穩定吞吐量 `M` 的 50%。
- Burst 為 `M` 的 150%。
- B1、B2、B3 的 burst duration 分別為 5、15、30 秒。
- Warm-up 180 秒、baseline 120 秒、recovery 600 秒、final drain 120 秒。
- 輸入資料固定保留 40%，其餘由 filter 丟棄。
- 每個產品、每個情境目前各執行一輪。

### 2.1 各產品負載

| 產品 | 最大穩定吞吐量 M | Baseline（50% M） | Burst（150% M） | Baseline 預期輸出 |
|---|---:|---:|---:|---:|
| WCM | 22,250 EPS | 11,125 EPS | 33,375 EPS | 4,450 EPS |
| Vector | 15,250 EPS | 7,625 EPS | 22,875 EPS | 3,050 EPS |
| Fluentd | 50,000 EPS | 25,000 EPS | 75,000 EPS | 10,000 EPS |
| Fluent Bit WASM | 5,500 EPS | 2,750 EPS | 8,250 EPS | 1,100 EPS |

## 3. 恢復判定

從 burst 結束後尋找第一個完整的 30 秒前向區間，且必須同時符合：

1. 30 秒總輸出率在 baseline 預期輸出的 ±5% 內。
2. 區間內每秒 p99 的中位數回到 baseline 門檻內。
3. 區間結束時 output backlog 不超過一秒 baseline 輸出量。
4. 區間結束時 source pending 不超過一秒 baseline 輸入量。
5. 最終資料無遺失、重複、內容錯誤、非預期資料與亂序。
6. DUT 沒有 OOM。

延遲恢復門檻為：

```text
max(1.2 × baseline p99, baseline p99 + 20 ms)
```

「恢復開始」是合格 30 秒區間的開始時間；「確認完成」為恢復開始再加 30 秒。

## 4. 完整實驗結果

| 產品 | 情境 | Burst | 判定 | 恢復開始 | 確認完成 | Baseline p99 | 最大 p99 | 最大 output backlog | 最大 source pending | CPU mean/max | MEM mean/max |
|---|---|---:|---|---:|---:|---:|---:|---:|---:|---:|---:|
| WCM | B1 | 5 s | RECOVERED | 6.194 s | 36.194 s | 152.522 ms | 3,055.992 ms | 25,532 | 33 | 52.108/101.7% | 86.782/118.79 MiB |
| WCM | B2 | 15 s | RECOVERED | 15.133 s | 45.133 s | 150.868 ms | 7,657.587 ms | 68,238 | 44,449 | 49.826/101.7% | 71.223/159.31 MiB |
| WCM | B3 | 30 s | RECOVERED | 53.086 s | 83.086 s | 191.752 ms | 22,859.196 ms | 172,026 | 301,937 | 54.285/104.1% | 81.481/159.57 MiB |
| Vector | B1 | 5 s | RECOVERED | 6.108 s | 36.108 s | 356.818 ms | 2,993.239 ms | 17,137 | 30,560 | 52.057/100.1% | 86.285/105.25 MiB |
| Vector | B2 | 15 s | RECOVERED | 36.110 s | 66.110 s | 365.159 ms | 13,642.695 ms | 65,600 | 153,832 | 65.690/102.1% | 37.928/57.01 MiB |
| Vector | B3 | 30 s | RECOVERED | 70.991 s | 100.991 s | 371.327 ms | 26,823.484 ms | 130,360 | 314,740 | 68.452/101.4% | 38.627/56.56 MiB |
| Fluentd | B1 | 5 s | RECOVERED | 6.075 s | 36.075 s | 115.580 ms | 2,806.547 ms | 52,926 | 124,476 | 52.571/100.2% | 129.273/171.69 MiB |
| Fluentd | B2 | 15 s | RECOVERED | 18.789 s | 48.789 s | 115.379 ms | 8,925.691 ms | 165,370 | 408,480 | 55.645/100.5% | 142.323/192.97 MiB |
| Fluentd | B3 | 30 s | RECOVERED | 38.045 s | 68.045 s | 115.915 ms | 18,101.737 ms | 339,754 | 840,163 | 60.220/100.4% | 136.900/178.90 MiB |
| Fluent Bit WASM | B1 | 5 s | RECOVERED | 9.874 s | 39.874 s | 1,014.507 ms | 4,761.853 ms | 8,269 | 13,027 | 63.860/101.9% | 302.228/307.93 MiB |
| Fluent Bit WASM | B2 | 15 s | RECOVERED | 22.075 s | 52.075 s | 1,011.651 ms | 10,742.566 ms | 21,116 | 44,008 | 55.475/100.8% | 281.463/287.44 MiB |
| Fluent Bit WASM | B3 | 30 s | RECOVERED | 33.069 s | 63.069 s | 1,011.203 ms | 16,755.295 ms | 35,938 | 79,413 | 59.625/100.8% | 281.502/288.34 MiB |

四個產品在全部情境中皆為：

- 遺失：0
- 重複：0
- 內容錯誤：0
- 非預期資料：0
- 亂序：0
- OOM／重啟：0

## 5. 正規化 backlog

不同產品的 baseline EPS 不同，因此不能直接以 backlog 筆數判斷優劣。下表將 backlog 換算為各產品 baseline 的等效秒數。

### 5.1 Output backlog 等效 baseline 輸出秒數

| 產品 | B1 | B2 | B3 |
|---|---:|---:|---:|
| WCM | 5.74 s | **15.33 s** | 38.66 s |
| Vector | 5.62 s | 21.51 s | 42.74 s |
| Fluentd | **5.29 s** | 16.54 s | 33.98 s |
| Fluent Bit WASM | 7.52 s | 19.20 s | **32.67 s** |

### 5.2 Source pending 等效 baseline 輸入秒數

| 產品 | B1 | B2 | B3 |
|---|---:|---:|---:|
| WCM | **0.003 s** | **4.00 s** | **27.14 s** |
| Vector | 4.01 s | 20.17 s | 41.28 s |
| Fluentd | 4.98 s | 16.34 s | 33.61 s |
| Fluent Bit WASM | 4.74 s | 16.00 s | 28.88 s |

WCM 在 B1 幾乎沒有將反壓傳回來源端，B2 的 source pending 也明顯少於其他產品。B3 時 WCM 的 source pending 明顯增加，顯示長突發已開始耗盡內部吸收空間。

## 6. 詳細觀察

### 6.1 吞吐量

Fluentd 的最大吞吐量最高。WCM 排名第二：

- WCM 比 Vector 高約 46%。
- WCM 約為 Fluent Bit WASM 的 4.05 倍。
- WCM 只有 Fluentd 的 44.5%，比 Fluentd 低約 55.5%。

Fluentd 的 TCP、JSON、buffer 與 HTTP 輸出多由成熟原生元件處理，只有欄位轉換經過 Ruby filter。WCM 則需要經過 parse、filter、format 與 transport 等多個 WASM 階段；跨 WASM 邊界、資料轉換及序列化可能降低單核心最大吞吐量。此項屬合理推測，仍需逐階段 profiling 才能證明。

### 6.2 穩態延遲

Fluentd baseline p99 約 115 ms，WCM 約 151–192 ms，Vector 約 357–371 ms，Fluent Bit WASM 約 1,011–1,015 ms。

以 B1 為例：

- WCM 比 Fluentd高約 32%。
- WCM 比 Vector低約 57%。
- WCM 比 Fluent Bit WASM低約 85%。

這些數字同時受到 batch 填滿速度影響。Fluentd baseline 經 filter 後約有 10,000 output EPS，WCM 約有 4,450 output EPS；同為 1 MiB batch 時，Fluentd 能更快填滿 batch。因此此實驗的 baseline p99 適合描述恢復前的各自穩態，不應取代相同絕對 EPS 的低流量延遲比較。

Fluent Bit WASM 的 baseline p99 接近一秒，與其 1 秒 flush 及原生 chunk 行為一致。

### 6.3 恢復時間

| 產品 | B1 | B2 | B3 |
|---|---:|---:|---:|
| WCM | 6.194 s | **15.133 s** | 53.086 s |
| Vector | 6.108 s | 36.110 s | 70.991 s |
| Fluentd | **6.075 s** | 18.789 s | 38.045 s |
| Fluent Bit WASM | 9.874 s | 22.075 s | **33.069 s** |

B1 中 WCM、Vector 與 Fluentd 的差距不到 0.12 秒。由於目前只有一輪，不能宣稱其中任何一個顯著較快。

B2 是 WCM 表現最好的情境。WCM 比 Fluentd 快約 19%、比 Fluent Bit WASM 快約 31%、比 Vector 快約 58%。

B3 時 WCM 不再領先：比 Fluentd 慢約 40%、比 Fluent Bit WASM 慢約 61%，但仍比 Vector 快約 25%。這顯示 WCM 在 15 至 30 秒突發之間可能存在 queue 或 memory 飽和轉折。

### 6.4 理論恢復模型

Burst 為 `1.5M`，最大服務能力為 `M`，因此 burst 期間每秒理論累積 `0.5M`。恢復時輸入降為 `0.5M`，亦有 `0.5M` 可用來清除 backlog。在沒有額外開銷時：

```text
理論恢復時間 ≈ burst duration
```

| 產品 | B1 恢復/突發 | B2 恢復/突發 | B3 恢復/突發 |
|---|---:|---:|---:|
| WCM | 1.24 | **1.01** | 1.77 |
| Vector | 1.22 | 2.41 | 2.37 |
| Fluentd | 1.22 | 1.25 | 1.27 |
| Fluent Bit WASM | 1.97 | 1.47 | **1.10** |

Fluentd 的比例最穩定。WCM B2 幾乎符合理論模型，但 B3 顯著偏離，支持長突發導致內部 queue 飽和或有效服務率下降的推論。

### 6.5 尾延遲

WCM 在 B2 的最大 p99 為 7.658 秒，是四者最低；B3 則增加到 22.859 秒，只優於 Vector。此結果和正規化 backlog 一致：WCM 能有效吸收中等突發，但 30 秒長突發使事件在 queue 中停留更久。

最大 p99 是 burst 後每秒 p99 的最大值，容易受單秒抖動影響。論文中應同時呈現延遲時間序列，而非只報告最大值。

### 6.6 CPU

WCM 的平均 CPU 為 49.83–54.29%，在 B2、B3 低於其他產品。但四個產品使用不同實際 EPS，不能把此結果直接解釋為 WCM 每筆事件最省 CPU。Fluentd 在相近 CPU 下處理更高 EPS，若以 EPS/CPU 衡量，Fluentd 的單核心吞吐效率仍然較高。

所有產品的 CPU max 都接近 100%，表示突發期間確實碰到單核心限制。少量超過 100% 的取樣值可歸因於 cgroup 取樣與時間邊界誤差，不代表持續使用超過一顆 CPU。

### 6.7 記憶體

WCM 的最大 RSS 為 118.79–159.57 MiB：

- 低於 Fluentd 的 171.69–192.97 MiB。
- 遠低於 Fluent Bit WASM 的 287.44–307.93 MiB。
- 高於 Vector B2、B3 的約 57 MiB。

Fluent Bit WASM 配置預留 256 MiB WASM heap，因此其 RSS 高不一定全是事件 backlog。

WCM B2、B3 的最大 RSS 都接近 159 MiB，可能表示 memory guard 或 buffer 上限開始作用。此限制可避免 OOM，但也可能在長突發下提早施加反壓，造成 B3 source pending 與恢復時間上升。

## 7. WCM 優勢

1. **吞吐量排名第二**：明顯高於 Vector 與 Fluent Bit WASM。
2. **穩態延遲良好**：只落後 Fluentd，明顯優於 Vector 與 Fluent Bit WASM。
3. **中等突發恢復能力佳**：B2 的恢復時間、最大 p99 與正規化 output backlog 均為四者最佳。
4. **短期入站吸收能力強**：B1 幾乎沒有 source pending，B2 也只有約 4 秒 baseline 輸入量。
5. **記憶體可控**：最大 RSS 約 160 MiB，低於 Fluentd 與 Fluent Bit WASM，且所有情境均無 OOM。
6. **資料正確性完整**：B1–B3 均為零遺失、零重複、零內容錯誤與零亂序。

## 8. WCM 劣勢

1. **吞吐量明顯低於 Fluentd**：WCM 只有 Fluentd 約 44.5% 的最大 EPS。
2. **長突發出現非線性惡化**：B2 到 B3 的突發時間增加 2 倍，但恢復時間增加約 3.51 倍，source pending 增加約 6.79 倍。
3. **B3 尾延遲偏高**：最大 p99 為 22.859 秒，高於 Fluentd 與 Fluent Bit WASM。
4. **記憶體高於 Vector**：尤其 Vector B2、B3 最大 RSS 僅約 57 MiB。
5. **單一 transport worker 可能限制排空速度**：公平配置固定單 worker，但這可能不是 WCM 的最佳實務配置。

WCM 的長突發轉折可能與以下設定有關：

- 50,000 筆 input channel capacity。
- 128 MiB memory limit。
- 1 MiB buffer。
- pipeline channel capacity 為 2。
- 單一 transport worker。
- memory guard 在壓力升高時施加反壓。

這些機制能控制記憶體並避免 OOM，但長時間過載時可能犧牲恢復速度。

## 9. 公平性與解讀限制

### 9.1 相對恢復不等於相同絕對延遲

恢復門檻取決於各產品自己的 baseline p99。例如 B3 約為：

- Fluentd：139 ms。
- WCM：230 ms。
- Vector：446 ms。
- Fluent Bit WASM：1,213 ms。

因此 Fluent Bit WASM B3 較早恢復，表示它較早回到自身約一秒的穩態，不代表它的絕對延遲低於 WCM 或 Fluentd。

論文中應將結論寫為「回到各自穩態的時間」，並另外報告相同絕對延遲門檻，例如 p99 回到 500 ms 所需時間。

### 9.2 Fluent Bit batch 無法完全對齊

WCM、Vector 與 Fluentd 可使用 1 MiB batch；Fluent Bit 只能使用原生約 2 MiB soft chunk。其結果可納入整體產品比較，但必須在論文中揭露此實作差異。

### 9.3 目前只有單輪

每項情境目前只有一輪，適合描述「觀察到的趨勢」，不能宣稱統計顯著。記憶體平均值也沒有隨 burst duration 單調增加，顯示單輪可能受到 allocator 狀態、啟動相位與取樣窗口影響。

## 10. WCM 綜合評價

WCM 是一個偏向平衡型、具良好短期突發吸收能力的轉發器。其最大吞吐量低於 Fluentd，但高於 Vector 與 Fluent Bit WASM；穩態延遲僅次於 Fluentd。面對 5 至 15 秒的短、中突發時，WCM 能有效吸收流量、降低上游反壓，並在 B2 取得四者最快的恢復時間、最低的最大尾延遲及最少的正規化 backlog。

但在 30 秒長突發下，WCM 的恢復時間、source pending 與尾延遲呈現非線性增加，顯示內部 queue、memory guard 或單一 transport worker 可能接近飽和。整體而言，WCM 的優勢是中高吞吐、低延遲、可控記憶體、資料正確性與短期突發韌性；主要限制是單核心最大吞吐不及 Fluentd，以及長時間過載後的 backlog 排空能力。

## 11. 建議後續實驗

1. WCM 補做 20 秒與 25 秒 burst，以定位 15 至 30 秒之間的飽和轉折。
2. B1–B3 各重複至少三輪，報告平均值、標準差與信賴區間。
3. 記錄 WCM 各 pipeline stage 的 queue depth，確認 backlog 出現在哪一階段。
4. 分別調整 input channel、pipeline channel、memory limit 與 memory guard，驗證 B3 非線性惡化原因。
5. 測試 2、4 個 transport workers，評估排空速度與 CPU、記憶體成本。
6. 除相對 baseline 恢復外，再報告 p99 回到固定 500 ms 門檻及 backlog 完全清空的時間。

## 12. 原始結果位置

- WCM：`runs/wcm_burst_forward30_formal_20260805/summary.csv`
- Vector：`runs/vector_burst_forward30_formal_20260805/summary.csv`
- Fluentd：`runs/fluentd_burst_forward30_formal_20260805/summary.csv`
- Fluent Bit WASM：`runs/fbit_wasm_burst_forward30_formal_20260806/summary.csv`
- 恢復分析程式：`analyze_burst_forward30.py`

## 13. WCM 五分鐘持續過載恢復實驗

### 13.1 實驗配置

- Warm-up：11,125 EPS，180 秒。
- Baseline：11,125 EPS，120 秒。
- Overload：33,375 EPS，300 秒（150% M）。
- Recovery：11,125 EPS，900 秒。
- Final drain：120 秒。
- 其他條件與前述突發實驗相同。

### 13.2 結果

本節正式結果改用使用者人工執行的三輪：`wcm_overload_manual_r1`、`wcm_overload_manual_r2`、`wcm_overload_manual_r3`。先前由代理執行的四輪已依要求刪除，不納入任何統計。

| 指標 | 三輪平均或共同結果 |
|---|---:|
| 判定 | 3／3 RECOVERED |
| 每輪總輸入 | 23,362,500 |
| 每輪預期／實際輸出 | 9,345,000／9,345,000 |
| 遺失／重複／內容錯誤／亂序 | 0／0／0／0 |
| OOM／重啟 | 0／0 |
| Baseline p99 | 150.055 ms |
| 最大 post-burst p99 | 137,609.290 ms |
| 恢復開始 | 266.537 s |
| 完成 30 秒穩態確認 | 296.537 s |
| 最大 output backlog | 1,262,787 筆 |
| 最大 source pending | 3,031,208 筆 |
| CPU mean／max平均 | 66.955%／102.80% |
| MEM mean／max平均 | 123.563／173.03 MiB |

平均最大 output backlog 相當於約 283.77 秒 baseline 輸出量；平均最大 source pending 相當於約 272.47 秒 baseline 輸入量。三輪恢復均早於理想流體模型的300秒，原因不是DUT憑空增加容量，而是單一同步TCP來源在過載期間受到反壓，實際進入DUT的高峰不是完整矩形波。

WCM三輪均未發生OOM、重啟或資料錯誤，且不需要人工介入便能回復穩態。三輪最高RSS觀察值為179.82 MiB，仍明顯低於512 MiB限制。

### 13.3 重要限制

三輪產生器 send-lag p99、p99.9 與最大值平均分別為129.528、131.779與132.074秒。這表示單一同步TCP來源在持續過載期間受到明顯反壓，33,375 EPS是排程提供速率，而不是每一秒都能完整送入DUT的矩形流量。

因此本結果適合支持以下結論：

> WCM 在長時間累積壓力與 TCP 反壓下不會 OOM 或遺失資料，且來源降回 baseline 後能自行恢復。

但不能單靠此輪精確宣稱「WCM 實際連續接收 33,375 EPS 五分鐘」。若要測試不受 TCP backpressure 限制的固定 offered load，需改用多 connection、非同步 sender，或在來源端加入足夠大的獨立送出佇列。

原始結果：`runs/wcm_overload_manual_r1/`、`runs/wcm_overload_manual_r2/`、`runs/wcm_overload_manual_r3/`。

### 13.4 WCM 三輪主要結果與 Vector 對照

以下三輪均由使用者在相同設定下人工執行：

| 指標 | WCM R1 | WCM R2 | WCM R3 | 平均 |
|---|---:|---:|---:|---:|
| 恢復開始 | 272.167 s | 264.346 s | 263.097 s | 266.537 s |
| 完成確認 | 302.167 s | 294.346 s | 293.097 s | 296.537 s |
| Baseline p99 | 150.193 ms | 149.974 ms | 149.998 ms | 150.055 ms |
| 最大 p99 | 141,279.062 ms | 135,924.482 ms | 135,624.327 ms | 137,609.290 ms |
| 最大 output backlog | 1,295,118 | 1,246,766 | 1,246,476 | 1,262,787 |
| 最大 source pending | 3,112,470 | 2,992,550 | 2,988,603 | 3,031,208 |
| CPU mean | 67.111% | 66.993% | 66.761% | 66.955% |
| MEM mean／max | 146.371／179.82 | 122.329／179.72 | 101.988／159.56 | 123.563／173.03 MiB |
| 資料錯誤／OOM | 0／0 | 0／0 | 0／0 | 0／0 |

三輪恢復時間的sample standard deviation為4.916秒，CV約1.84%，範圍為263.097–272.167秒。三輪資料完整性與OOM判定一致。先前由代理執行的輪次不再保留於工作區，也不作為正式或敏感度統計來源。

Vector使用相同相對負載，由使用者人工完成三輪，結果如下：

| 指標 | WCM三輪平均 | Vector三輪平均 |
|---|---:|---:|
| Baseline／Burst | 11,125／33,375 EPS | 7,625／22,875 EPS |
| 恢復開始 | 266.537 s | 347.076 s |
| 完成確認 | 296.537 s | 377.076 s |
| Baseline p99 | 150.055 ms | 354.493 ms |
| 最大 p99 | 137,609.290 ms | 168,011.257 ms |
| Output backlog 等效秒數 | 283.77 s | 324.99 s |
| Source pending 等效秒數 | 272.47 s | 323.40 s |
| CPU mean | 66.955% | 72.064% |
| MEM mean／max平均 | 123.563／173.03 MiB | 57.208／70.88 MiB |
| 資料錯誤／OOM | 0／0 | 0／0 |

Vector三輪平均恢復時間比WCM三輪平均多80.540秒，約慢30.2%；WCM的正規化output backlog、source pending、baseline latency與CPU mean較低，但Vector的記憶體用量較低。Vector三輪恢復時間為311.078–379.079秒，sample SD 34.176秒、CV 9.85%，變異高於WCM人工三輪。

Vector原始結果：`runs/vector_overload_manual_r1/`、`runs/vector_overload_manual_r2/`、`runs/vector_overload_manual_r3/`。

## 14. WCM 一小時 80% 容量穩定性重測

### 14.1 實驗配置

- 輸入：1024 B、36 欄位。
- 流量：20,000 EPS，為 WCM 手動重測最大穩定吞吐量
  25,000 EPS 的 80%。
- Warm-up：180 秒。
- 正式 steady window：3,600 秒。
- Final drain：120 秒。
- 資源：1 CPU、512 MiB、無 swap。
- 預期 filter 後輸出：8,000 EPS。

### 14.2 結果

| 指標 | 結果 |
|---|---:|
| 狀態 | COMPLETED |
| 正式觀察時間 | 3,600 秒 |
| 總輸入 | 75,600,000 |
| 預期輸出 | 30,240,000 |
| 實際輸出 | 30,240,000 |
| 遺失／重複／內容錯誤／亂序 | 0／0／0／0 |
| OOM／重啟 | 0／0 |
| 實際平均輸入 | 20,000.0 EPS |
| 產生器 lag p99 | 1.131 ms |
| 預期／實際輸出 | 8,000.000／7,999.943 EPS |
| p50 中位數 | 84.194 ms |
| p99 中位數／最大值 | 111.665／167.891 ms |
| p99.9 中位數／最大值 | 115.329／174.954 ms |
| 最初／最後 5 分鐘 p99 | 110.894／112.038 ms |
| p99 首末漂移 | +1.144 ms |
| CPU mean／max | 83.844%／101.90% |
| RSS mean／max | 48.437／52.09 MiB |
| 最初／最後 5 分鐘 RSS | 48.158／48.654 MiB |
| RSS 首末差 | +0.496 MiB |
| RSS 線性斜率 | +0.387 MiB/hour |

一小時內實際輸出與理論值的差距小於 0.001%，且產生器
lag p99 僅 1.131 ms，表示 20,000 EPS 能持續送入 WCM，沒有形成長期
TCP 反壓或 throughput deficit。所有預期資料均被正確接收，沒有 OOM
或重啟。

最後 5 分鐘 p99 比最初 5 分鐘高 1.144 ms，相對於約 112 ms 的基準只有
約 1%，沒有觀察到具實務意義的延遲惡化。整小時最大 p99 僅 167.891 ms，
沒有舊 17,800 EPS pilot 中的秒級尖峰。

最後 5 分鐘 RSS 平均比最初 5 分鐘只高 0.496 MiB，整段線性估計為
每小時增加 0.387 MiB，且最高 RSS 僅 52.09 MiB。這輪沒有顯示明顯的單調
記憶體成長；但單一一小時實驗仍不足以排除緩慢洩漏。

CPU 平均為 83.84%，短暫最高 101.90%，平均 headroom 約 16%。舊輪在
更低的 17,800 EPS 反而有較高平均 CPU 與延遲；兩輪使用相同 image digest，
但仍可受主機溫度、排程與啟動相位影響，因此不應解讀為提高流量會降低 CPU，
只能說新輪在 20,000 EPS 下有更佳的實測穩定性。

### 14.3 Pilot 結論

> WCM 在新容量 80%（20,000 EPS）下連續運行一小時，完成 75,600,000
> 筆輸入並正確送達 30,240,000 筆預期輸出，沒有延遲漂移、持續
> backlog、資料錯誤、OOM 或重啟。p99 中位數為 111.665 ms，RSS 線性
> 斜率僅 +0.387 MiB/hour。本輪支持 WCM 在此工作點的一小時穩定性，
> 但仍不能取代 8 或 24 小時長期測試。

正式原始結果：
`runs/wcm_stability_20k_1h_manual_r1_20260809/S1_rep1/stability_analysis.json`。
舊 17,800 EPS pilot 仍保留於
`runs/wcm_stability_80pct_1h_20260806/S1_rep1/`，但不再作為新容量 80% 的正式摘要。

## 15. 四產品五分鐘持續過載比較

### 15.1 比較方式與完成狀態

四個產品均使用 1024 B、36 欄位 log、1 CPU、512 MiB、單一同步 TCP 來源及相同 filter 比例。每輪依序執行 warm-up 180 秒、baseline 120 秒、150% 最大吞吐量的 overload 300 秒、recovery 900 秒及 final drain 120 秒。Baseline 為各產品最大吞吐量 `M` 的 50%，overload 為 `M` 的 150%，所以每個產品承受的都是相同相對負載，而非相同絕對 EPS。

最終樣本數為WCM `n=3`、Vector `n=3`、Fluentd `n=3`、Fluent Bit WASM `n=1`。WCM、Vector與Fluentd均使用使用者人工執行的三輪；Fluent Bit WASM第二輪未執行，因此其結果只能視為單輪觀察，不能與其他產品做同等強度的變異性推論。

| 產品 | M | Baseline | Overload | 輪數 | 完成恢復 | 資料錯誤 | OOM／重啟 |
|---|---:|---:|---:|---:|---:|---:|---:|
| WCM | 22,250 EPS | 11,125 EPS | 33,375 EPS | 3 | 3／3 | 0 | 0／0 |
| Vector | 15,250 EPS | 7,625 EPS | 22,875 EPS | 3 | 3／3 | 0 | 0／0 |
| Fluentd | 50,000 EPS | 25,000 EPS | 75,000 EPS | 3 | 3／3 | 0 | 0／0 |
| Fluent Bit WASM | 5,500 EPS | 2,750 EPS | 8,250 EPS | 1 | 1／1 | 0 | 0／0 |

「資料錯誤」包含遺失、重複、內容錯誤、非預期資料及亂序；已完成各輪全部為零。

### 15.2 各輪原始結果

| 產品／輪次 | 恢復開始 | 完成 30 秒確認 | Baseline p99 | 最大 p99 | Output backlog | Source pending | CPU mean／max | RSS mean／max | Generator lag p99 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| WCM R1（人工） | 272.167 s | 302.167 s | 150.193 ms | 141,279.062 ms | 1,295,118 | 3,112,470 | 67.111／102.90% | 146.371／179.82 MiB | 133.239 s |
| WCM R2（人工） | 264.346 s | 294.346 s | 149.974 ms | 135,924.482 ms | 1,246,766 | 2,992,550 | 66.993／102.70% | 122.329／179.72 MiB | 127.837 s |
| WCM R3（人工） | 263.097 s | 293.097 s | 149.998 ms | 135,624.327 ms | 1,246,476 | 2,988,603 | 66.761／102.80% | 101.988／159.56 MiB | 127.507 s |
| Vector R1（人工） | 311.078 s | 341.078 s | 356.677 ms | 153,901.918 ms | 930,511 | 2,314,016 | 70.732／100.50% | 77.697／90.38 MiB | 150.299 s |
| Vector R2（人工） | 351.072 s | 381.072 s | 354.010 ms | 168,204.249 ms | 982,545 | 2,443,801 | 72.599／100.40% | 49.053／63.44 MiB | 164.493 s |
| Vector R3（人工） | 379.079 s | 409.079 s | 352.793 ms | 181,927.603 ms | 1,060,645 | 2,639,906 | 72.862／100.50% | 44.874／58.81 MiB | 178.162 s |
| Fluentd R1（人工） | 305.042 s | 335.042 s | 115.696 ms | 151,604.876 ms | 3,029,556 | 7,564,835 | 70.647／100.40% | 157.499／198.59 MiB | 148.761 s |
| Fluentd R2（人工） | 321.065 s | 351.065 s | 115.383 ms | 157,767.745 ms | 3,107,248 | 7,755,819 | 72.167／100.40% | 129.498／168.20 MiB | 154.838 s |
| Fluentd R3（人工） | 323.243 s | 353.243 s | 115.358 ms | 151,063.387 ms | 2,993,422 | 7,478,733 | 72.066／100.40% | 153.720／192.41 MiB | 147.989 s |
| Fluent Bit WASM R1 | 656.866 s | 686.866 s | 1,020.043 ms | 253,593.206 ms | 453,144 | 1,124,852 | 86.443／103.30% | 304.851／309.38 MiB | 248.163 s |

### 15.3 正規化彙整

不同產品的 EPS 不同，因此 raw backlog 筆數不能直接比較。Output backlog 以各產品 baseline 預期輸出 `baseline × 40%` 換算為等效秒數；source pending 則以 baseline 輸入 EPS 換算。表內時間、延遲、CPU 與記憶體均為各產品已完成輪次的算術平均；`±` 為 sample standard deviation，單輪不計算標準差。

| 產品 | n | 恢復時間平均 ± SD（範圍） | Baseline p99 | 最大 p99 | Output backlog 等效秒數 | Source pending 等效秒數 | CPU mean | RSS mean／各輪最高值 | Generator lag p99 |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| WCM | 3 | 266.537 ± 4.916 s（263.097–272.167） | 150.055 ms | 137,609.290 ms | 283.772 s | 272.468 s | 66.955% | 123.563／179.82 MiB | 129.528 s |
| Vector | 3 | 347.076 ± 34.176 s（311.078–379.079） | 354.493 ms | 168,011.257 ms | 324.995 s | 323.398 s | 72.064% | 57.208／90.38 MiB | 164.318 s |
| Fluentd | 3 | 316.450 ± 9.939 s（305.042–323.243） | 115.479 ms | 153,478.669 ms | 304.341 s | 303.992 s | 71.627% | 146.906／198.59 MiB | 150.529 s |
| Fluent Bit WASM | 1 | 656.866 s | 1,020.043 ms | 253,593.206 ms | 411.949 s | 409.037 s | 86.443% | 304.851／309.38 MiB | 248.163 s |

以目前已完成輪次觀察，WCM的平均恢復時間最短，約為Vector的76.8%、Fluentd的84.2%與Fluent Bit WASM的40.6%。WCM的正規化output backlog、source pending與CPU mean也都是四者最低。Fluentd的baseline p99最低，比WCM平均低約34.6 ms；Vector的記憶體用量最低；Fluent Bit WASM的baseline p99、恢復時間與記憶體用量在目前單輪中均最高。

### 15.4 觀察與可能原因

1. **WCM人工三輪重複性最好，但不能直接套用理論300秒。** 三輪平均為266.537秒，CV為1.84%，正規化backlog約272–284秒。恢復早於理想流體模型的300秒，與generator lag p99約129.528秒一致，表示同步sender受到反壓，實際高峰不是完整150% M矩形波。這三輪適合描述「單連線TCP反壓情境下」的穩定恢復，不適合宣稱完整接收33,375 EPS五分鐘。

2. **Fluentd絕對吞吐量最高，人工三輪相對恢復排名第二。** Fluentd的M為50,000 EPS，明顯高於其他產品，baseline p99也最低。人工三輪恢復為305.042、321.065與323.243秒，平均316.450秒、CV 3.14%，顯著比先前舊輪次穩定。其正規化backlog約304秒，接近理想模型；但平均CPU與記憶體均高於WCM，恢復仍比WCM人工三輪平均慢約49.913秒。

3. **Vector記憶體效率最好，但人工三輪仍有逐輪變慢。** 三輪最高RSS為58.81–90.38 MiB，低於其他產品；恢復時間由311.078增至351.072及379.079秒，並伴隨generator lag、source pending與最大p99同步上升。三輪CV為9.85%，表示其恢復結果仍受sender backpressure相位、batching／flush邊界、主機狀態或單worker排空能力影響，但變異已遠小於先前代理執行的舊輪次。

4. **Fluent Bit WASM 的固定批次等待與 WASM 記憶體成本明顯。** Baseline p99 約 1.02 秒，與 Fluent Bit 原生約一秒 flush cadence 相符；其 RSS 約 305–309 MiB，也受到預留 WASM heap 與 runtime 的影響，不能把全部 RSS 解讀成 backlog。單輪恢復 656.866 秒，且正規化 backlog 約 409–412 秒，但因第二輪依照硬體降溫需求取消，目前不能評估其重複性。

5. **所有產品均保住資料正確性且沒有 OOM。** 即使在五分鐘持續過載下，所有已完成輪次仍是零遺失、零重複、零內容錯誤、零亂序、零 OOM／重啟。這支持四者都能透過 buffer 或 TCP backpressure 保護資料；差異主要落在恢復速度、延遲、CPU 與記憶體成本。

6. **最大 p99 主要反映排隊時間。** 過載後四者最大 p99 都達 146–268 秒，與 generator lag 及 backlog 同方向變化。這些值不是單筆解析本身需要數分鐘，而是事件在來源、socket、內部 queue 或 output batch 中等待的端到端時間。

### 15.5 公平性限制與論文表述

本實驗在資源限制、資料格式、相對負載比例、階段時間與驗證方法上對齊，因此適合比較「各產品承受自身最大吞吐量 150% 壓力後，回到自身 baseline 的能力」。它不適合直接宣稱某產品在相同絕對 EPS 下較快，因為四者的 M、baseline EPS 與內部 batch 實作不同。

最重要的限制是單一同步TCP generator。各產品的generator lag p99平均為129.528–248.163秒，代表高峰期間sender被DUT反壓，實際進入DUT的流量不是完全固定的150% M矩形波。產品或輪次間的恢復差異同時包含DUT行為與sender backpressure相位；Vector人工三輪的恢復與lag同步上升，而WCM與Fluentd人工三輪的變異較小。因此論文宜使用「在單連線TCP反壓情境下觀察到」的措辭，不宜把平均值解讀為純粹的內部排空速率。

目前樣本數仍小且不平衡：WCM、Vector與Fluentd `n=3`，Fluent Bit WASM `n=1`。可報告描述統計、範圍與觀察趨勢，但不能宣稱統計顯著或建立產品總排名。若要加強因果解釋，後續應在主機完全降溫後隨機化執行順序，Fluent Bit WASM補足至少三輪，並增加非同步／多連線open-loop generator對照，以分離DUT capacity與sender backpressure。

### 15.6 本節原始結果

- WCM（使用者人工三輪）：`runs/wcm_overload_manual_r1/`、`runs/wcm_overload_manual_r2/`、`runs/wcm_overload_manual_r3/`
- Vector（使用者人工三輪）：`runs/vector_overload_manual_r1/`、`runs/vector_overload_manual_r2/`、`runs/vector_overload_manual_r3/`
- Fluentd（使用者人工三輪）：`runs/fluentd_overload_manual_r1/`、`runs/fluentd_overload_manual_r2/`、`runs/fluentd_overload_manual_r3/`
- Fluent Bit WASM：`runs/fbit_wasm_overload_150pct_5m_r1_20260808/`
