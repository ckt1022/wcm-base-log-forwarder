# pattern_suite 分析：五個設計模式的證據（2026-10-14／15）

> 範圍：`test/compare/pattern_suite/`（Phase 0／1／2，共 208 + 1 個量測單位）；輔助資料：`../streaming_validation/`。
> 本文件只分析既有資料，**沒有重跑任何量測**。數字可用 `python3 collect_suite.py runs && python3 analyze_suite.py` 重算，輸出在 `analysis/`，圖在 `analysis/charts/`。
> 數字慣例：「中位數 [最小–最大]（n）」。容量 = 搜尋到的最高 PASS 速率（單次 60 s trial，解析度約 5%）。

## 0. 資料品質：先讀這節

### 0.1 時鐘階梯：舊的延遲數字大多被污染，已全部重新校正
- 2026-10-15 **04:04 之後**，WSL2 的 wall clock 改為**向後**跳（D = wall − monotonic 每 ~30 s 減少一次）。04:04–06:15 每次 −0.4～−0.9 s；09:26 host 恢復後每次 −1.1～−2.7 s。之前的 session（batch_alignment、streaming_validation）是向前跳。
- `../batch_alignment/correct_latency.py` 假設階梯向前，以 receive 的 wall 時間查 D。向後跳時，階梯前最後 |step| 秒收到的記錄會被套上階梯後的 D，延遲多算 |step|，約佔記錄的 1.5–8%。
  - 因此 `latency_stats_30_150.txt`、`run_suite.log`、`analysis/latency.csv` 中 04:04 之後的 p99 **不可使用**。
  - 96 個延遲 run 中有 69 個帶向後階梯，其中 51 個有效 run 的舊 p99 偏差 > 20 ms。
  - 例：WCM-stream W=0.1 r3 1,787.8 → 135.5 ms；FB Lua W=0.01 r3 915.3 → 31.3 ms；Fluentd W=0.01 rerun 2,006.8 → 57.8–66.5 ms。
- 新的校正程式 **`fix_latency.py`**（只讀原始資料）：
  1. 階梯的大小與時間取自各 run 的 `clock_drift.csv`（logger 每 50 ms 輪詢一次）。
  2. 落在 wall 重疊區的 POST（同一 receive 時間的所有記錄）一起判定：若用階梯前的 D 校正會出現 < −50 ms 的延遲，就歸為階梯後。
  3. gen 啟動時刻也可能落在重疊區（2 個 run：WCM-batch W=1 r1、WCM-stream W=1 r2）。這時兩個 d0 候選都跑一次完整指派，在負延遲 ≤ 0.01% 的候選中選中位數較小者。
  4. 30–150 s 窗口與子窗都在 monotonic 軸上計算。
- **驗證**：
  - (a) 在 38 個向前階梯的 PASS run 上，新舊兩支程式的 p99 最大只差 0.77 ms（`analysis/fix_latency_validation.csv`）。
  - (b) 同一設定下，r1（04:04 前、無階梯）與 r2／r3（有階梯）校正後一致。例：FB Lua W=0.1 為 117.4／117.3／117.0 ms；W=0.01 為 31.5／31.5／31.3 ms。
  - (c) 每秒第 5 小延遲的 5-bin 中位數，在 30–150 s 內的變動範圍：所有有效 run ≤ 89 ms。唯一例外是 FB WASM 4,000 EPS W=1 r3（1,223 ms，見 0.4）。
- **CPU** 也重算：`stats.csv` 的 elapsed 是 wall 差，向後階梯時會出現負差，因此改用去除階梯後的取樣時間計算 Δcpu_time／Δt。
- `stepdetect.py` 只處理向上階梯；本次每個延遲 run 都有 `clock_drift.csv`，所以沒有用到。

### 0.2 Fluentd 的計時器會被向後階梯「拖住」（真實的 DUT 行為）
- Fluentd 的 flush thread 以 wall clock 計時。wall 往回跳 1.9 s 之後，約 1.9 s 內完全沒有 POST，接著一次送出約 3,900 筆（`fluentd/lat_rate_5000_m3_W0.01_r3` 第 128 s）。這些記錄的真實延遲確實約 2 s。
- FB、Vector、WCM 用 monotonic 計時，不受影響。
- 因此延遲同時報兩個值：
  - `p99`：完整校正後的值。
  - `p99s`：再排除每個向後階梯之後 0–3 s 收到的記錄。
- 對 FB Lua／Vector／FB WASM／WCM，兩者相差 ≤ 10 ms。只有 Fluentd 的 6 個 run 相差 > 20 ms：
  - W=0.01 r3：734.8 → 66.5
  - W=0.1 r3：660.5 → 127.8
  - W=0.5 r3：1,134 → 523
  - W=1 r1／r2／r3：1,212／1,515／2,470 → 1,025–1,032
- Fluentd 在無階梯的 r1（W=0.01、0.1）為 57.8／128.1 ms，與 p99s 一致。因此 **p99s 才是公平值**，圖 a1–a3 都用 p99s。

### 0.3 Host 暫停與 r3 的恢復後狀態
- 暫停時間：`fluentd/lat_rate_5000_m3_W0.01_r3_INVALID_host_paused` 的 clock_drift 顯示 wall 在 **06:18:21 → 09:26:38** 之間分三段跳過。該 run 已排除，由 14:51–14:54 的 rerun 取代。
- 是否還有其他 run 受影響：`analysis/run_validity.csv` 對 `logs/runs.log` 的 324 筆逐一檢查，只有這一筆與暫停重疊（耗時 11,465 s）。
  - 前一個 run（FB Lua W=0.01 r3）的 gen 在 06:15:19 結束，clock_drift 也沒有大跳躍。
  - correctness／fault 全部在 03:32 前完成。
  - **沒有其他受污染的 run。**
- 09:26 之後機器狀態不同：
  - (a) guest 時鐘比 host 快 5–8%；cap trial 的 wall 耗時從約 98 s 變成約 83 s。
  - (b) 同設定、同 5,000 EPS 下，CPU 上升幅度依產品不同：WCM-batch W=0.1 +34%（29.6／29.9 → 39.9%）、Vector +22%、WCM-stream +19%、FB Lua +15%、Fluentd +5%。
- r3 延遲校正後與 r1／r2 大致一致（只有 WCM-batch W≥0.5 的 r3 高 50–130 ms），所以保留 3 reps。但有兩個限制：
  - **Phase 2 的全部容量（11:12–14:49）都在恢復之後量測**，不能和 Phase 0／P2 或 batch_alignment 的絕對值比較。
  - CPU 絕對值即使在同一夜也會漂移（WCM-stream k=64 W=0.1：streaming_validation 29%，本套件 40%），只引用同時段配對的差值。

### 0.4 其他處理
| 項目 | 處理 |
|---|---|
| FB WASM 5,000 EPS | 12／12 FAIL：gen 只送出 4,241–4,632/s，CPU 95–100%。p99 11.8–23 s 是真實積壓，不是時鐘污染（W=0.01 r1 的 17,702 ms 發生在無階梯時段）。W≥0.1 的 4,000 EPS × 3 全部 PASS，**FB WASM 的延遲點用 4,000 EPS**。W=0.01 沒有備援，因此沒有有效延遲。 |
| FB WASM 4,000 EPS W=1 r3 | 最後 60 s 延遲持續上升（子窗 p99 1,027 → 2,157 ms，CPU 90%），是接近容量時的真實漂移。保留，報中位數 1,032 ms。 |
| P2 k=1 | 見 §1。延遲 run 為 FAIL（超載），不列入延遲統計。 |
| 容量 `unbracketed` | k=1 的 3 次搜尋從 16,125 一路降到下限 4,500 都 FAIL，collect 記成 0。實際是 < 4,500；飽和吞吐另外計算。 |

## 1. Amortized Boundary Crossing（攤提跨邊界的固定成本）
**主張**：每次跨越 plugin 邊界都有固定成本，把多筆記錄合成一次呼叫就能攤掉。批量太小時容量崩潰，批量到某個程度後就接近上限。

**證據**（P2：streaming、W=0.1；`analysis/p2_k_sweep.csv`、圖 `b_capacity_vs_k.svg`）
| k | 容量（3 reps） | 中位數 [範圍] | 5,000 EPS（r1）p99／CPU |
|---:|---|---:|---|
| 1 | < 4,500 ×3（飽和吞吐 ≈ **3,085 EPS**，CPU 99.9%） | — | FAIL：gen 只送出 3,136/s，lag p99 59.8 s |
| 16 | 15,750（00:53）／17,500（01:22）／14,750（02:00） | 15,750 [14,750–17,500] | 126.9 ms／46.4% |
| 64 | 21,500（23:09）／21,500（01:30）／18,500（02:10） | 21,500 [18,500–21,500] | 132.7 ms／40.3% |
| 256 | 25,000（23:16）／25,000（01:37）／20,000（02:18） | 25,000 [20,000–25,000] | 140.9 ms（3 reps：135–141）／36.0% |

- **k=1 的崩潰是真的**：
  - 11 個速率 × 3 次搜尋全部 FAIL，原因是 gen 被擋（avg 3,805–4,175/s）且 backlog > 3×rate。
  - sink 每秒輸出中位數 1,122 筆，對應的處理量約 2,800–3,100 EPS；CPU 99.9%。
  - 「容量 0」只是 unbracketed 被記成 0；「p99 64 s」是超載下積壓的排隊時間。
- **配對比較**：每一輪都是 k256 ≥ k64（×1.16／1.16／1.08）。第 3 輪三個 k 同時下降，屬於共同漂移。
- **成本模型**：以 t = b + a／k 擬合，得到 a ≈ **0.28–0.39 ms／micro-batch**、b ≈ 39–42 µs／筆。
  - 對照 `../../ours/per_record_invocation/`：Component Model 呼叫本身的 a ≈ 0.85–1.6 µs，k=1 子呼叫只讓容量 −16.5%。
  - 兩者差兩個數量級，所以主宰項是 host 端每個 micro-batch 的管線成本（推論，未 profile）。
- **輕載代價**：5,000 EPS 時每個 micro-batch 只有約 25 筆。WCM-stream 比 batch 多用約 6 pp CPU（36.0 vs 29.6–29.9%，同時段配對），大約等於每秒 200 批 × 0.3 ms。

**Consequences／Evidence**
- *Benefit*：在 1 CPU 容器中，把批量從 1 提高到 256，WCM 的可持續吞吐從約 3,100 EPS 提高到 25,000 EPS，約 8 倍。k=16／64／256 的中位數為 15,750／21,500／25,000 EPS，各 3 次。延遲不受批量影響：5,000 EPS、W=0.1 s 時 p99 為 127–141 ms。
- *Liability*：只有在「有記錄可等」時才湊得出大批次。低負載時 micro-batch 只有約 25 筆，CPU 比整批模式高約 20%（36.0% vs 29.6–29.9%）。k=1 會崩潰到約 3,100 EPS（CPU 100%）。批量越大，一筆 poison record 牽連的記錄越多，必須搭配 Poison Record Bisection。

**仍薄弱**：每個 k 只有 3 次單一 trial，session 內變異達 20%；k ∈ (1,16) 沒有量；固定成本沒有 profile；k 對延遲的影響只有 1 rep（k=256 有 3 reps）。

## 2. Streaming Micro-Batch（邊收邊處理，只緩衝輸出）
**主張**：batch 模式先累積 W 秒的原始輸入再整批處理，延遲 ≈ W + 整批處理時間。streaming 每 ≤ k 筆或 5 ms 就處理一次，W 只控制 flush，因此超額延遲固定、記憶體不隨 W 成長、容量不下降。

**A. 超額延遲**（excess = p99s − 0.99·W，5,000 EPS × 3 reps；`analysis/m3_tradeoff.csv`、圖 a2）
| W | WCM-batch excess | WCM-stream excess | batch p50 | stream p50 |
|---:|---|---|---:|---:|
| 0.01 s | 38.5 [25.8–38.5]（p99 48.4） | 46.5 [33.9–47.9]（p99 56.4） | 21.1 | 28.2 |
| 0.1 s | 69.5 [69.0–70.9]（p99 168.5） | 41.3 [36.2–41.9]（p99 140.3） | 89.4 | 75.6 |
| 0.5 s | 166.1 [158.9–217.0]（p99 661.1） | 30.5 [26.0–32.3]（p99 525.5） | 390.4 | 277.9 |
| 1 s | 292.2 [285.2–420.8]（p99 1,282.2） | 29.3 [24.9–35.4]（p99 1,019.3） | 768.5 | 530.0 |

- 輔助資料：streaming_validation（k=64）在 W=0.01／0.1 的 p99 為 32.4／129.6 ms；Phase 0 的 k=64 在 W=0.5／1 為 514.6／1,010.6 ms。

**B. 記憶體**：5,000 EPS 的 RSS，batch 為 19.7／24.0／49.3／83.8 MiB，stream 為 15.8／15.9／18.2／22.1 MiB。容量搜尋時，batch 在 W=0.5（24,250）和 W=1（19,250）出現 `oom=true`；streaming 的 FAIL 全部是 CPU 不足（gen 被擋），沒有 OOM。

**C. 容量**
| 比較 | batch | stream k=256 | 比值 |
|---|---:|---:|---:|
| Phase 0，同一 session，W=0.1 | 21,750 | 25,000（k=64：21,500） | **1.149** |
| W=0.1 全部 reps | 20,500 [20,000–21,750]（r2／r3 在恢復後） | 25,000 [20,000–25,000]（恢復前） | regime 不同 |
| Phase 2 同 regime，W=0.01 | 13,500 | 13,250 | 0.98 |
| W=0.5 | 22,750（24,250 OOM） | 25,000 | 1.10 |
| W=1 | 18,750（19,250 OOM） | 20,000 | 1.07 |

**D. 正確性**：P1a stream 15／15 PASS；streaming_validation C1–C4 × 3 = 12／12 PASS（C2：400,000 筆 @17,500 EPS；C3：114,000／114,000）。

**Consequences／Evidence**
- *Benefit*：在相同 flush 視窗下，streaming 讓 W = 0.1／0.5／1 s 的超額延遲固定在 41／31／29 ms（5,000 EPS、各 3 次），舊模式為 69／166／292 ms。W = 1 s 的 p99 從 1,282 降到 1,019 ms，與 FB Lua（1,017 ms）相同。RSS 不隨 W 成長（W=1：22 vs 84 MiB）。容量不下降：同一 session W=0.1 為 25,000 vs 21,750 EPS；W=0.5／1 分別 +10%／+7%。
- *Liability*：低負載時 CPU 高約 20–25%（W ≥ 0.1、5,000 EPS：35–36% vs 28–30%）。W = 0.01 s 時沒有優勢：p99 56 vs 48 ms，p50 28 vs 21 ms，容量 13,250 vs 13,500。flush 是全域週期，記錄在 buffer 中停留 [0, W)。k 必須夠大。

**仍薄弱**：W=0.1 的容量只有 1 組同時段配對；Phase 2 的容量只有 1 rep 且都在恢復後；CPU 差值受 session 漂移影響；沒有高負載下的延遲曲線。

## 3. Watchdog and Respawn（每次呼叫設 epoch deadline，trap 後丟棄 instance）
**主張**：沒有 watchdog 時，卡死的 plugin 會讓 stage 永久停住。用 epoch deadline 中斷呼叫、丟棄被污染的 instance、再交給二分隔離，就能在有界時間內恢復，只遺失一筆，常態開銷也有限。

**證據**（`p1_poison_recovery_vs_timeout.csv`、`p4_epoch_reuse.csv`、`p4_watchdog_timeline.csv`、`p4_fault_mem.csv`；圖 d、e、f）
| 情境（stream k=256，原生執行，FAULT_LOOP 於 t=15 s 注入） | reps | host | 遺失 | 0 吞吐秒數 | 恢復時間 | 隔離呼叫次數 |
|---|---:|---|---:|---:|---:|---:|
| timeout 1 s | 3 | 3／3 存活 | 1／1／1 | 3／3／3 | **4 s** | 12 |
| timeout 2 s | 3 | 3／3 | 1／1／1 | 7／7／7 | **8 s** | 12 |
| timeout 5 s | 3 | 3／3 | 1／1／1 | 18／18／18 | **20 s** | 12 |
| timeout 30 s | 3 | 3／3 | 1／1／1 | 118／118／118 | **119 s** | 12 |
| watchdog 關閉 | 1 | 存活，但 parse 永久停住 | 31,700 筆未送達 | 59 s 直到結束 | **不恢復**，CPU ≈ 98% | 0 |
| reuse 關閉，timeout 2 s | 3 | 3／3 | 1／1／1 | 7／7／7 | 8 s | 12 |
| FAULT_MEM（batch 模式） | 3 | 3／3 | 1／1／1 | 0 | ≤ 1 s | 20 |

- 恢復時間 = **4.0 × T**（斜率 3.97；1 s 解析度下 3 reps 相同）。組成是整批重試 3×T，加上二分探測合計約 T（每次探測的 deadline 依子區段長度縮放）。
- 常態開銷 @15,000 EPS、3 reps（02:41–03:14，無時鐘階梯）：
  - CPU：epoch 開 83.9% [83.8–84.3]，關 77.0% [76.8–78.4] → **+6.9 pp（8.2%）**。
  - p99：173 [166–181] vs 161 [156–172] ms，差距在 rep 變異之內。
- reuse 關閉：CPU 90.6% [90.2–91.3]（+6.7 pp），p99 193 [167–199] ms。reuse 只影響 format／transport，而 FAULT_LOOP 打在 parse，所以恢復時間相同（8 s）。
- FAULT_MEM：baseline RSS 41.4 MiB，注入後峰值 135.6／76.1／95.5 MiB（1 s 取樣，變異大）。plugin 回報 `memory allocation of 33554432 bytes failed` 後 trap，二分隔離後只遺失 1 筆。

**Consequences／Evidence**
- *Benefit*：沒有 watchdog 時，一筆無窮迴圈記錄讓 parse stage 在剩下的 59 s 吞吐為 0（CPU 約 98%，31,700 筆未送達）。有 watchdog 時，host 3／3 次存活、只遺失那一筆，恢復時間約 4 × T（T = 1 s → 4 s，T = 30 s → 119 s，各 3 次）。記憶體炸彈在 ≤ 1 s 內恢復。
- *Liability*：epoch 的常態成本為 15,000 EPS 時 CPU +6.9 pp（8.2%）。若常態就每次重建 instance（reuse 關閉），CPU 再加 6.7 pp。恢復時間約為 4T，T 太大會停擺很久（T = 30 s 時停 118 s），太小可能誤殺慢但正常的批次，所以 T 必須依最大批次處理時間調整。

**仍薄弱**：watchdog 關閉的消融只有 1 rep；epoch 開銷只量一個負載點；恢復時間解析度 1 s；沒有量「不重試、直接二分」；reuse 消融沒有打到 format／transport。

## 4. Poison Record Bisection（批次失敗時遞迴二分）
**主張**：plugin 對整批回傳錯誤時，host 把批次二分、只重送出問題的那一半，最後只拒絕壞記錄。沒有錯誤時成本為零；錯誤稀疏時，每筆壞記錄約需 2·log₂(n) 次呼叫。

**證據**（C3 型，2,500 EPS、100,000 筆，3 reps；`p1_bisection.csv`、圖 c1–c3）
| malformed（筆數） | 模式 | PASS | delivered | 隔離呼叫 | 呼叫／壞記錄 | CPU | ΔCPU | RSS max |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| 0% | batch | 3／3 | 100% | 0 | — | 14.2 [14.1–14.5] | — | 17.2 |
| 0.1%（122） | batch | 3／3 | 100% | 1,886 [1,878–1,890] | 15.5 | 15.4 [15.3–15.8] | +1.3 | 17.0 |
| 1%（1,090） | batch | 3／3 | 100% | 13,618 [13,594–13,630] | 12.5 | 18.9 [18.7–19.3] | +4.8 | 17.7 |
| 5%（5,176） | batch | 3／3 | 100% | 45,914 [45,782–46,036] | 8.9 | 19.7 [19.2–20.0] | +5.6 | 21.9 |
| 20%（19,972） | batch | 3／3 | 100% | 110,762 [110,762–110,824] | 5.5 | 17.5 [17.3–18.1] | +3.3 | 36.0 |
| 0% | stream | 3／3 | 100% | 0 | — | 18.2 [16.4–18.4] | — | 14.0 |
| 0.1% | stream | 3／3 | 100% | 1,388 [1,382–1,398] | 11.4 | 18.7 [17.6–19.1] | +0.5 | 14.1 |
| 1% | stream | 3／3 | 100% | 11,452 [11,442–11,454] | 10.5 | 20.8 [18.6–21.1] | +2.6 | 14.9 |
| 5% | stream | 3／3 | 100% | 43,290 [43,236–43,366] | 8.4 | 23.3 [23.1–23.9] | +5.1 | 19.0 |
| 20% | stream | 3／3 | 100% | 109,200 [108,928–109,208] | 5.5 | 22.3 [22.0–22.4] | +4.1 | 33.9 |

- 30／30 run 的 missing／dup／corrupt／reorder 都是 0，rejected 行數恰好等於 malformed 數。
- batch 每批約 250 筆，0.1% 時每筆壞記錄 15.5 次呼叫，與 2·log₂250 ≈ 16 相符。
- 壞記錄越密，越多上層探測可以共用，所以呼叫／壞記錄下降到 5.5。stream 的批次小（約 12 筆），稀疏時呼叫較少，但牽涉的批次較多（20% 時 1,936 vs 401 批）。
- 20% 時 CPU 比 5% 低，因為送達的記錄少 16%，下游工作量隨之下降。
- trap 型的 poison 見 §3：100 筆的批次用 12 次呼叫隔離，耗時約 4T。

**Consequences／Evidence**
- *Benefit*：malformed 從 0.1% 到 20%，所有合法記錄都送達（30／30 次），被拒絕的恰好是那些 malformed 記錄。沒有錯誤時不多呼叫任何一次。錯誤稀疏時每筆壞記錄約 2·log₂(批量) 次呼叫（批量 250 時為 15.5 次）；錯誤越密攤得越低（20% 時 5.5 次）。
- *Liability*：成本與壞記錄數成正比。2,500 EPS 時 CPU 最多 +5.6 pp（約 +40%）；20% 時 RSS 峰值約 2 倍（17 → 36 MiB）。若 plugin 以 trap／逾時失敗，每次探測都可能耗掉一個 timeout，因此需要呼叫次數與深度上限（實測 12 次、約 4×T）。長期最乾淨的做法是 per-record 的 WIT 結果型別，但要改所有 plugin。

**仍薄弱**：只在 2,500 EPS 量；沒有 malformed 負載下的容量與 p99；沒有其他錯誤分布（例如壞記錄集中成一段）。

## 5. Capability-Scoped Stage（每個 stage 各自連結 WASI，載入期檢查 import）
**主張**：每個 stage 只連結被授權的 WASI 介面。未授權的 import 在載入期就被拒絕；授權為 inert 時 plugin 可以載入，但存取在執行期被拒絕。檢查本身的成本可以忽略。

**證據**（`p5_capability_probes.csv`、`p5_loadcheck_summary.csv`、圖 g）
| 探測 | parse 的授權 | 擋下時機 | 結果（3 reps） | 遺失 | 證據 |
|---|---|---|---|---:|---|
| FAULT_NET | sockets: deny（只授權 fs: inert） | 載入期 | REFUSED_AT_LOAD ×3（exit 1，0.26–0.32 s） | 未啟動 | 7 個 `wasi:sockets/*` import |
| FAULT_FILE／ENV | 全部 deny | 載入期 | REFUSED_AT_LOAD ×3 | 未啟動 | 9 個 import（fs 2 + sockets 7） |
| FAULT_NET | fs: inert + sockets: inert | 執行期 | DENIED ×3 | 0 | `connect 127.0.0.1:8080 failed: PermissionDenied`（8080 有在 listen） |
| FAULT_FILE | 同上 | 執行期 | DENIED ×3 | 0 | `failed to find a pre-opened file descriptor`（讀 /etc/passwd） |
| FAULT_ENV | 同上 | 執行期 | DENIED(empty) ×3 | 0 | vars=0 bytes=0 |

- 檢查成本（26 個 plugin × 2 種授權 × 3 reps）：`check_ms` 中位數 0.008–0.009 ms，最大 0.031 ms；compile + link + instantiate_pre 為 25–1,497 ms。檢查佔載入時間 ≤ **0.031%**。
- 預設 deny 時，26 個 plugin 只有 6 個能載入，多數被拒是因為工具鏈連帶 import 了 filesystem／sockets。授權 inert 後 24 個可以載入；剩下的 2 個中，`parser_c_bad` 是故意做壞的，`parser_c_json` 連 compile 都失敗。
- 執行期被拒的 run：host 存活、0 遺失，CPU 15.4–16.6%。

**Consequences／Evidence**
- *Benefit*：parse stage 預設拒絕網路與檔案系統。帶 socket import 的 plugin 3／3 次在載入期被拒（約 0.3 s 內 exit 1，並列出每個未授權 import）。授權 inert 時，連線、讀檔、讀環境變數在執行期被拒，host 不受影響、0 遺失。檢查 ≤ 0.031 ms，不到載入時間的 0.031%。
- *Liability*：多數工具鏈（C／Go／C#）的產物會連帶 import filesystem／sockets，所以預設 deny 時 26 個 plugin 有 20 個無法載入，運維者必須逐一明確授權 inert。授權 inert 後錯誤延到執行期才出現，plugin 必須能處理 PermissionDenied。

**仍薄弱**：只測 parse stage；env 的「拒絕」其實是看到空環境；沒有比較 deny 與 inert 的執行期開銷；沒有測授權過寬的情況；檢查是靜態的，看不到細粒度權限。

## 6. 五產品公平比較（方法 3）
**5,000 EPS 的 p99s（ms，3 reps 中位數 [範圍]；FB WASM 為 4,000 EPS）**
| 產品 | W=0.01 | W=0.1 | W=0.5 | W=1 |
|---|---|---|---|---|
| FB Lua | **31.5** [31.3–31.5] | **117.3** [117.0–117.4] | **516.9** [515.9–517.8] | **1,016.6** [1,016.6–1,023.9] |
| Fluentd | 57.8 [57.7–66.5] | 128.1 [127.8–129.8] | 521.3 [521.1–522.6] | 1,025.3 [1,024.8–1,032.1] ¹ |
| Vector | 85.7 [56.7–87.7] | 163.3 [140.2–163.4] | 558.9 [557.2–567.5] | 1,084.9 [1,070.9–1,101.5] |
| FB WASM | FAIL ×3 | 143.3 [142.4–145.4] @4k | 527.5 [525.6–531.6] @4k | 1,032.0 [1,027.7–1,492.6] @4k |
| WCM-batch | 48.4 [35.7–48.4] | 168.5 [168.0–169.9] | 661.1 [653.9–712.0] | 1,282.2 [1,275.2–1,410.8] |
| WCM-stream | 56.4 [43.8–57.8] | 140.3 [135.2–140.9] | 525.5 [521.0–527.3] | 1,019.3 [1,014.9–1,025.4] |

¹ Fluentd 只做時鐘校正、不排除階梯後 3 s 的 p99 為 1,212／1,515／2,470 ms。

**容量（EPS；Phase 2 全在恢復後；W=0.1 為 3 reps，其餘 1 rep）**
| 產品 | W=0.01 | W=0.1 | W=0.5 | W=1 |
|---|---:|---:|---:|---:|
| FB Lua | 39,000 | 58,500 [55,500–58,500] | 67,500 | 64,500 |
| Fluentd | 20,500 | 36,500 [36,500–41,500] | 38,000 | 45,000 |
| WCM-stream | 13,250 | 25,000 [20,000–25,000] ² | 25,000 | 20,000 |
| WCM-batch | 13,500 | 20,500 [20,000–21,750] ³ | 22,750 | 18,750 |
| Vector | 11,750 | 11,750 [11,750–14,250] | 14,250 | 13,750 |
| FB WASM | 4,000 | 4,400 [4,200–5,000] | 5,200 | 5,200 |

² 恢復前（P2）。³ r1 在恢復前，r2／r3 在恢復後。

**公平結論**
> 把時間視窗 W 對齊後（1 KiB／36 欄位、1 CPU／512 MiB、5,000 EPS），延遲主要由 W 決定；差別在視窗之外的超額延遲，以及容量。
> **WCM-stream** 在 W ≥ 0.5 s 時與 FB Lua、Fluentd 相差 ≤ 9 ms（525／1,019 vs 517–521／1,017–1,025 ms）。W=0.1 時多約 23 ms（140 vs 117），W=0.01 時多約 25 ms（56 vs 31）。W ≥ 0.1 時比 Vector 低 23–66 ms。舊的 **WCM-batch** 在 W ≥ 0.1 時延遲最高，W=1 時多約 265 ms，因為它要等視窗關閉才整批處理。
> 容量方面（同一 regime、單次搜尋），兩種 WCM 都排第三：落後 FB Lua（約 2.3–3.2×）和 Fluentd（約 1.5–2.3×），領先 Vector（約 1.1–2.1×）和 FB WASM（約 3.3–5.7×）。在同樣以 WASM 執行使用者程式碼的系統中，WCM 的容量約為 FB WASM 的 5 倍，而 FB WASM 在 5,000 EPS 下 12／12 FAIL。
> **stream vs batch**：W ≥ 0.1 時 streaming 的 p99 較低（−28／−136／−263 ms），容量相同或較高（W=0.5／1 為 +10%／+7%，同一 session 的 W=0.1 為 +15%），也不會 OOM。W=0.01 時兩者相當（p99 +8 ms，容量 −2%）。

**與前一 session 比較**：batch_alignment 的 WCM（tw128）在 W=1 的 p99 為 1,206 ms，本次 batch 為 1,282 ms。FB Lua／Fluentd 的 W=0.01 容量前次為 57,500／30,000，本次為 39,000／20,500。跨 session 的絕對容量不可比，但兩次的排名相同。

## 7. streaming 該不該成為預設？
**建議：是。** 預設 `processing_mode: streaming`、`micro_batch_records: 256`（不小於 64）、`micro_batch_max_wait_ms: 5`，保留 `batch` 作為選項。

**支持的數字**
1. W ≥ 0.1 時 p99 全部下降：168→140、661→526、1,282→1,019 ms（各 3 reps，範圍不重疊）。超額延遲不再隨 W 成長：29–41 vs 69–292 ms。
2. RSS 不隨 W 成長（W=1：22 vs 84 MiB）；batch 在 W=0.5／1 接近容量時會 OOM。
3. 容量不低於 batch：同一 session +15%；恢復後 W=0.5／1 +10%／+7%；W=0.01 −2%，在搜尋解析度之內。
4. 正確性 15／15 + 12／12 PASS；trap 隔離與恢復本來就是在 streaming 下量的。

**代價與保留條件**
- 輕載時 CPU 多約 20–25%。提高 `micro_batch_max_wait_ms` 可能改善，但沒有量測。
- W=0.01 時沒有好處（p99 +8 ms、p50 +7 ms）。
- **k 不可小於 16，建議 ≥ 64**；最好在程式中檢查 k 的下限或發出警告。
- 改預設之前，建議在同一 session 交錯跑 batch／stream 的 W=0.1 容量各 3 次。

## 8. 仍然薄弱或缺少的部分
1. 容量：單次 trial、約 5% 解析度，只有 W=0.1 有 3 reps，session 內變異 20%。Phase 2 全在恢復後量測（CPU +5～35%、guest 時鐘快 5–8%）。
2. 向後階梯的校正方法是新的：在向前階梯資料上 ≤ 0.77 ms，r1 與 r2／r3 也一致，但沒有獨立的 ground truth。Fluentd 的 p99s 是以排除法估計的。
3. FB WASM：沒有 5k EPS 的有效點，W=0.01 沒有任何延遲，延遲點是 4k EPS。
4. Watchdog：關閉 watchdog 的消融只有 1 rep；epoch 開銷只量一個負載點；恢復時間解析度 1 s；reuse 消融沒有打到 format／transport。
5. Bisection：只量 2,500 EPS；沒有 malformed 負載下的容量與 p99。
6. Capability：只測 parse stage；env 被拒其實是空環境；沒有量 deny 與 inert 的執行期開銷。
7. Boundary：每個 micro-batch 約 0.3 ms 的固定成本沒有 profile；k ∈ (1,16) 沒有量。
8. CPU 絕對值跨 session 會漂移，只能引用同時段配對的差值。

## 9. 檔案
| 檔案 | 內容 |
|---|---|
| `fix_latency.py` | 時鐘階梯校正（向前與向後） |
| `analyze_suite.py` | 產生下列 CSV 與圖 |
| `analysis/latency_runs_corrected.csv` | 每個延遲 run：p50／p99／p99.9、p99s、舊 p99、raw p99、階梯數、CPU、valid 與排除原因 |
| `analysis/run_validity.csv` | 暫停重疊與耗時檢查 |
| `analysis/capacity_searches.csv` | 每次搜尋（時間、regime、trial 序列、k=1 飽和吞吐） |
| `analysis/m3_tradeoff.csv` | 6 產品 × 4 W 的延遲、CPU、RSS、容量 |
| `analysis/p2_k_sweep.csv`、`p1_bisection.csv`、`p1_poison_recovery_vs_timeout.csv` | P2／P1 |
| `analysis/p4_epoch_reuse.csv`、`p4_watchdog_timeline.csv`、`p4_fault_mem.csv` | P4 |
| `analysis/p5_capability_probes.csv`、`p5_loadcheck_summary.csv` | P5 |
| `analysis/fix_latency_validation.csv` | 校正方法交叉驗證 |
| `analysis/{latency,capacity,correctness,fault,loadcheck}.csv` | `collect_suite.py` 的原始整理（latency.csv 在 04:04 之後的 p99 不可用） |
| `analysis/charts/a1_*.svg`、`a2_*.svg`、`a3_*.svg` | (a) 取捨、超額延遲、p99 vs W |
| `analysis/charts/b_capacity_vs_k.svg` | (b) |
| `analysis/charts/c1_*.svg`、`c2_*.svg`、`c3_*.svg` | (c) |
| `analysis/charts/d_poison_recovery_vs_timeout.svg` | (d) |
| `analysis/charts/e_epoch_reuse_cpu.svg` | (e) |
| `analysis/charts/f_watchdog_timeline.svg` | (f) |
| `analysis/charts/g_capability_probe_table.svg` | (g) |
