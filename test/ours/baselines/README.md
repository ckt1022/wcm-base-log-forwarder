# No-WASM 對照組（Item 2）

回答「WCM 沙箱整體要付多少成本」：把 WCM 和**不用 WebAssembly** 的做法放在同一套輸入與計時方式下比較。

- **原生 stage**：`src/native.rs`，host 內建的 Rust parse／filter／format／transport，邏輯逐行對照 `test-plugins/rust-plugin/*`。
  在 `forwarder.yaml` 中把 `plugins.<stage>` 設為 `native`（或留空）即可選用。微基準透過 `#[path]` 引用**同一份檔案**。
- **微基準**（`src/bin/micro.rs`、`src/bin/stage_child.rs`）：每個 stage 單獨量 copy-in／logic／copy-out。
- **系統層**（`scripts/run_system_capacity.sh`）：同一個 forwarder binary、同一套 generator／sink／判定，只切換 `system/forwarder_{wcm,native}.yaml` 的 `plugins`。

## 每種 baseline 隔離了什麼、沒隔離什麼

| 方法 | 記憶體隔離 | crash 隔離 | CPU／時間上限 | 記憶體上限 | 能力（檔案、網路、env）限制 | 邊界成本 |
|---|---|---|---|---|---|---|
| Native Rust（in-process） | 無 | 無：panic 會讓 host 執行緒崩潰 | 無 | 無 | 無：擁有 host process 的全部權限 | 0（借用輸入） |
| Subprocess + pipe | 有（獨立位址空間） | 有：child 掛掉不會帶走 host | 可以靠 rlimit／cgroup，**本實驗沒有設定** | 同左，未設定 | **預設沒有**：child 繼承啟動者的檔案、網路、env；需另外套 seccomp／namespace／landlock | 序列化 + 2 次 pipe 複製 + context switch |
| Core Wasm（JSON ABI／Direct Memory） | 有（線性記憶體） | 有（trap） | 有（epoch） | 有（limiter） | 有（只給 import 的函式） | JSON 編解碼，或手寫二進位 layout |
| WCM（Canonical ABI） | 有 | 有 | 有 | 有 | 有，且本次新增逐 stage WASI 限制（Item 3） | Canonical ABI lift／lower |

所以 native 是「完全沒有隔離」的下限；subprocess 是「只有 OS 程序隔離、沒有能力限制」的常見替代方案；
Core Wasm 與 WCM 都有沙箱，差在介面型別與複製方式。

## 結果

### 微基準：1024 筆（約 1 MiB），5 種方法

資料：[`results/micro_20261013_181014/five_method_1024_tidy.csv`](results/micro_20261013_181014/five_method_1024_tidy.csv)（長表），
全部 batch 大小見 [`five_method_comparison.csv`](results/micro_20261013_181014/five_method_comparison.csv)，
圖：[`charts/`](results/micro_20261013_181014/charts/)（`{parse,filter,format}_total_five_methods.svg`、`breakdown_1024_five_methods.svg`）。

300 個樣本的中位數（3 runs × 100 rounds，各有 10 次 warm-up），p95 用 nearest-rank：

| 階段 | 方法 | Copy-in | Logic | Copy-out | Total（中位數） | Total p95 | 相對 native |
|---|---|---:|---:|---:|---:|---:|---:|
| Parse | WCM Canonical ABI | 0.863 | 2.126 | 1.354 | **4.399** | 6.465 | 2.1× |
| Parse | Core Wasm JSON ABI | 3.085 | 2.235 | 5.689 | 11.122 | 12.433 | 5.2× |
| Parse | Core Wasm Direct Memory | 0.803 | 2.304 | 1.090 | 4.220 | 5.143 | 2.0× |
| Parse | Native Rust | 0 | 2.119 | 0 | **2.119** | 2.925 | 1.0× |
| Parse | Subprocess + pipe | 1.668 | 1.950 | 3.564 | 7.307 | 9.078 | 3.4× |
| Filter | WCM Canonical ABI | 1.790 | 0.002 | 0.143 | **1.945** | 4.759 | — |
| Filter | Core Wasm JSON ABI | 3.036 | 0.131 | 0.345 | 3.515 | 4.248 | — |
| Filter | Core Wasm Direct Memory | 0.700 | 0.110 | 0.015 | 0.828 | 0.987 | — |
| Filter | Native Rust | 0 | 0.001 | 0 | **0.001** | 0.001 | — |
| Filter | Subprocess + pipe | 3.077 | 0.014 | 0.036 | 3.308 | 4.588 | — |
| Format | WCM Canonical ABI | 1.961 | 10.646 | 0.269 | **14.833** | 21.458 | 7.7× |
| Format | Core Wasm JSON ABI | 3.924 | 4.099 | 5.514 | 13.619 | 14.983 | 7.1× |
| Format | Core Wasm Direct Memory | 0.808 | 4.473 | 0.044 | 5.315 | 5.704 | 2.8× |
| Format | Native Rust | 0 | 1.926 | 0 | **1.926** | 2.584 | 1.0× |
| Format | Subprocess + pipe | 1.503 | 1.696 | 0.910 | 4.250 | 6.593 | 2.2× |

（單位 ms。Subprocess child 冷啟動 spawn → 第一個 ping：中位數 0.856 ms，p95 1.285 ms，n=60。）

讀法：
- **Parse**：WCM 比 native 慢 2.1 倍（+2.3 ms／MiB），幾乎全是邊界複製；比 subprocess 快 1.7 倍；和 Core Direct 相近。
- **Filter**：邏輯只有 1–14 µs，總時間就是邊界成本。native 借用輸入、完全不複製，所以「相對 native」的倍數沒有意義（> 2,000×）；
  有意義的比較是 WCM 1.9 ms vs subprocess 3.3 ms vs Core Direct 0.8 ms。
- **Format**：WCM 的 logic 段（10.6 ms）比 Core Direct（4.5 ms）、native（1.9 ms）都高很多，邊界只佔 15%；差距主要在 guest 內的格式化成本，不是 ABI。

**注意：** WCM／Core JSON／Core Direct 三欄來自 2026-08-15～17 的 `test/ours/copy_overhead/comparison_results/`，
native／subprocess 是 2026-10-13 新量的（同樣釘在 CPU 2、相同 batch 與取樣數）。**兩批資料不是同一天、同一次開機量的**，
所以不同日期的方法之間，差距 < ~20% 的部分不宜解讀。

### 系統層：WCM vs native，同一個 binary

資料：[`results/system_system_20261013_235508/system_latency.csv`](results/system_system_20261013_235508/system_latency.csv)、
[`system_capacity.csv`](results/system_system_20261013_235508/system_capacity.csv)、
[`system_max_pass.csv`](results/system_system_20261013_235508/system_max_pass.csv)、
圖 [`charts/capacity_ladder.svg`](results/system_system_20261013_235508/charts/capacity_ladder.svg)。

條件：原生程序（無容器、無 cgroup），DUT 用 taskset 釘在**單一核心**（core 4）。1024 B／36 欄位 JSON，filter 保留 40%。
batch 設定同正式 wcm-b1（`buffer_size_mb: 1`、`max_wait_ms: 1000`），`transport_workers: 1`。

**5,000 EPS 延遲**（150 s 負載，窗 30–140 s，約 22 萬筆，各 1 次）：

| variant | p50 | p90 | p99 | p99.9 | max | CPU mean | RSS mean |
|---|---:|---:|---:|---:|---:|---:|---:|
| WCM | 159.4 ms | 241.3 ms | **265.1 ms** | 295.5 ms | 319.6 ms | 21.7% | 40.0 MiB |
| native | 125.6 ms | 207.6 ms | **229.3 ms** | 236.2 ms | 241.3 ms | 8.3% | 17.3 MiB |
| 差 | +33.7 | +33.7 | **+35.8** | +59.3 | | 2.6× | 2.3× |

- 時鐘：兩個 run 都用 `test/compare/batch_alignment/stepdetect.py` 檢查，30–140 s 窗內**沒有偵測到 wall-clock 階梯**（`steps_detected=0`，子窗 p99 穩定在 229–231／265–267 ms），所以原始值即校正值，**沒有修正**。
- 延遲大部分來自批次等待（1 MiB 約 0.2 s），兩者共同；WCM 多出的約 34 ms 是沙箱與邊界的成本（每批 parse→filter→format→transport 都多一次跨邊界）。

**容量階梯**（每個 rate 1 次 60 s trial，判定 = `test/compare/capacity/eval_trial.py`）：

| rate (EPS) | 10k | 15k | 20k | 25k | 30k | 40k | 50k | 70k | 100k |
|---|---|---|---|---|---|---|---|---|---|
| WCM | PASS | PASS | PASS | FAIL（backlog 48,879 > 30,000） | FAIL | FAIL | FAIL | FAIL | FAIL |
| native | PASS | PASS | PASS | PASS | PASS | PASS | PASS | PASS | FAIL（gen 被擋，avg 96,944，lag p99 1,797 ms） |

- **最大 PASS rate：WCM 20,000 EPS（第一個 FAIL 是 25,000）；native 70,000 EPS（第一個 FAIL 是 100,000）。**
  階梯沒有在中間二分，所以真正的容量分別落在 [20k, 25k) 與 [70k, 100k)。
- 飽和時的處理量：WCM 輸出約 9.2–10.2k EPS（≈ 23–25.5k 輸入 EPS），CPU 72%、RSS 升到 146–168 MiB（積壓）；native 在 100k 時輸出 38.2k EPS（≈ 95.5k 輸入），CPU 64%。
- 換算：在相同單核條件下，native 的容量至少是 WCM 的 2.8 倍（70k／25k），至多約 5 倍（100k／20k）。
- WCM 25k 只因 backlog 判定失敗（generator 沒有被擋、sink 收齊），是邊緣點。
- CPU 量測是 process 範圍的 `/proc` 取樣（`sample_stats.sh`，single 模式，不受 tree bug 影響），不可與容器 cgroup 數字並列。

## 限制

- 系統層延遲與容量都是**每點 1 次**；容量沒有在 [20k,25k)、[70k,100k) 內二分。
- 原生執行、單核釘選，**不可**與 1 CPU／512 MiB 容器的正式數字直接比較（正式容器的 WCM 容量是 25,000）。
- native transport 每個 POST 新建一條 TCP 連線（對齊 wasmtime-wasi-http 預設），兩者 transport 行為一致。
- 微基準的 WCM 與 native 數字不同日期（見上）。
- native 沒有任何隔離；subprocess 沒有設定 rlimit／cgroup／seccomp，所以它的成本是「最低隔離」的成本，不是「與 WCM 同等隔離」的成本。

## 重現方式

### 環境

- Windows + WSL2：Linux 6.6.87.2-microsoft-standard-WSL2、Ubuntu 24.04，12 邏輯核心，7.5 GiB；clocksource=tsc。
- Rust 1.93.1；Go（`test/compare/capacity/gen_bin`、`httpsink_bin` 不存在時自動編譯）；Python 3.12 + numpy；圖表用 `test/tools/venv`（matplotlib 3.10）。
- 微基準：釘 CPU 2（subprocess 的 child 繼承同一顆核心）。系統層：DUT core 4、sink core 8、generator core 9,10。
- 所有量測單元都在 `flock /home/ckt1022/.wcm-bench.lock` 內獨占執行。
- host binary sha256 `e28f6cd49277a31f62a37c7bc32443c5505fe3dc3ac7c64336d82e24a977b157`（`run_remaining_build.log` 顯示當時沒有重新編譯）。

### 建置

```bash
cd /home/ckt1022/wcm-base-log-forwarder
cargo build --release                                   # host（含 src/native.rs）
(cd test/ours/baselines && cargo build --release)       # micro、stage_child（獨立 crate，#[path] 引用 src/native.rs）
# system/plugin/ 內的 WCM plugin 是既有 Rust plugin（與 correctness／latency 實驗同一份）：
sha256sum test/ours/baselines/system/plugin/*.wasm
```

WCM／Core JSON／Core Direct 的微基準在 `test/ours/copy_overhead/`（見其 `comparison_results/README.md`），本次沒有重跑。

### 執行

```bash
# 微基準（native + subprocess + spawn，3 runs）
CPU=2 RUNS=3 ROUNDS=100 WARMUP=10 INTERVAL=10 bash test/ours/baselines/scripts/run_micro.sh

# 系統層（與 test/tools/run_remaining.sh 步驟 2 相同）
SYS_ID=system_$(date +%Y%m%d_%H%M%S)
RUN_ID="$SYS_ID" MODE=lat RATES=5000 bash test/ours/baselines/scripts/run_system_capacity.sh          # 2 × 約 3 分鐘
RUN_ID="$SYS_ID" MODE=cap RATES="10000 15000 20000 25000 30000 40000 50000 70000 100000" \
    bash test/ours/baselines/scripts/run_system_capacity.sh                                          # 18 × 約 1.5 分鐘
```

### 分析

```bash
python3 test/ours/baselines/scripts/summarize_micro.py test/ours/baselines/results/micro_20261013_181014
test/tools/venv/bin/python test/ours/baselines/scripts/summarize_system.py \
    test/ours/baselines/results/system_system_20261013_235508 \
    test/ours/baselines/results/micro_20261013_181014
# → system_latency.csv（含 stepdetect 偵測結果）、system_capacity.csv、system_max_pass.csv、charts/capacity_ladder.svg、
#   micro 目錄的 five_method_1024_tidy.csv
```
