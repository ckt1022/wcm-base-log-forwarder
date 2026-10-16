# pattern_suite — 設計模式論文的整夜實驗套件

> 狀態：2026-10-14 23:03 啟動，2026-10-15 14:54 完成（含一個重跑的 run）。本文件寫於啟動前，描述實驗設計；
> **結果與分析請看 `ANALYSIS.md`**（`run_suite.log` 與各 run 的 `latency_stats_30_150.txt` 的 p99 在 04:04 之後被時鐘階梯污染，見下方「分析方式」）。

## 目的：每個 phase 支撐哪個 pattern

| Phase／實驗 | Pattern | 要回答的問題 |
|---|---|---|
| Phase 0 容量檢查 | （前置）Batch the Crossing | 同一 session、同一 image 下，streaming（k=64／256）的 W=0.1 容量是否 ≥ 0.95 × batch 模式（tw128 型）；選出後續使用的 k（BEST_K） |
| P1 malformed-rate sweep | **Poison Record Bisection** | 0／0.1／1／5／20% malformed 時，二分隔離是否仍「只拒絕壞記錄」（delivered/expected = 1），付出多少額外 parse 呼叫、CPU、RSS；batch（大批）vs stream（k 筆小批）的差異 |
| P1 stage timeout sweep | Poison Record Bisection × Watchdog | 一筆 FAULT_LOOP 的隔離時間與遺失筆數如何隨 `stage_timeout_ms` ∈ {1000, 5000, 30000} 變化 |
| P2 k sweep | **Batch the Crossing** | 跨 WASM 邊界的批量 k ∈ {1, 16, 64, 256} 對容量（W=0.1，3 reps）與 5,000 EPS 延遲的影響 |
| P4 epoch on/off | **Watchdog** | epoch interruption 本身的開銷（15,000 EPS 的 CPU% 與延遲，3 reps） |
| P4 watchdog 消融 | **Watchdog** | 關掉 watchdog 時 FAULT_LOOP 讓 parse 永久停住（吞吐降到 0）；開著時恢復 |
| P4 instance reuse | **Disposable Instance** | `plugin_instance_reuse` true vs false 的 CPU 代價（15,000 EPS）與 FAULT_LOOP 恢復時間 |
| P4 FAULT_MEM 重跑 | Disposable Instance／記憶體上限 | 以修正後的 RSS 取樣器重跑（舊資料 RSS 無效，見 `../../ours/fault_isolation/README.md`） |
| P5 | **Capability-Scoped World** | FAULT_NET／FILE／ENV 在「預設拒絕（載入期拒絕）」與「inert（執行期拒絕）」下實際發生什麼；載入期 import 檢查的開銷 |
| Phase 2 方法 3 | 對照（整體定位） | 六個設定（FB Lua、Fluentd、Vector、FB WASM、WCM-batch、WCM-stream）在 W ∈ {0.01, 0.1, 0.5, 1} 的延遲（5,000 EPS × 3）與容量 |

## 這次新增的程式開關（預設關閉，既有行為不變）

1. **`batch.epoch_interruption: true|false`（預設 true）**（`src/config.rs`、`src/app.rs`、`src/runtime.rs`、`src/main.rs`）
   false 時：engine 以 `Config::epoch_interruption(false)` 建置（Cranelift 不插 epoch 檢查）、不啟動 epoch ticker 執行緒、
   不設任何 `set_epoch_deadline`（全部改走 `app::arm_epoch_deadline`，停用時為 no-op）。啟動時設定一次，熱抽換重建同樣遵守。
2. **停用 stage timeout 的方式：只用 `epoch_interruption: false`**。`stage_timeout_ms` 的語意不變；
   `validate_and_describe` 改為：epoch 開啟時 `stage_timeout_ms` 必須 > 0（錯誤訊息會提示改用 `epoch_interruption: false`），
   epoch 關閉時允許 0、並印出 WARNING（watchdog 停用、stage_timeout_ms 不生效）。設定表多一列 `epoch_interruption`。
   單元測試：`config::epoch_switch_tests`（3 個）。`cargo test --release`：**42 passed**（原 39 + 3）。
3. **chaos plugin**（`test-plugins/rust-plugin/parse_chaos/src/lib.rs`）：新增 `FAULT_NET`（`std::net::TcpStream::connect("127.0.0.1:8080")`，
   走 wasi:sockets；8080 是實驗的 HTTP sink，確實在 listen）。FILE／ENV／NET 都把實際觀察寫到 stderr（→ `dut.log`）：
   `[parse_chaos] PROBE net|file|env ... outcome=DENIED|LEAK detail=<錯誤字串或變數數／位元組數>`。舊版保留為 `parse_chaos_v1.wasm`
   （sha256 `2558f724…`，與 `test/ours/fault_isolation/parse_chaos.wasm` 相同）。
4. **correctness generator／sink**：新增 `-malformed-rate <percent>`（例如 0.1）。以 seq 的乘法雜湊決定哪幾筆 malformed，
   generator 與 sink 必須給同一值；**未給時維持原本固定 5% 樣式**（seq%100 ∈ {0,21,42,63,84}）。實際 malformed 筆數記在
   `generator_summary.json`（100,000 筆時：0.1% → 約 122、1% → 約 1,090、5% → 約 5,176、20% → 約 19,972）。
5. `--check-plugin` 多印一行 `TIMING compile_ms check_ms link_ms instantiate_pre_ms`（第二次載入的分段計時，僅此 CLI）。
6. `Dockerfile`：可選 `--build-arg CARGO_BUILD_JOBS=4`（未給時與原本相同）。

## 設定檔

由 `gen_confs.sh` 從既有設定複製產生（只改列出的欄位）：

| 檔案 | 來源 | 差異 |
|---|---|---|
| `conf/wcm_batch_W<W>.yaml` | `../batch_alignment/conf/wcm_tw128_wait<ms>.yaml` | `max_transport_bytes` 1 MiB → **128 MiB** |
| `conf/wcm_stream_k<K>_W<W>.yaml`（K ∈ 1,16,64,256） | `../streaming_validation/conf/wcm_stream_W<W>.yaml` | `micro_batch_records=K`、`max_transport_bytes` → 128 MiB |
| `conf/wcm_stream_k<K>_W0.1_noepoch.yaml` | 上一列 W=0.1 | + `epoch_interruption: false` |
| `conf/wcm_stream_k<K>_W0.1_noreuse.yaml` | 上一列 W=0.1 | `plugin_instance_reuse: false` |
| `fault/fault_stream_k<K>_t<T>.yaml`（T ∈ 1000,2000,5000,30000） | `../../ours/fault_isolation/forwarder_fault.yaml` | streaming（k=K、`max_wait_ms=100`、`micro_batch_max_wait_ms=5`）、128 MiB、`stage_timeout_ms=T`；capabilities 不變（parse: filesystem inert + sockets inert） |
| `fault/fault_stream_k<K>_noepoch.yaml`／`_noreuse.yaml` | t2000 | + `epoch_interruption: false`／`plugin_instance_reuse: false` |
| `fault/fault_stream_k<K>_capdeny.yaml` | t2000 | 刪除 `capabilities:`（全部預設 deny） |
| `fault/fault_stream_k<K>_sockdeny.yaml` | t2000 | 只授權 `filesystem: inert`（sockets deny） |
| `fault/plugin` | → `../../../ours/fault_isolation/plugin`（parse_chaos + 正式 Rust filter／format／transport） | |

**max_transport_bytes 的修正：** 5,000 EPS × 1 KB 時 filter 後輸出約 2.1 MB/s，1 MiB 的 size trigger 約 0.5 s 就觸發，
所以 streaming_validation 的 W=0.5／1 兩點其實是 size-triggered（p99 都 ≈ 510 ms）。batch 模式的 tw128 設定同樣是 1 MiB
（W=1 時每批 ≈ 2.1 MB，會被切成 3 個 POST）。本套件所有 WCM 設定都改成 128 MiB（與 Vector `max_bytes`、Fluentd `chunk_limit_size` 相同）。
smoke 驗證：W=0.5、5,000 EPS、60 s → sink 收到 121 個 POST（= 2/s，時間觸發）。

其他產品沿用 `../batch_alignment/conf/`（`fbit_{lua,wasm}_flush_<W>.conf`、`fluentd_m3_W<W>.conf`、`vector_align.yaml` + `FLUSH_INTERVAL=W`）。
已確認 size trigger 關閉：Fluentd `chunk_limit_size 128m`；Vector 由 `capacity_align.sh` 注入 `VECTOR_BATCH_MAX_BYTES=134217728`、
`VECTOR_BATCH_MAX_EVENTS=100000000`；Fluent Bit 沒有 output size trigger（引擎 chunk ≈ 2 MiB soft limit）。

WCM container 一律用 image `wcm-base-log-forwarder:suite`（含本次程式開關）；plugin 為 `test/ours/latency-benchmark-Rust/plugin/`。

## 腳本

| 腳本 | 說明 |
|---|---|
| `run_suite.sh` | 主流程（Phase 0 → 1 → 2）。可續傳、STOP、DONE；`DRYRUN=1`（只走控制流程、寫假結果到 `runs_dry/`）、`SMOKE=1`（每種單位一個縮短版真實量測到 `runs_smoke/`） |
| `capacity_align.sh`／`run_point.sh`／`search_align.sh` | streaming_validation 的複本；預設 WCM image 改為 `:suite`、結果在 `runs/`、搜尋 trial 之間檢查本目錄 `STOP`。量測流程逐行不變（1 CPU／512 MiB、json-fields1k、60 s cap／180 s lat、20 s drain、eval_trial_align C1–C4、clock_drift.csv + correct_latency.py） |
| `run_correctness.sh` | correctness harness 複本；`MALFORMED_RATE` 傳給 generator 與 sink |
| `run_fault_case.sh` | 原生 fault case（改寫自 `fault_isolation/scripts/run_fault.sh`）：CONFIG／OUT 可指定、記錄 clock_drift.csv、generator 限時（`GEN_TIMEOUT`）、`MODE=refusal` 記錄載入期拒絕 |
| `fault_gen.py` | fault_gen.py 複本；被 SIGTERM 時仍寫 summary（`terminated=true`） |
| `loadcheck.sh` | 26 個 plugin × {預設 deny, inert-fs inert-sockets} 的 `--check-plugin` 計時 |
| `collect_suite.py` | 整理成 `analysis/{latency,capacity,correctness,fault,loadcheck}.csv` |
| `gen_confs.sh` | 產生 `conf/`、`fault/` |

**原生執行的 fault case**（P1 timeout、P4 watchdog／reuse／mem、P5）：不在容器內、沒有 cgroup 限制，以 taskset 釘核
（DUT core 4、sink core 8、generator core 9,10），與 `test/ours/fault_isolation` 相同。CPU／RSS 由修正後的 `sample_stats.sh tree` 取樣。
host binary = `target/release/wcm-base-log-forwarder`（與 `:suite` image 同一份原始碼）。

## 執行順序與決策規則

每個量測單位（一次完整 run，含 container／process 啟停）都在 `flock /home/ckt1022/.wcm-bench.lock` 內（`run_point.sh`、
`run_fault_case.sh`、`loadcheck.sh` 自己持鎖；correctness 由 `run_suite.sh` 以 flock 包住）。每個延遲 run 與 fault run 都記錄 `clock_drift.csv`。

### Phase 0（約 35 分）
1. W=0.1 容量搜尋：WCM-batch（`wcm_batch_W0.1`，起點 24,750）、stream k=64、stream k=256（起點 22,000）。
2. 5,000 EPS 延遲：stream k ∈ {64, 256} × W ∈ {0.5, 1}（128 MiB → 時間觸發）。
3. **決策（`runs/phase0_decision.txt`）：BEST_K = W=0.1 容量較高的 k（相同時取 64）**；之後的「WCM-stream」= stream k=BEST_K。
   同時記錄 `ratio_best_stream_over_batch` 與 `stream_ge_0.95x_batch=YES|NO`。

### Phase 1（約 3.6 小時）
- **P1a**：C3 型（2,500 EPS、100,000 筆）× malformed {0, 0.1, 1, 5, 20}% × {batch = `wcm_batch_W0.1`, stream = `wcm_stream_k<BEST_K>_W0.1`} × 3 reps
  → `runs/correctness/p1_<mode>_mr<rate>/C3_rep<n>/`（含 `summary.csv`）。
- **P1b**：FAULT_LOOP × stage_timeout {1000, 5000, 30000} × 3（stream k=BEST_K；負載 5,000 EPS、第 15 s 注入；SECS = 50／70／200 s
  讓 30 s timeout 的整批重試 2 次 + 二分探測（估計約 2 分鐘停頓）後仍有穩態）→ `runs/fault/p1_timeout/`。
- **P2**：容量 W=0.1 × k ∈ {1, 16, 64, 256} × 3（Phase 0 的 k64／k256 = rep1）；5,000 EPS 延遲 × 每個 k × 1。
- **P4**：
  - 15,000 EPS、stream k=BEST_K、W=0.1：epoch on（`stream_k<K>_W0.1_r<n>`）／epoch off（`_noepoch_r<n>`）／reuse off（`_noreuse_r<n>`）各 3 reps。
    **reuse on 的那一組與 epoch on 是同一份設定，共用同一組 3 次 run**（不另跑）。
  - FAULT_LOOP watchdog 停用 × 1（注入後 60 s 由 `GEN_TIMEOUT=75` 結束 generator，再 drain 8 s、停 DUT）；啟用（t2000）× 3。
  - FAULT_LOOP reuse off × 3（reuse on = 上一項 t2000 的 3 次）。注意：`plugin_instance_reuse` 只影響 format／transport（parse／filter 一律重用），
    parse 的 FAULT_LOOP 預期不受影響（這本身是一個要驗證的結果）。
  - FAULT_MEM × 3 + baseline × 1，用**原本的** `forwarder_fault.yaml`（batch 模式），可與 `fault_isolation/runs/native_faults/mem` 直接對照。
- **P5**（stream k=BEST_K，各 3 reps）：
  - FAULT_NET、sockets deny（`_sockdeny`）→ `MODE=refusal`：預期**載入期拒絕**，`refusal.txt` 記錄 exit code 與 `[capability]` 訊息。
  - FILE／ENV 的「預設政策」（`_capdeny`，全部 deny）：chaos plugin 因 Rust std 連帶 import filesystem＋sockets，**整個 plugin 在載入期就被拒絕**，
    與 marker 無關，所以只跑一組 `fileenv_capdeny_r<n>` 代表 FILE 與 ENV 兩者。
  - FAULT_NET／FILE／ENV 在 `filesystem: inert` + `sockets: inert` 下（30 s 負載）：預期執行期拒絕，`dut.log` 的 `PROBE` 行記錄觀察結果。
  - `loadcheck.sh` × 3：`check_ms`（import 檢查）相對 `compile_ms + link_ms + instantiate_pre_ms`（沒有檢查時的載入）就是檢查開銷；
    `wall_ms` 是整個 CLI（含兩次載入），只作參考。
- 每個 pattern 結束寫一行 `PHASE1-Px SUMMARY` 到 log。

### Phase 2（約 8.5 小時，最後）
- 產品：FB Lua、Fluentd、Vector、FB WASM、WCM-batch（`wcm_batch_W<W>`）、WCM-stream（`wcm_stream_k<BEST_K>_W<W>`）。
- 延遲 @5,000 EPS × 3 reps × W ∈ {0.01, 0.1, 0.5, 1}（順序：rep → W → 產品）。WCM-stream 的 rep1（W=0.1／0.5／1）就是 P2／Phase 0 的同設定 run。
  **FB WASM：W ≥ 0.1 的 3 次 5,000 EPS 只要有一次 FAIL，就加跑 4,000 EPS × 3。**
- 容量搜尋：每個產品 × 每個 W × 1，再加 W=0.1 的 rep2、rep3。起點 = batch_alignment 同 session 容量（WCM-stream 用 Phase 0／P2 的值）。
  WCM-stream W=0.1 的 3 reps = P2 的 k=BEST_K 3 reps；WCM-batch W=0.1 rep1 = Phase 0。
- 結果：`runs/<dut>/lat_rate_<rate>_<tag>/`、`runs/<dut>/search_<tag>.txt`（tag：`m3_W<W>_r<n>`、`batch_W<W>[_r<n>]`、`stream_k<K>_W<W>[_r<n>]`）。

### 結束
`collect_suite.py` → `analysis/*.csv`；`DONE` 一行摘要。

## 預估時間

依前次實測：cap trial ≈ 99 s、lat run ≈ 220 s、correctness ≈ 80 s、fault case ≈ 45–220 s；搜尋平均 4–5 trials。

| Phase | 單位 | 估計 |
|---|---|---|
| 0 | 3 搜尋 + 4 lat | ~35 分 |
| 1 | 30 correctness + 10 搜尋（k=1 可能 6–8 trials）+ 13 lat + 35 fault + 3 loadcheck | ~3.6 小時 |
| 2 | 69 lat（+ FB WASM 4,000 EPS 最多 9）+ 32 搜尋 | ~8.5 小時 |
| **合計** | 約 210 個單位 | **約 12.5–13 小時**（若與其他實驗搶鎖會更久） |

## 監看與停止

```bash
cd /home/ckt1022/wcm-base-log-forwarder/test/compare/pattern_suite
tail -f run_suite.log                     # 主流程（每個單位一行開始 + 一行結果；每個 phase 一行 SUMMARY）
tail -f logs/runs.log                     # 每個 run_point 單位完成一行（rc、耗時、verdict）
tail -f logs/search.out logs/lat.out logs/correctness.out logs/fault.out   # 細節
cat runs/phase0_decision.txt              # Phase 0 決策
kill -0 $(cat run_suite.pid) && echo running
touch STOP                                # 目前單位跑完後停止（搜尋則在 trial 之間停）；rm STOP 後重跑即續傳
kill $(cat run_suite.pid)                 # 立即停止；殘留時：docker rm -f align-wcm align-fbit-lua ...；pkill -f wcm-base-log-forwarder
```

## 重現方式

### 環境
- Windows + WSL2：Linux 6.6.87.2-microsoft-standard-WSL2、Ubuntu 24.04，12 邏輯核心、約 7.5 GiB RAM，clocksource 依開機而異（WSL2 有 0.6–1.2 s 的 wall-clock 跳躍；
  延遲以 `clock_drift.csv` + `../batch_alignment/correct_latency.py` 校正）。
- Rust 1.93.1（host 與 plugin，`wasm32-wasip2` target）、Go 1.27.1（correctness generator／sink）、Python 3.12.3、Docker。
- 共享鎖：`/home/ckt1022/.wcm-bench.lock`（所有建置與量測）。

### 建置

```bash
cd /home/ckt1022/wcm-base-log-forwarder
flock /home/ckt1022/.wcm-bench.lock cargo build --release -j 4
flock /home/ckt1022/.wcm-bench.lock cargo test --release -j 4          # 42 passed
flock /home/ckt1022/.wcm-bench.lock docker build --build-arg CARGO_BUILD_JOBS=4 -t wcm-base-log-forwarder:suite .
# chaos parser（含 FAULT_NET 與 PROBE 輸出）
cd test-plugins/rust-plugin/parse_chaos
flock /home/ckt1022/.wcm-bench.lock cargo build --release --target wasm32-wasip2 -j 4
cp target/wasm32-wasip2/release/parse_chaos.wasm parse_chaos.wasm      # fault_isolation/plugin/parse_chaos.wasm 是指向這裡的 symlink
cd -
target/release/wcm-base-log-forwarder --check-plugin parse test/ours/fault_isolation/plugin/parse_chaos.wasm                           # LOAD_REFUSED
target/release/wcm-base-log-forwarder --check-plugin parse test/ours/fault_isolation/plugin/parse_chaos.wasm inert-fs inert-sockets    # LOAD_OK
# correctness generator／sink（-malformed-rate）
(cd test/compare/correctness/generator && go build -o ../correctgen_bin main.go)
(cd test/compare/correctness/validationsink && go build -o ../validationsink_bin main.go)
bash test/compare/pattern_suite/gen_confs.sh
```

### 本次使用的版本

| 項目 | ID |
|---|---|
| `wcm-base-log-forwarder:suite` | `sha256:9b7a60f2566e99cb4d37cd1ffa2f8f35b5bfe3c9ea01456ad4545ddd8dd894d8` |
| `fluent/fluent-bit:5.0.9` | `sha256:bbce9a7b36b0ddd4fcb01a5320815808d1b8b5e1e100725ac95c2b6714d64964` |
| `fluent/fluentd:v1.16-debian-1` | `sha256:4f65d55e8f24de4c59c75d4fdd702fb73e4bbe3a25f3fdd77f3c34dc9cec1930` |
| `timberio/vector:0.49.0-alpine` | `sha256:30141ab1ce5a7f84ef63580863e4ec561291e7c43dd0d7252985c9f2bb87aa5e` |
| host binary（原生 fault） | sha256 `b5ab86755805f9716b85d84b7573ad76b3c69c5651e851f68af248783279c5fd` |
| `parse_chaos.wasm`（新） | sha256 `fefc19aaff9535d560e48a4178d693abf09c66681ff6d9555ba9c06f9f899875` |

每個 run 另存 `run_config.txt`（image ID、conf 全文）或 `meta.txt`（binary／plugin sha256、conf 全文）；`logs/images.txt`、`logs/environment.txt` 記錄啟動時狀態。
程式碼為 branch `wit_test` 的未 commit 工作樹（本次未 commit）。

### 啟動

```bash
cd /home/ckt1022/wcm-base-log-forwarder
for f in test/compare/pattern_suite/*.sh; do bash -n "$f"; done
DRYRUN=1 bash test/compare/pattern_suite/run_suite.sh      # 控制流程檢查（runs_dry/，數秒）
SMOKE=1 bash test/compare/pattern_suite/run_suite.sh       # 每種單位一個短量測（runs_smoke/，約 8 分）
setsid nohup bash test/compare/pattern_suite/run_suite.sh > test/compare/pattern_suite/run_suite.log 2>&1 < /dev/null &
pgrep -f 'pattern_suite/run_suite.sh' > test/compare/pattern_suite/run_suite.pid
```

### 分析方式

```bash
cd test/compare/pattern_suite
python3 collect_suite.py runs            # → analysis/latency.csv capacity.csv correctness.csv fault.csv loadcheck.csv（原始整理）
python3 analyze_suite.py                 # → analysis/*.csv（tidy）、analysis/charts/*.svg（約 40 s，只讀 runs/）
python3 fix_latency.py runs/wcm/lat_rate_5000_stream_k256_W0.1_r3   # 單一 run 的校正結果（JSON，一行）
```
結果與解讀見 `ANALYSIS.md`（依 pattern 整理）。

**2026-10-15 補充：延遲校正與無效 run 的處理（分析時新增）**
- 04:04 之後 WSL2 的 wall clock 改為**向後**跳（每 ~30 s −0.4～−2.7 s）。`correct_latency.py` 假設階梯向前，向後時會把階梯前最後 |step| 秒的記錄
  多算 |step|，所以 run 目錄內的 `latency_stats_30_150.txt`／`run_suite.log`／`analysis/latency.csv` 的 p99 在 04:04 之後**不可使用**
  （例：stream k256 W=0.1 r3 記錄為 1,787.8 ms，校正後 135.5 ms）。一律改用 `fix_latency.py`（依檔案順序與每個 POST 的最小延遲判定階梯前後；
  在 38 個向前階梯的舊 run 上與 correct_latency.py 相差 ≤ 0.77 ms，見 `analysis/fix_latency_validation.csv`）。CPU 也以去階梯後的取樣時間重算。
- Fluentd 的 flush 計時器用 wall clock，向後階梯會讓它停 ~|step| 秒（真實行為）。`analysis/*` 同時給 `p99`（完整校正）與 `p99s`
  （再排除每個向後階梯後 0–3 s 的記錄；其他產品兩者差 ≤ 10 ms）。
- **無效 run**：host 在 06:18:21–09:26:38 睡眠，正好落在 `fluentd/lat_rate_5000_m3_W0.01_r3` 中 → 已改名為 `*_r3_INVALID_host_paused`（保留原始資料、不刪除），
  14:51–14:54 以 `run_suite.sh` 續傳重跑（`run_suite_rerun.log`、`DONE`；第一次的 DONE 存為 `DONE.first_pass`）。`analyze_suite.py` 排除所有 `*_INVALID_*` 目錄，
  並以 `logs/runs.log` 的「結束時間 − 耗時」檢查其他 run 是否跨過暫停（`analysis/run_validity.csv`：只有這一筆）。
- 09:26 之後（r3 延遲、Phase 2 全部容量、FB WASM 4,000 EPS）機器狀態不同（guest 時鐘快 5–8%、同負載 CPU +5～+35%），分析時標為 `regime=post_resume`。
- 下面幾點是啟動前寫的原始分析計畫；延遲的部分已由上述取代。

- 延遲（原計畫）：`latency_stats_30_150.txt`（30–150 s 窗、clock-step corrected）的 p50／p99／p99.9；excess = p99 − 0.99·W；CPU／RSS 來自 `resource_stats_30_150.txt`（Δcpu_time／Δmonotonic）。
  每個點 3 reps 報中位數與範圍；`subwindows_clockfix.txt` 檢查子窗穩定性。
- 容量：`search_*.txt` 的 `CAPACITY eps=`（約 5% 解析度、單次 trial）；W=0.1 的 3 reps 報中位數與範圍。Phase 0 的 0.95 判準與 BEST_K 已寫在 `phase0_decision.txt`。
- P1a：delivered/expected（= received/expected）、rejected_lines（應 = malformed 輸入數）、isolation_parse_calls（`[parse-isolate] calls=`）、CPU、RSS，對 malformed rate 作圖，batch vs stream。
- P1b／P4 fault：`fault/<group>/summary.csv`（`../../ours/fault_isolation/scripts/summarize_fault.py`：records_lost、stall_zero_sec、recovery_sec、extra CPU）
  與 `analysis/fault.csv`（zero_sec_tail = 停滯到結束的秒數，watchdog 停用時應 > 0；PROBE 行；RSS）。恢復時間以 sink 的 monotonic 秒計，1 s 解析度。
- P4 epoch 開銷：epoch on vs off 的 cpu_mean 與 p99 差（3 reps 中位數）；reuse off vs on 同理。
- P5：`refusal.txt`（載入期拒絕訊息）、`dut.log` 的 `[parse_chaos] PROBE`（執行期觀察）、`analysis/loadcheck.csv`（check_ms vs compile_ms）。

## 限制與已知事項
- 容量搜尋是單次 trial、約 5% 解析度；W=0.1 才有 3 reps。
- fault case 是原生執行（無 cgroup）；與容器量測的絕對 CPU 不可直接比較。
- P1b 的 30 s timeout：停頓期間輸入 channel（50,000 筆 ≈ 10 s）滿了以後由 TCP 回壓擋住 generator，generator 之後會補送；恢復時間以 sink 吞吐判定。
- `fault_gen.py` 被終止時，`sent` 扣除尚未送出的最後一批，可能有 ≤ 100 筆的誤差（只影響 watchdog 停用那一個 run）。
- `-malformed-rate` 的實際比例不是精確的 rate%（雜湊），以 `generator_summary.json` 的 `malformed` 為準。
