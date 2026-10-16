# 批次對齊（batch alignment）延遲比較：方法 1、2、3

工作負載：1024 B／36 欄位 JSON，filter 保留 warn+error（40%）。DUT 容器 1 CPU／512 MiB（無 swap），單一 TCP 連線，HTTP sink。
延遲 = sink 收到的 wall time − 記錄內 generator 時戳；統計窗 = 第一筆 receive 之後的 30–150 s（與正式 sweep 相同）。

> **狀態：完成（2026-10-14）。** 所有量測都已結束。本文件只分析既有原始資料，沒有跑任何新量測。

---

## 0. 為什麼要對齊

正式比較寫的是「5,000 EPS 時 WCM 的 p99 最低」（WCM 264 ms、Fluentd 480、Vector 523、FB Lua 1,013、FB WASM 1,020）。但五個產品的**送出批次條件不同**：

| 正式設定 | 實際的 flush 觸發（看 sink 每秒收到幾個 POST 得知） | 5,000 EPS 時約多久送一批 |
|---|---|---|
| WCM `wcm-b1` | 1 MiB **輸入**量（buffer_size_mb=1） | ≈0.2 s（880 POST／180 s） |
| Fluentd | 1 MiB **輸出** chunk（chunk_limit_size 1m；≈2.5 MiB 輸入） | ≈0.36 s（498 POST／180 s） |
| Vector | 1 MiB **輸出**（batch.max_bytes） | ≈0.5 s（360 POST／180 s） |
| Fluent Bit Lua／WASM | 純 1 s 計時器（Flush=1） | 1 s（181 POST／180 s） |

所以 p99 的高低主要反映「多久送一次」，而不是誰處理得比較快。本實驗用三種方法把批次條件對齊：

| 方法 | 對齊什麼 | 量測 |
|---|---|---|
| 1 時間對齊 | WCM 也改成 1 s 時間窗（`wcm-tw128`），和 Fluent Bit 的 Flush=1 一樣 | 延遲 sweep 2,500–22,250 EPS |
| 2 批量對齊 | 每批都是 1 MiB **輸入**（1,024 筆輸入 ≈ 410 筆輸出）；等效時間窗 W_eq = 1 MiB ／（rate × 1 KiB） | 5k／10k／15k／20k EPS |
| 3 取捨曲線 | 純時間窗 W ∈ {0.01, 0.1, 0.5, 1} s，所有 size trigger 關掉 | 5,000 EPS 延遲 + 每個 W 的容量 |

---

## 1. 結論（直接回答四個問題）

### Q1 用時間對齊時，WCM 的延遲如何？

**WCM 不是最低，而是最高。** 兩組獨立資料都顯示這點：

- **方法 3、5,000 EPS、同一個 W：** W=0.01 s 時，WCM 的 p99 是 32.7 ms，落在其他產品之間（FB Lua 29.5、Fluentd 35.2、Vector 38.9、FB WASM 200.6）。W ≥ 0.1 s 時 WCM 最高：W=0.1 為 160.7 ms（其他 116–131），W=0.5 為 661.0 ms（其他 515–558），W=1 為 **1,206.0 ms**（FB Lua 1,017.6、Fluentd 1,018.3、Vector 1,075.2）。
- **方法 1（tw128，1 s 窗），對照相同 rate 的正式 FB（Flush=1）：** WCM 的 p99 從 1,198 ms（2.5k）升到 1,787 ms（20k）。FB Lua 在所有 rate 都是 1,007–1,016 ms。22,250 EPS 時 tw128 被 OOM kill。
- **原因：** 「超出等待的延遲」excess = p99 − 0.99·W。WCM 的 excess 會隨 W 和 rate 變大：5,000 EPS 時 W=0.01／0.1／0.5／1 分別為 23／62／166／216 ms。FB Lua 一直維持在 17–28 ms，Fluentd 24–28 ms，Vector 29–85 ms。WCM 是**等時間窗關閉之後才整批跑 parse→filter→format→transport**，所以一批越大，最後一筆要等的處理時間就越長。這可以從方法 1 每秒最小延遲的底線看出來：2.5k 時 ≈200 ms，20k 時 ≈845 ms。其他產品則在窗內就邊收邊處理，flush 時只剩送出。

### Q2 用批量對齊（1 MiB 輸入／批）時？

**WCM 的 p99 仍是最高，比其他產品多約 40–60 ms。**

| rate | W_eq | FB Lua | Fluentd | FB WASM | Vector | WCM（本次） | WCM（正式 wcm-b1，同設定） |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 5,000 | 0.205 s | 220.1 | 221.3 | 225.6 | 228.8 | 366.4 ¹ | 264.0 |
| 10,000 | 0.102 s | 119.7 | 121.4 | — | 131.6 | 161.3 | 168.1 |
| 15,000 | 0.068 s | 87.6 | 89.1 | — | FAIL ² | 134.9 | 137.8 |
| 20,000 | 0.051 s | 71.2 | 72.7 | — | — | 128.3 | 129.8 |

p99（ms），已校正時鐘。¹ 本次 WCM 5k 的 30–60 s 子窗有一次 723 ms 的尖峰，其他三個子窗的 p99 為 260–262 ms，與正式值一致。² Vector 每批 410 筆（約 37 POST/s）時，在 15k 無法維持。
每批的實際筆數已用 sink 核對：WCM／FB／Vector 409–410 筆，Fluentd 413 筆。

### Q3 每個產品在取捨曲線上的位置？（方法 3，見圖 c）

| product | W=0.01 | W=0.1 | W=0.5 | W=1 |
|---|---|---|---|---|
| Fluent Bit Lua | 57,500 ／ 29.5 | **65,000** ／ 115.9 | 61,500 ／ 515.3 | 51,000 ／ 1,017.6 |
| Fluentd | 30,000 ／ 35.2 | **48,500** ／ 124.9 | 46,000 ／ 518.8 | 39,500 ／ 1,018.3 |
| WCM（tw128） | 20,250 ／ 32.7 | **24,750** ／ 160.7 | 22,000 ／ 661.0 | 19,250 ／ 1,206.0 |
| Vector | 12,750 ／ 38.9 | **15,500** ／ 131.0 | 14,250 ／ 557.6 | 13,750 ／ 1,075.2 |
| Fluent Bit WASM | 4,300 ／ 200.6 ³ | **5,500** ／ FAIL | 5,300 ／ FAIL | 5,300 ／ FAIL |

容量（EPS）／5,000 EPS 時的 p99（ms）。³ 5,000 EPS 已高於 FB WASM 在 W=0.01 時搜到的容量 4,300，屬邊緣點：子窗 p99 從 45 一路升到 220 ms。FB WASM 在 W ≥ 0.1 時，180 s 的 5,000 EPS run 全部 FAIL（generator 被擋）。

- **Fluent Bit Lua 支配所有產品**：每個 W 下容量都最高、p99 都最低。
- **Fluentd 第二。**
- **WCM 第三**：它支配 Vector 和 FB WASM（容量更高，且 W=0.01 時 p99 更低），但被 FB Lua 和 Fluentd 支配。WCM 的低延遲端（W=0.01：p99 33 ms、容量 20k）可用；高延遲端（W ≥ 0.5）同時輸在延遲和容量。
- **所有產品的容量都在 W=0.1 達到峰值。** W=0.01 時每次 flush 的固定成本變重（Fluentd −38%、WCM −18%、FB Lua −12%）。W ≥ 0.5 時批次變大、突發變大：WCM 在 W=1 是被**記憶體**限制（1 s × 20 MB/s 的 buffer，RSS 在 20k 時平均 309 MiB，超過 20k 就 OOM），其他產品則是 generator 被擋（C1）。

### Q4 論文應該寫的公平延遲結論

用以下內容取代「5,000 EPS 時 WCM 的 p99 最低」：

> 在 1 KiB／36 欄位、1 CPU／512 MiB 的條件下，端到端延遲主要由**批次送出條件**決定（p99 ≈ 0.99 × 批次窗 + 處理開銷），不是由處理引擎決定。正式比較中 WCM 的 p99 最低（264 ms @5,000 EPS），是因為 wcm-b1 每累積 1 MiB 輸入（約 0.2 s）就送出，而 Fluent Bit 每 1 s 才送、Fluentd／Vector 每 1 MiB 輸出（約 0.36–0.5 s）才送。
> 把批次條件對齊之後，WCM **沒有延遲優勢**。時間窗相同（W=0.01 s）時，WCM 的 p99（33 ms）與其他產品同一量級（29–39 ms，FB WASM 201 ms）。W ≥ 0.1 s 時，WCM 的 p99 比 Fluent Bit Lua 高 45–190 ms（W=1 s：1,206 vs 1,018 ms）。每批輸入量相同（1 MiB）時，WCM 的 p99 比 Fluent Bit／Fluentd 高約 40–60 ms。原因是 WCM 在批次窗關閉後才整批處理，處理時間直接加在延遲上，而且隨批次大小增加。
> WCM 能以小時間窗做到低延遲（W=0.01 s 時 p99 33 ms），同時維持約 20,000 EPS 的容量。在延遲／容量取捨曲線上，WCM 位於 Fluent Bit Lua、Fluentd 之後，Vector、Fluent Bit WASM 之前。

---

## 2. 每個產品的確切參數（knobs）

所有設定檔都在 `conf/`，由 `gen_confs.sh` 從正式設定複製產生，**只改批次參數**（`reproduce.sh confs` 會重新產生並比對）。

| 產品 | 方法 3（時間窗 W） | 方法 2（1 MiB 輸入／批） | 方法 1 |
|---|---|---|---|
| WCM | `wcm_tw128_wait{10,100,500,1000}.yaml`：`max_wait_ms = W×1000`、`buffer_size_mb=128`（size 不會觸發）、`mem_limit_mb=256`、`max_transport_bytes=1 MiB`、`transport_workers=1` | `wcm_b1.yaml`（= 正式 wcm-b1）：`buffer_size_mb=1`（輸入位元組）、`max_wait_ms=1000`、`mem_limit_mb=128` | `wcm-tw128` = `test/ours/latency-benchmark-Rust/forwarder_membound_tw128.yaml`（與 `conf/wcm_tw128_wait1000.yaml` 去掉註解後完全相同） |
| Fluentd | `fluentd_m3_W*.conf`：W=1 時 `flush_interval 1`、`flush_thread_interval 0.1`；W<1 時 `flush_interval=W`、`flush_thread_interval=W`（fluentd 會把 <1 的 flush_interval 截成 0，真正的窗口 = enqueue 輪詢週期 = flush_thread_interval）；`chunk_limit_size 128m`、`total_limit_size 256m`、`retry_max_times 0` | `fluentd_m2_rec410.conf`：`chunk_limit_records 432`（×0.95 ≈ 410 筆滿）、`chunk_limit_size 128m`、`flush_interval 10`、`flush_thread_interval 0.01` | 正式 `fluentd/latency/conf/capacity_1m_1s.conf`（chunk 1m、flush 1） |
| Vector | `vector_align.yaml`：`batch.timeout_secs=W`（FLUSH_INTERVAL）、`max_bytes=128 MiB`、`max_events=1e8`、`retry_attempts 0` | 同一檔：`timeout_secs=10`、`max_events=410`、`max_bytes=128 MiB` | 正式 sweep（max_bytes 1 MiB、timeout 1 s） |
| Fluent Bit Lua／WASM | `fbit_{lua,wasm}_flush_W.conf`：`[SERVICE] Flush W`（其餘 = 正式 `capacity_{lua,wasm}_1s.conf`；chunk 由引擎管理，約 2 MiB soft limit，沒有 size trigger） | `Flush = W_eq`（0.2048／0.1024／0.0683／0.0512） | 正式 sweep（Flush=1） |

Image（tag → `docker image inspect --format '{{.Id}}'`，2026-10-14 取得，與 `environment.txt` 及每個 run 的 `run_config.txt` 相同）：

| image | Id |
|---|---|
| `wcm-base-log-forwarder:memguard` | `sha256:6365acbf5f7f95c2a1ace3ce8e3600e45babe9f3d859a560e85b93f0c8086fba` |
| `fluent/fluent-bit:5.0.9` | `sha256:bbce9a7b36b0ddd4fcb01a5320815808d1b8b5e1e100725ac95c2b6714d64964` |
| `fluent/fluentd:v1.16-debian-1` | `sha256:4f65d55e8f24de4c59c75d4fdd702fb73e4bbe3a25f3fdd77f3c34dc9cec1930` |
| `timberio/vector:0.49.0-alpine` | `sha256:30141ab1ce5a7f84ef63580863e4ec561291e7c43dd0d7252985c9f2bb87aa5e` |

---

## 3. 結果表

### 3.1 方法 1：時間對齊（`method1_time_aligned.csv`、圖 f）

p50／p99（ms）。tw128 已用資料偵測法校正；其餘為正式 sweep，經偵測確認 30–150 s 窗內沒有時鐘階梯。

| rate | WCM tw128（1 s 窗） | WCM b1（1 MiB 輸入） | FB Lua（Flush=1） | FB WASM（Flush=1） | Fluentd（1 MiB 輸出／1 s） | Vector（1 MiB 輸出／1 s） | tw128 RSS mean |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 2,500 | 700／1,198 | 263／466 | 517／1,007 | 519／1,009 | 456／946 | 523／1,013 | 45 MiB |
| 5,000 | 728／1,217 | 158／264 | 523／1,013 | 530／1,020 | 235／480 | 277／523 | 75 MiB |
| 7,500 | 830／1,318 | 123／197 | 525／1,015 | — | 167／331 | 197／362 | 110 MiB |
| 10,000 | 949／1,434 | 107／168 | 524／1,014 | — | 128／253 | 158／284 | 139 MiB |
| 12,500 | 1,041／1,531 | 93／143 | 539／1,016 | — | 109／208 | 136／238 | 176 MiB |
| 15,000 | 1,176／1,685 | 87／138 | 537／1,016 | — | 93／176 | 124／215 | 220 MiB |
| 17,500 | 1,235／1,722 | 84／135 | 536／1,015 | — | 83／154 | — | 257 MiB |
| 20,000 | 1,311／1,787 | 79／130 | 536／1,015 | — | 76／138 | — | 309 MiB |
| 22,250 | **OOM-killed** | 77／126 | — | — | — | — | — |

- 和 FB（同樣是 1 s 計時器）相比，tw128 的 p99 多出 190–770 ms，差距隨 rate 增加。tw128 與 b1 在相同 rate 下的 CPU 幾乎一樣（例如 5k：23.0% vs 23.9%；20k：84.3% vs 85.2%），差別在 RSS（b1 約 25 MiB，tw128 隨 rate 線性增加）。
- tw128 原始（未校正）的 p99 是 5,583–8,056 ms，完全是時鐘污染造成的。舊版 `method3_latency_5000eps.csv` 的 WCM W=1 列曾直接引用這個 7,750 ms 的原始值，現在已改正。

### 3.2 方法 2（`method2_batch_volume.csv`、圖 e）

見 Q2 的表。CSV 中 `variant=output_1MiB_formal` 的列是正式 sweep 的 Fluentd／Vector（1 MiB **輸出** ≈ 2.5 倍筆數），僅作對照：例如 5k 時 Fluentd 480、Vector 523 ms，正是正式比較的數字。

### 3.3 方法 3（`method3_latency_5000eps.csv`、`method3_capacity.csv`、圖 a–d）

5,000 EPS 延遲（ms，用 clock_drift.csv 校正；全部 stable=yes，FAIL 除外）：

| product | W | p50 | p99 | p99.9 | excess | CPU % | RSS MiB |
|---|---:|---:|---:|---:|---:|---:|---:|
| WCM | 0.01 | 19.4 | 32.7 | 41.7 | 22.8 | 35.8 | 15.7 |
| WCM | 0.1 | 85.4 | 160.7 | 178.8 | 61.7 | 25.7 | 20.8 |
| WCM | 0.5 | 389.7 | 661.0 | 693.3 | 166.0 | 30.1 | 38.9 |
| WCM | 1 | 717.1 | 1,206.0 | 1,235.1 | 216.0 | 22.9 | 75.1 |
| Fluentd | 0.01 | 19.8 | 35.2 | 39.5 | 25.3 | 23.2 | 101.0 |
| Fluentd | 0.1 | 72.8 | 124.9 | 128.8 | 25.9 | 15.5 | 117.4 |
| Fluentd | 0.5 | 272.7 | 518.8 | 533.5 | 23.8 | 19.6 | 98.4 |
| Fluentd | 1 | 519.2 | 1,018.3 | 1,034.4 | 28.3 | 19.4 | 98.0 |
| Vector | 0.01 | 28.3 | 38.9 | 41.4 | 29.0 | 33.2 | 55.4 |
| Vector | 0.1 | 70.3 | 131.0 | 1,274.1 ⁴ | 32.0 | 32.1 | 39.8 |
| Vector | 0.5 | 291.1 | 557.6 | 616.1 | 62.6 | 45.0 | 35.8 |
| Vector | 1 | 559.3 | 1,075.2 | 1,122.9 | 85.2 | 45.0 | 47.0 |
| FB Lua | 0.01 | 16.7 | 29.5 | 30.8 | 19.6 | 11.6 | 25.9 |
| FB Lua | 0.1 | 62.6 | 115.9 | 120.3 | 16.9 | 9.7 | 6.7 |
| FB Lua | 0.5 | 269.2 | 515.3 | 526.8 | 20.3 | 12.0 | 7.3 |
| FB Lua | 1 | 527.3 | 1,017.6 | 1,035.6 | 27.6 | 12.2 | 9.3 |
| FB WASM | 0.01 | 26.5 | 200.6 | 228.3 | 190.7 | 99.4 | 284.6 |
| FB WASM | 0.1／0.5／1 | FAIL | FAIL | | | ≈100 | ≈285 |

⁴ 單一尖峰：p99.9 會受影響，p99 不會。excess = p99 − 0.99·W。

本次 session 的結果與正式資料可以互相核對：FB Lua W=1 本次 1,017.6 ms vs 正式 1,012.8 ms；WCM W=1 本次 1,206.0 ms vs 方法 1 的 tw128 @5k 1,217.5 ms（設定相同，校正方式不同）。

容量：見 Q3 的表。每列的 `first_fail_reason`：WCM 在 W ≤ 0.5 是 C2（backlog > 3 s 輸出），W=1 是 C4（OOM，mem-guard 撐不住）；其他產品都是 C1（generator 被擋：send-lag p99 > 10 ms）。

---

## 4. 資料品質與校正

### 4.1 WSL2 wall-clock 階梯（所有延遲數字都已確認為乾淨值或校正值）

本次開機後 WSL2 的 clocksource 是 `tsc`，wall clock 每 30–40 s 向前跳 0.5–2.0 s（`clock_drift.csv` 實測）。generator 時戳 = start_wall + 排程偏移，不受跳躍影響；sink 的 receive 用 `time.Now()`，會受影響。結果是原始延遲每 ~33 s 抬高一階（原始 p99 3.7–8.3 s）。

| 資料 | 數量 | 校正方式（CSV 的 `correction` 欄） |
|---|---:|---|
| 方法 2、3 的新 run（`runs/*/lat_rate_*`） | 36 | **clock_drift.csv 量測式校正**（`correct_latency.py`）：延遲 − [D(receive) − D(gen_start)]，D = wall − monotonic；窗口邊界也改在 monotonic 時間軸 |
| 方法 1 `wcm-tw128`（沒有 clock_drift.csv） | 8 PASS | **資料偵測校正**（`stepdetect.py`）：每個 run 偵測到 5–6 個階梯（787–1,860 ms），窗內記錄 100% 被位移 |
| 正式 sweep（wcm-b1、FB Lua／WASM、Fluentd、Vector） | 33 | 跑 `stepdetect.py` 確認：30–150 s 窗內**沒有**階梯，原始值即乾淨值 |
| 最早 4 個 W=0.01 run | 4 | 在加入 drift logger 之前測的，已移到 `runs_invalid_clockstep/`，**不採用** |

`stepdetect.py` 改寫自 `test/ours/per_record_invocation/scripts/analyze_followup.py`（原本在正式 wcm-b1 上驗證過），做了三點強化，才能處理 1 s 窗的資料（自然延遲範圍 ~1.2 s、暖機尖峰）：每秒取第 5 小值、5 秒中位數濾波、向下的水準變化不計入 offset。

**交叉驗證**（`clock_correction_audit.csv`）：在 32 個有 clock_drift.csv 的 PASS run 上，兩種校正的 p99 差距中位數為 0.9 ms；29 個 run 在 ±10 ms 以內，WCM m2 5k 差 20 ms（有尖峰）。有 2 個 run（Vector／WCM W=0.01）的階梯發生在第 1 秒以內，偵測法抓不到，偏差約 1 s。這就是偵測法的限制：**第一筆 receive 之前或第 1 秒內的階梯無法察覺。** 方法 1 的 tw128 用兩點排除了這個疑慮：(a) 每個 run 的階梯前底線隨 rate 單調上升（199→230→331→452→551→681→748→845 ms），沒有任何一個 run 平白高出 ~1 s；(b) tw128 @5k 的偵測值 1,217.5 ms 與本次 session 用 clock_drift 校正的同設定 run（1,206.0 ms）相差 11.5 ms。tw128 的數字應視為 **±20 ms**。

資源：新 run 的 CPU = Δcpu_time／Δmonotonic（`resource_stats_30_150.txt`）；正式與 tw128 的 CPU 是 docker 1 s 取樣的平均（wall），階梯發生的那一秒會略為低估。

### 4.2 W=1 的容量比正式 1 s 容量低：原因調查

| | 正式 1 s（2026-08-09，3 輪） | 本次 W=1 | 本次 W=0.1 |
|---|---:|---:|---:|
| FB Lua | 67,500 | 51,000（−24%） | 65,000（−4%） |
| Fluentd | 52,000 | 39,500（−24%） | 48,500（−7%） |
| Vector | 15,000 | 13,750（−8%） | 15,500（+3%） |
| FB WASM | 5,400 | 5,300（−2%） | 5,500（+2%） |
| WCM | 25,000（**wcm-b1**，不同設定） | 19,250（tw128） | 24,750（tw128） |

逐項檢查的結果：

1. **批次設定：等效，不是原因。** FB conf 與正式 `capacity_*_1s.conf` 只差 Script／WASM 的絕對路徑。Fluentd 的 conf 只有 `chunk_limit_size` 1m→128m 不同，但 sink 顯示正式 52,000 那次是 61 POST／81 s，和本次 W=1 一樣（61 POST）。也就是說，正式容量 run 實際上就是 1 s 時間主導，與 PROGRESS.md 記錄的一致。repo 中 `capacity_1m_1s.conf` 寫的是 1m，這和正式 sink 行為不一致，原因不明。Vector：正式 52 POST／60 s，本次 W=1 也是 52 POST。FB Lua：每千 EPS 的 POST 數相同（約 11.5–11.7）。
2. **PASS 判準：相同。** 都是 60 s 負載 + 20 s drain，eval_trial C1–C4。`eval_trial_align.py` 只把 wall 時刻換到 monotonic 時間軸。
3. **時鐘：不是原因。** 容量搜尋期間（00:45–03:00）每個 trial 的 `clock_drift.csv` 都是平的，60 s 內沒有任何階梯（階梯在 22:50 的最後一個延遲 run 之後就停了）。因此 C1（send-lag 本來就用 monotonic）和 C2（backlog）都沒有受到影響。
4. **失敗模式：在 C1 的門檻邊緣。** 本次 FB Lua 52,500 @W=1：gen_avg 達標、backlog 為 −1，但只有 98.39% 的記錄 send-lag ≤ 10 ms（門檻是 99%）；59,500 時為 94.7%。正式 67,500 的三輪是 99.72–99.99%。Fluentd 40,500 @W=1 也一樣（98.33%）。W=1 時每次 flush 一次送出約 1 s 的輸出（FB Lua 51k ≈ 20 MB），在單一 CPU 上和輸入搶資源，TCP 回壓讓 generator 偶發 10–50 ms 的延遲。
5. **機器狀態：有影響，但程度不一。** 同一個 session 中，W=0.1 的容量與正式 1 s 只差 −7% 到 +3%；per-record 研究在這個 session 量到的 WCM 整批容量是 24,200（正式 25,000，−3%）；FB WASM 只差 −2%。只有「大批次突發」的情境（FB Lua／Fluentd @W=1）掉了 24%。

**判斷：** 最可能的原因是**本次機器狀態**（WSL2 clocksource=tsc 的另一次開機、可用記憶體約 2.1 GB 等）讓單一 CPU 處理大 flush 突發的能力變差，再加上 C1 的 send-lag 門檻對突發很敏感，而且每個 rate 只跑一次 trial。「W 越大，突發越大」這個趨勢本身是真的（同一 session 內 W=1 比 W=0.1 低 11–22%），但正式資料證明，相同設定在另一個機器狀態下可以撐到 67,500／52,000，所以 W=1 懲罰的**幅度**取決於 session。WCM 的 W=1 不屬於這種情況：它的上限是 **OOM**（記憶體 = 窗口 × rate），在方法 1 中 22,250 也被 OOM kill，這是 tw128 設定本身的行為。
**這兩組數字不能混用。** 方法 3 的曲線（圖 b、c）全部採用本次 session 的數字；正式 1 s 容量只放在 `method3_capacity.csv` 的 `formal_capacity_eps` 對照欄，並不畫在圖上。若要把原因確認到單一因素，需要在目前的 session 用正式設定重跑一次 FB Lua 67,500（本文件沒有做）。

### 4.3 latency_stable 判準

`collect.py` 的定義：stable = 120–150 s 子窗的 p50 ≤ 1.3 × 30–60 s 子窗的 p50 + 20 ms。所有延遲列都用**最終採用的（校正後）序列**統一計算，結果記在 `latency_stable` 欄。
- 所有 PASS 的列都是 stable=yes。所有 FAIL 的列（FB WASM W ≥ 0.1、Vector m2 15k）都是 NO(growing)。
- 之前觀察到「FB Lua W=0.01 雖 PASS 但延遲階梯式增長」，那是時鐘階梯造成的假象。校正後四個子窗的 p99 都是 29–30 ms。
- 用 p50 判斷不會被單一尖峰誤判，但也看不到只發生在尾端的劣化。FB WASM W=0.01 @5k 的 p50 穩定（判為 stable），但 p99 從 45 升到 220 ms，CSV 的 `note` 欄有標記。

### 4.4 單次 vs 重複

- **延遲：** 每個點只有**一次** 180 s run（30–150 s 窗約 24 萬筆）。子窗 p99 的變異通常 < 2%。例外：WCM m2 5k 有一次尖峰，Vector W=0.1 的 p99.9 有一次尖峰。
- **容量：** 粗略搜尋，解析度約 5%，**每個 rate 只跑一次 60 s trial**，沒有確認重跑。正式容量是手動 3 輪。邊緣不一致的例子：WCM W=1 的 20,000 在 60 s trial 中 OOM，但方法 1 的 180 s 延遲 run 在 20,000 PASS；FB WASM W=0.01 搜到的容量是 4,300，但 180 s @5,000 PASS（邊緣）；FB WASM W ≥ 0.1 搜到 5,300–5,500，但 180 s @5,000 FAIL。
- 正式的 wcm-b1、FB、Fluentd、Vector 延遲 sweep 也都是每點一次。

---

## 5. 各方法的限制

- **方法 1：** 只把 WCM 改成時間主導，其他產品沿用正式設定。所以只有 FB（Flush=1）是真正同條件；Fluentd／Vector 的正式 sweep 是 1 MiB 輸出 size trigger。tw128 的 buffer 隨窗口累積，22,250 EPS 時 OOM，記憶體代價很大。tw128 的延遲是偵測校正值（±20 ms），沒有量測式的 drift 紀錄。
- **方法 2：** 「批量」以輸入位元組定義。FB 沒有 size trigger，只能用 `Flush = W_eq` 近似（每批筆數已核對為 409）。Fluentd 用 `chunk_limit_records`，Vector 用 `max_events`，WCM 用輸入 buffer 的位元組。四者的觸發機制不同，只是平均批量相同。Vector 15k 撐不住小批次；FB WASM 只量了 5k。
- **方法 3：** Fluentd 在 W<1 時沒有真正的 flush_interval（被截成 0），窗口 = flush_thread_interval 輪詢週期，結果仍符合 p99 ≈ W + 25 ms。延遲只量了 5,000 EPS。FB WASM 在 W ≥ 0.1 時，5,000 EPS 已超過它在 180 s 下的持續能力，所以沒有有效延遲。容量是單次 trial、約 5% 解析度。原本計畫的 FB Lua W=0.02／0.05 補點，以及「各設定 80% 容量點延遲」（`extra` 步驟），**都沒有執行**。
- **共通：** WSL2 主機；1 CPU 容器；單一 TCP 連線；sink 與 generator 在同一台主機（綁 core 8／9–10）。

---

## 6. 檔案

| 檔案 | 內容 |
|---|---|
| `method1_time_aligned.csv` | 方法 1：rate × DUT（tw128、b1、FB Lua／WASM、Fluentd、Vector）的 p50／p99／p99.9、校正方式、stable、CPU、RSS |
| `method2_batch_volume.csv` | 方法 2：product × rate 的 p50／p99（`input_1MiB` = 本次；`output_1MiB_formal` = 正式對照） |
| `method3_latency_5000eps.csv` | 方法 3：product × W 的 p50／p99／p99.9／excess／CPU／RSS／stable，W=1 附正式對照值 |
| `method3_capacity.csv` | 方法 3：product × W 的容量、first_fail 與原因、所有 trial、正式 1 s 對照欄（不混用） |
| `clock_correction_audit.csv` | 77 個被引用的延遲 run：原始值、clock_drift 校正值、偵測校正值、偵測到的階梯 |
| `charts/a_p99_vs_window_5000eps.svg` | (a) 5,000 EPS 下 p99 vs W |
| `charts/b_capacity_vs_window.svg` | (b) 容量 vs W |
| `charts/c_tradeoff_capacity_vs_p99.svg` | (c) 取捨曲線：x = 容量，y = 5,000 EPS 下的 p99 |
| `charts/d_excess_latency_vs_window.svg` | (d) excess vs W |
| `charts/e_method2_p99_vs_rate.svg` | (e) 方法 2：p99 vs rate |
| `charts/f_method1_tw128_vs_b1.svg` | (f) 方法 1：tw128 vs b1（附 FB 1 s 對照與 tw128 原始值） |
| `collect.py` → `finalize.py` → `plot.py` | 分析流程（只讀原始資料） |
| `stepdetect.py` | 資料偵測式時鐘階梯校正 |
| `correct_latency.py`、`clockfix.py`、`clock_drift_logger.py` | 量測式校正 |
| `runs/`、`runs_invalid_clockstep/`、`logs/runs.log`、`logs/search_*.out` | 原始資料（未刪除） |

---

## 7. 重現方式

### 7.1 環境（`environment.txt`，2026-10-13 18:49 擷取）

- Windows + WSL2：Linux 6.6.87.2-microsoft-standard-WSL2、Ubuntu 24.04.1 LTS，12 個邏輯核心，記憶體 7.5 GiB（量測時可用約 2.1 GiB）
- Docker 29.8.0；Python 3.12.3 + numpy 2.4.4；Go 1.27.1（只在 gen_bin／httpsink_bin 不存在時編譯）
- 量測二進位的 sha256 記錄在 `environment.txt`（gen_bin、httpsink_bin、eval_trial.py、calc_latency_stream.py、sample_cgroup_stats.sh）
- **clocksource=tsc**（正式實驗要求 `hyperv_clocksource_tsc_page`），所以全部以 `SKIP_CLOCK_CHECK=1` 執行，並全程記錄 `clock_drift.csv`
- 每個量測點都在 `flock /home/ckt1022/.wcm-bench.lock` 內獨占執行。sink 綁 core 8，gen 綁 core 9、10
- Image 與 Id：見 §2

### 7.2 設定檔

`conf/`（版本控管）。`gen_confs.sh` 從正式設定重新產生：FB `capacity_{lua,wasm}_1s.conf`、Fluentd `capacity_1m_1s.conf`、Vector `latency_tuned.yaml`、WCM `forwarder_membound_tw128.yaml`／`forwarder_membound_b1.yaml`。量測清單：`queue_m3_lat.txt`（方法 3 延遲）、`queue_m2.txt`（方法 2）、`searches.txt`（方法 3 容量，含起點）、`queue_last.txt`（WCM W=1 延遲）。方法 1 的量測腳本是 `../capacity/run_wcm_tw128_latency_cgroup.sh`（DUT `wcm-tw128`，輸出到 `../capacity/runs/wcm-tw128/`）。

### 7.3 量測（約 7–9 小時；已有 verdict.txt 的點會跳過，可中斷後續跑）

```bash
bash test/compare/batch_alignment/reproduce.sh            # = env confs m3lat m2 cap last extra analyze
bash test/compare/batch_alignment/reproduce.sh env        # 環境與 image Id
bash test/compare/batch_alignment/reproduce.sh m3lat      # 或 m2 / cap / last / analyze
RUNS_DIR=/path/new/runs RESULTS_DIR=/path/new bash test/compare/batch_alignment/reproduce.sh   # 不覆蓋既有 runs/
# 方法 1：
bash test/compare/capacity/run_wcm_tw128_latency_cgroup.sh
```

**search_align.sh 的 bug 修正：** 第 26 行原本是 `local rate=$1 out="$RUNS_DIR/$DUT/cap_rate_${rate}_${TAG}"`。bash 會先展開同一個 `local` 敘述中的 `${rate}`，那時 rate 還沒被賦值，所以 out 會指向錯誤的目錄。修正方式是拆成兩行（`local rate=$1`／`local out=...`）。修正時間（00:45）早於所有容量搜尋開始的時間（00:45:53），因此所有 `search_m3_W*.txt` 都是用修正後的版本跑的。

### 7.4 分析（不需 Docker，只讀既有資料，約 1 分鐘）

```bash
cd test/compare/batch_alignment
python3 collect.py      # 基礎彙整（讀 runs/ 與 ../capacity/runs/）
python3 finalize.py     # 最終 CSV：時鐘校正稽核、stable、方法 1、容量失敗原因（覆寫 collect.py 的 CSV）
python3 plot.py         # charts/a–f
# 等同 bash reproduce.sh analyze；RESULTS_DIR=<dir> 可輸出到別處（已驗證輸出與本目錄的 CSV 逐位元組相同）
python3 correct_latency.py runs/<dut>/lat_rate_<R>_<tag>   # 單一 run 重新做量測式校正（run_point.sh 已自動執行）
```
