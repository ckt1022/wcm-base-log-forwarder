# pattern_gaps — 補齊設計模式論文的證據缺口

> 本文件寫於啟動前，描述實驗設計。前一個套件是 `../pattern_suite/`，它的 `ANALYSIS.md` §8「仍然薄弱」列出了本套件要補的缺口。
> 所有量測沿用 pattern_suite 的 harness、image（`wcm-base-log-forwarder:suite`）、設定與 PASS 判定。主程式 `src/` **沒有任何修改**。

## 目的：每個 group 支撐哪個 pattern

| Group | 優先 | Pattern | 要補的缺口（ANALYSIS.md §8） | 內容 |
|---|---|---|---|---|
| **G1** | 高 | Streaming Micro-Batch | §2「W=0.1 容量只有 1 組同時段配對」；§7「改預設前在同一 session 交錯跑 3 次」 | W=0.1 容量搜尋交錯 batch → stream(k=256) × 3 對；5,000 EPS 延遲交錯 × 3 對 |
| **G2** | 高 | Watchdog and Respawn | §3／§8.4「watchdog 關閉的消融只有 1 rep」 | FAULT_LOOP，`epoch_interruption: false` × 3，與 watchdog 開（T=2 s）× 3 交錯（同為 75 s 負載） |
| **G3** | 中 | Watchdog and Respawn | §8.4「epoch 開銷只量一個負載點（15k）」 | epoch on/off × {5k, 10k, 20k} EPS × 3 reps（WCM-stream k=256），交錯；奇數 rep 先 on、偶數 rep 先 off |
| **G4** | 中 | Amortized Boundary Crossing | §8.7「k ∈ (1,16) 沒有量」；k=256 在約 25k EPS 時 5 ms wait 只湊得到約 125 筆，k=256 從未滿 | 容量：k ∈ {4, 8, 512, 1024} × 2、k=256 + `micro_batch_max_wait_ms` ∈ {10, 20} × 2；延遲 @5k：k512、k1024、wait10、wait20 × 1 |
| **G5** | 中 | Poison Record Bisection | §8.5「只量 2,500 EPS；沒有 malformed 負載下的容量與 p99」 | malformed 1%／5% × {batch, stream}：容量 × 1、5,000 EPS 延遲 × 2（0% 的對照 = G1） |
| **G6** | 低 | Capability-Scoped Stage | §8.6「只測 parse stage」 | filter／format／transport 的 chaos 變體：deny（載入期拒絕）× 3、inert 下 FAULT_NET／FILE／ENV × 3；transport 的 endpoint 白名單（wasi:http 與 raw sockets）× 3；`--check-plugin` 矩陣 × 3 |
| G7 | 低 | Amortized Boundary Crossing | §8.7「約 0.3 ms／micro-batch 的固定成本沒有 profile」 | **跳過**：`/usr/bin/perf` 是 linux-tools wrapper，執行時回報 `perf not found for kernel 6.6.87.2-microsoft`，沒有對應此 WSL2 kernel 的 perf，不用 sudo 無法安裝。log 中只寫一行 G7 SUMMARY。 |

每個 group 結束時在 `run_gaps.log` 寫一行 `G<n> SUMMARY: ...`（延遲為 `fix_latency.py` 校正後的 p99，CPU 為 `resource_stats_30_150.txt` 的 cpu_mean，只供快速檢視；正式數字以分析程式重算）。

## 設定

| 用途 | 設定檔 | 來源與差異 |
|---|---|---|
| WCM-batch W=0.1（G1、G5） | `../pattern_suite/conf/wcm_batch_W0.1.yaml` | 與 pattern_suite Phase 0 相同（含協調者後加的明確 `processing_mode: batch`，等於預設值） |
| WCM-stream k=256 W=0.1（G1、G3、G5） | `../pattern_suite/conf/wcm_stream_k256_W0.1.yaml` | 與 pattern_suite Phase 0／P2 相同 |
| epoch off（G3） | `../pattern_suite/conf/wcm_stream_k256_W0.1_noepoch.yaml` | 與 P4 相同 |
| G4 | `conf/wcm_stream_k{4,8,512,1024}_W0.1.yaml`、`conf/wcm_stream_k256_W0.1_wait{10,20}.yaml` | `gen_confs.sh` 由 k256 設定複製，只改 `micro_batch_records` 或 `micro_batch_max_wait_ms` |
| G2 | `../pattern_suite/fault/fault_stream_k256_{noepoch,t2000}.yaml` | 與 P4 相同（原生執行，parse = parse_chaos） |
| G6 | `fault/cap_<stage>_<grant>.yaml`（stage ∈ filter／format／transport；grant ∈ deny／inert，transport 另有 endpoint） | 由 `../pattern_suite/fault/fault_stream_k256_t2000.yaml` 複製：parse 改回正式 `latency_rust_parse.wasm`、該 stage 換成 `<stage>_chaos.wasm`、`capabilities:` 只授權該 stage（deny = 不寫；inert = `filesystem: inert` + `sockets: inert`；endpoint = `filesystem: inert` + `sockets: endpoint`） |
| G6 plugin 目錄 | `fault/plugin/` | symlink：正式 4 個 Rust plugin（`../../ours/latency-benchmark-Rust/plugin/`）、3 個新 chaos 變體、parse_chaos |

量測條件與 pattern_suite 相同：容器 1 CPU／512 MiB、json-fields1k（1 KiB／36 欄位）、容量 trial 60 s 負載 + 20 s drain、
延遲 180 s（窗口 30–150 s）、C1–C4 判定；原生 fault case 以 taskset 釘核（DUT core 4、sink core 8、generator core 9,10）。
G1 兩種模式的容量搜尋起點都是 22,000（同一個速率網格，避免起點造成解析度差異）。G4 起點：k=4 → 9,000、k=8 → 12,000、其他 → 22,000；rep2 從 rep1 的結果開始。
G5 起點：1% → 18,000、5% → 12,000。

## 這次新增或修改的程式（主程式 src/ 未改）

1. **generator `-malformed-rate`**（`tools/gen/main.go`）：任何 mode 都可把一定比例的行換成無法解析的 1,024 B 行
   （以 seq 的乘法雜湊決定，雜湊與 5 種樣式與 `test/compare/correctness/generator` 相同；不呼叫 rand，不改變亂數序列）。
   **預設 0 = 關閉，行為與舊版完全相同**。啟用時在 gen.log 的 done 行之前多印 `[gen] malformed: rate=R total=N keep_slot=M`
   （M = 被取代的 warn／error slot 數，這些行原本會通過 filter）。編譯到**新路徑** `bin/gen_malformed_bin`，
   `test/compare/capacity/gen_bin` 與 `tools/gen/main` 都沒有被覆寫。另外 `gofmt -w` 只調整了 flag 區塊的對齊空白。
   驗證（20,000 EPS × 2 s，stdout）：rate 0 → 0 行 malformed、warn/error = 40.00%（與舊 gen_bin 相同，全部 1,024 B）；
   0.01 → 1.05%；0.05 → 5.24%；送達比例 = 0.4 − M/total。
2. **`eval_trial_gaps.py`**：`../batch_alignment/eval_trial_align.py` 的複本，C1–C4 相同；只有 gen.log 有上述 malformed 行時
   expected = 0.4 × gen_total − M（否則與原版逐字相同；已用 pattern_suite 的一個 trial 驗證輸出相同）。
3. **chaos plugin 變體**（`test-plugins/rust-plugin/{filter,format,transport}_chaos/`，由正式 crate 複製）：
   - filter_chaos／format_chaos：唯一差異是 entry.message 含 FAULT_NET／FAULT_FILE／FAULT_ENV 時呼叫 `src/probe.rs`。
     FILE 讀 /etc/passwd、ENV 列環境變數、NET 以 raw TCP connect `127.0.0.1:8080`（sink，確實在 listen）與 `127.0.0.1:8081`。
     結果寫到 stderr：`[<plugin>] PROBE <kind> target=... outcome=DENIED|LEAK|DENIED(empty)|CONNECTED detail=...`。
   - transport_chaos：輸出資料含 marker 時探測。FAULT_NET 時：(a) wasi:http POST 到不在 endpoint map 的
     `http://127.0.0.1:8081/ingest` 與 `http://localhost:8080/ingest`（與 sink 同一個 socket，只是名稱不同）；
     (b) raw TCP 同上；(c) 同一批次照常 POST 到 endpoint map 的 endpoint，成功後印 `PROBE http_listed outcome=ACCEPTED`。
   - 因為使用了 std::fs／std::net，三個變體都 import wasi:filesystem（2）與 wasi:sockets（7）：預設 deny 時載入期被拒（9 個未授權 import），
     inert 與 endpoint 授權可載入（已用 `--check-plugin` 確認）。
4. 主程式、image、host binary（`target/release/wcm-base-log-forwarder`）都沒有重建。

## 腳本

| 腳本 | 說明 |
|---|---|
| `run_gaps.sh` | 主流程 G1 → G7。可續傳（verdict.txt／CAPACITY／host_status.txt／refusal.txt／#DONE 存在即跳過）、STOP、DONE；`SMOKE=1` 每種單位一個縮短版真實量測（`runs_smoke/`） |
| `capacity_align.sh`／`run_point.sh`／`search_align.sh` | pattern_suite 的複本。capacity_align 多了 `GEN_BIN`、`GEN_EXTRA_ARGS`（記入 run_config.txt）並改用 `eval_trial_gaps.py`；未設時量測流程逐行相同。run_point／search_align 內容不變（runs.log 在 `logs/`，STOP 檢查本目錄） |
| `eval_trial_gaps.py` | 見上 |
| `checkplugin.sh` | G6：8 個 plugin（每個 stage 的正式版 + chaos）× {deny, inert,（transport）endpoint} 的 `--check-plugin` 結果與分段計時，整個檔案在 flock 內 |
| `gen_confs.sh` | 產生 `conf/`、`fault/cap_*.yaml` |
| 沿用 | `../pattern_suite/run_fault_case.sh`（原生 fault case，記錄 clock_drift.csv）、`../pattern_suite/fault_gen.py`、`../pattern_suite/fix_latency.py`、`../../ours/fault_isolation/scripts/summarize_fault.py` |

結果：`runs/wcm/search_<tag>.txt`、`runs/wcm/{cap,lat}_rate_<rate>_<tag>/`（延遲 run 另有 `fix_latency.json`）、
`runs/fault/{g2_watchdog,g6_cap}/<case>/`、`runs/g6_checkplugin/checkplugin_r<n>.csv`；log：`run_gaps.log`、`logs/*.out`、`logs/runs.log`。
每個 run 目錄有 `clock_drift.csv`（延遲、容量 trial、fault case 全部都有）。

## 預估時間

依 pattern_suite 實測：容量 trial ≈ 99 s、搜尋 4–5 trials ≈ 7–8 分、延遲 run ≈ 220 s、fault case 45–100 s。

| Group | 單位 | 估計 |
|---|---|---|
| G1 | 6 搜尋 + 6 延遲 | ~70 分 |
| G2 | 6 fault（每個約 95 s） | ~10 分 |
| G3 | 18 延遲 | ~66 分 |
| G4 | 12 搜尋（k=4 可能較多 trial）+ 4 延遲 | ~105 分 |
| G5 | 4 搜尋 + 8 延遲 | ~60 分 |
| G6 | 9 refusal + 30 fault（30 s 負載）+ 3 checkplugin | ~30 分 |
| **合計** | 約 108 個單位 | **約 5.5–6 小時**（另一個 agent 也會拿同一把鎖做 build／smoke，可能更久） |

## 監看與停止

```bash
cd /home/ckt1022/wcm-base-log-forwarder/test/compare/pattern_gaps
tail -f run_gaps.log                       # 每個單位一行開始 + 一行結果；每個 group 一行 SUMMARY
grep SUMMARY run_gaps.log                  # 只看各 group 摘要
tail -f logs/runs.log                      # 每個 run_point 單位完成一行（rc、耗時、verdict）
tail -f logs/search.out logs/lat.out logs/fault.out   # 細節
kill -0 $(cat run_gaps.pid) && echo running
touch STOP                                 # 目前單位跑完後停止（搜尋則在 trial 之間停）；rm STOP 後重跑即續傳
kill $(cat run_gaps.pid)                   # 立即停止；殘留時：docker rm -f align-wcm；pkill -f target/release/wcm-base-log-forwarder
cat DONE                                   # 完成時出現
```

## 重現方式

### 環境
- Windows + WSL2：Linux 6.6.87.2-microsoft-standard-WSL2、Ubuntu 24.04，12 邏輯核心、約 7.5 GiB RAM。WSL2 wall clock 會跳（向前或向後），
  所以每個 run 都記錄 `clock_drift.csv`，延遲一律以 `../pattern_suite/fix_latency.py` 校正。
- Rust 1.93.1（`wasm32-wasip2` target）、Go 1.27.1、Python 3.12.3（numpy）、Docker。
- 共享鎖：`/home/ckt1022/.wcm-bench.lock`（所有建置與量測都在鎖內）。

### 建置（本次新增的 binary 與 plugin）

```bash
cd /home/ckt1022/wcm-base-log-forwarder
# generator（新路徑，不覆寫 test/compare/capacity/gen_bin）
(cd tools/gen && flock /home/ckt1022/.wcm-bench.lock go build -o ../../test/compare/pattern_gaps/bin/gen_malformed_bin main.go)
# chaos 變體
for s in filter format transport; do
  (cd test-plugins/rust-plugin/${s}_chaos && flock /home/ckt1022/.wcm-bench.lock cargo build --release --target wasm32-wasip2 -j 4 \
     && cp target/wasm32-wasip2/release/${s}_chaos.wasm ${s}_chaos.wasm)
done
bash test/compare/pattern_gaps/gen_confs.sh
# 確認：deny → LOAD_REFUSED（9 個未授權 import）；inert → LOAD_OK
target/release/wcm-base-log-forwarder --check-plugin transport test/compare/pattern_gaps/fault/plugin/transport_chaos.wasm
target/release/wcm-base-log-forwarder --check-plugin transport test/compare/pattern_gaps/fault/plugin/transport_chaos.wasm inert-fs inert-sockets
```
主程式與 image 沿用 pattern_suite（建置方式見 `../pattern_suite/README.md`「重現方式」），本次沒有重建。

### 本次使用的版本

| 項目 | ID |
|---|---|
| `wcm-base-log-forwarder:suite` | `sha256:9b7a60f2566e99cb4d37cd1ffa2f8f35b5bfe3c9ea01456ad4545ddd8dd894d8` |
| host binary（原生 fault／check-plugin） | sha256 `b5ab86755805f9716b85d84b7573ad76b3c69c5651e851f68af248783279c5fd`（與 pattern_suite 相同） |
| `bin/gen_malformed_bin` | 見 `logs/environment.txt` |
| `filter_chaos.wasm` | sha256 `87e981bced7cd05168697a5f1672eedb211e1f1a69be410eafe3a1846c3e4aed` |
| `format_chaos.wasm` | sha256 `61c7b0b47e91c464ecfe39a15bea78d5b41e9ea6ca36de52e98225aedcdfdc0a` |
| `transport_chaos.wasm` | sha256 `334144714b9619f5e560d1e597bee0291cc8ee07375d651aa5bee5776639fa22` |
| `parse_chaos.wasm` | sha256 `fefc19aaff9535d560e48a4178d693abf09c66681ff6d9555ba9c06f9f899875`（與 pattern_suite 相同） |
| 正式 Rust plugin | parse `94bbf1ad…`、filter `28c80863…`、format `09646209…`、transport `51529f84…`（與 pattern_suite 相同） |

每個 run 另存 `run_config.txt`（image ID、gen_bin、gen 額外參數、conf 全文）或 `meta.txt`（binary／plugin sha256、conf 全文）；
`logs/images.txt`、`logs/environment.txt` 記錄啟動時狀態。程式碼為 branch `wit_test` 的未 commit 工作樹（本次未 commit）。

### 啟動

```bash
cd /home/ckt1022/wcm-base-log-forwarder
for f in test/compare/pattern_gaps/*.sh; do bash -n "$f"; done
SMOKE=1 bash test/compare/pattern_gaps/run_gaps.sh > test/compare/pattern_gaps/smoke.log 2>&1     # 約 10 分（runs_smoke/）
setsid nohup bash test/compare/pattern_gaps/run_gaps.sh > test/compare/pattern_gaps/run_gaps.log 2>&1 < /dev/null &
pgrep -f '^bash test/compare/pattern_gaps/run_gaps.sh' > test/compare/pattern_gaps/run_gaps.pid
```

### 分析方式（計畫；沿用 pattern_suite 的做法）
- **延遲**：一律用 `fix_latency.py`（`runs/wcm/lat_*/fix_latency.json` 已由主流程寫好；也可重跑
  `python3 ../pattern_suite/fix_latency.py <run_dir>`）。報 p50／p99／p99.9 與 `p99_excl_post_step_3s`，3 reps 報「中位數 [最小–最大]」。
  `latency_stats_30_150.txt` 的 p99 在有向後階梯時不可用（見 pattern_suite ANALYSIS §0.1）。CPU 以去階梯的取樣時間重算 Δcpu_time／Δt
  （沿用 `analyze_suite.py` 的函式；分析時另寫 `analyze_gaps.py`，只讀 `runs/`，輸出到 `analysis/`）。
- **容量**：`search_*.txt` 的 `CAPACITY eps=`（約 5% 解析度、單次 trial）。G1 報 3 對的 stream/batch 比值（每對同一時段）與中位數；
  G4 加入 pattern_suite P2 的 k=1／16／64／256 畫 capacity vs k（注意跨 session，只看形狀，G4 的 k=256 參考值用 G1 的 stream）。
  wait10／20 與 G1 的 k=256（wait 5）比較；以 `[stream]` 類 dut.log 或 sink 每秒 POST 數推估有效批量（若可得）。
- **G2**：`summarize_fault.py` 的 `summary.csv`（records_lost、stall_zero_sec、recovery_sec）與 sink 最後連續 0 吞吐秒數（watchdog off 應 ≈ 60 s、on 應為 0）；
  host 存活、CPU（`stats.csv`）。
- **G3**：每個負載 on − off 的 CPU 差（pp 與 %）與 p99 差，3 reps 中位數；與 P4 的 15k 合併成 overhead vs EPS。20k 接近容量，FAIL 的 run 另列。
- **G5**：容量 vs malformed（0% = G1 同 session）、5k p99 與 CPU vs malformed；`dut.log` 的 `[parse-isolate]` 行數／呼叫次數；
  verdict 的 `malformed=`／`sink_total` 確認送達 = 合法記錄（C3 以修正後的 expected 判定）。
- **G6**：每個 stage × 授權 × 探測的 outcome（`refusal.txt`、`dut.log` 的 `PROBE` 與 `[SECURITY][transport]` 行）、host 存活、遺失
  （`summarize_fault.py`）；`checkplugin_r*.csv` 的 LOAD_OK／REFUSED 與 check_ms。

## 已知限制與注意事項
- 容量仍是單次 trial、約 5% 解析度；G1 才有 3 對同時段配對。
- G5 的 malformed 行是整行無法解析（5 種樣式），只量 parse 回傳錯誤型的 poison；trap 型見 pattern_suite §3。
  latency 只統計送達的合法記錄。
- G6 的 ENV 探測只能觀察到「看見空環境」（host 不注入任何變數），與 pattern_suite 相同，不是嚴格意義的「拒絕」。
- G6 transport 的 `localhost:8080` 探測：白名單以字串比對 host（`127.0.0.1` ≠ `localhost`），即使指向同一個 socket 也會被拒；這是現行實作的行為，分析時要寫明。
- `--check-plugin` CLI 不做 YAML 的 `sockets: endpoint 只限 transport` 驗證（filter／format 加 endpoint-sockets 也會 LOAD_OK），所以矩陣只對 transport 測 endpoint。
- 原生 fault case 沒有 cgroup，CPU 絕對值不可與容器量測比較。
