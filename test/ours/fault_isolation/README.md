# Fault isolation 實驗（Item 3／Item 4：單一中毒記錄對 host 的影響）

在穩定負載下，對 parse stage 注入**一筆**觸發故障的記錄，觀察 WCM host：
是否存活、遺失幾筆、多久恢復吞吐、故障期間 CPU／RSS，以及 `error.txt` 與 stderr 的內容。

- parse：`parse_chaos.wasm`（`test-plugins/rust-plugin/parse_chaos`），正常記錄的處理與 Rust parser 相同，
  `msg` 含 `FAULT_*` marker 時在處理「該行」時觸發故障。
- filter／format／transport：正式 Rust WCM plugin（與 latency／correctness 實驗同一份，sha256 見下）。
- 負載：5,000 EPS × 1 KiB JSON，40 s，單一 TCP 連線；每兩筆一筆 `warn`，filter 保留 warn+，所以預期輸出 = 送出數 / 2。
- marker 在 generator 第 15 s 注入（取代第 75,000 筆，它本來就是 warn）。
- 設定 `forwarder_fault.yaml`：`stage_timeout_ms: 2000`（讓無窮迴圈在實驗時間尺度內被 epoch 中止）、
  `parse_isolation: true`、`max_wait_ms: 1000`、`buffer_size_mb: 1`、`mem_limit_mb: 128`；
  chaos plugin 因 Rust std（wasip2）連帶 import filesystem 與 sockets，設定 `capabilities.parse: {filesystem: inert, sockets: inert}`
  （連結介面，但零 preopen、位址一律拒絕）。filter／format／transport 維持預設 deny。

| marker | 故障 | 預期的 host 防線 |
|---|---|---|
| `FAULT_LOOP` | 無窮迴圈 | epoch deadline → trap |
| `FAULT_MEM` | 每次 32 MiB 無上限配置 | `ResourceLimiter` 拒絕 `memory.grow` → guest 配置失敗 → abort trap |
| `FAULT_PANIC` | `panic!()` | unreachable trap |
| `FAULT_OOB` | 越界索引 | Rust bounds-check panic → trap |
| `FAULT_FILE` | `std::fs::read_to_string("/etc/passwd")` | 零 preopen，開檔失敗（回傳一筆說明訊息，不 trap） |
| `FAULT_ENV` | `std::env::vars()` | WasiCtx 不注入任何環境變數（回傳一筆說明訊息，不 trap） |

**沒有網路／socket 的故障 case。** 見下方「限制」。

## 結果（`runs/native_faults/`，原生執行，每個 case 1 次）

整理檔：[`runs/native_faults/summary.csv`](runs/native_faults/summary.csv)、[`runs/native_faults/throughput_timeline.csv`](runs/native_faults/throughput_timeline.csv)、
圖 [`runs/native_faults/charts/throughput_timeline.svg`](runs/native_faults/charts/throughput_timeline.svg)。

| case | host | 收到／預期 | 遺失 | 隔離結果 | 輸出為 0 的秒數 | 吞吐恢復 | 故障視窗 CPU 峰值（故障前中位數） | 額外 CPU（核心·秒） |
|---|---|---:|---:|---|---:|---:|---:|---:|
| baseline | 存活 | 100,000／100,000 | 0 | — | 0 | 無掉點 | 48.8%（26.5%） | 1.3（雜訊水準） |
| loop | 存活 | 99,999／100,000 | 1 | 1025 行中保留 1024、拒絕 1；整批重試 2 次 + 20 次 trap 探測 | 7 | **9 s** | 124.2%（27.2%） | **8.2** |
| mem | 存活 | 99,999／100,000 | 1 | 同上（20 次 trap 探測） | 0 | **1 s** | 97.3%（20.0%） | 1.1 |
| panic | 存活 | 99,999／100,000 | 1 | 同上 | 0 | < 1 s（1 s 解析度看不到掉點） | 21.5%（18.5%） | 0.1 |
| oob | 存活 | 99,999／100,000 | 1 | 同上 | 0 | < 1 s | 23.0%（16.8%） | 0.3 |
| file | 存活 | 100,000／100,000 | 0 | 沒有 trap，plugin 正常回傳 | 0 | 無掉點 | 21.0%（16.2%） | 0.2 |
| env | 存活 | 100,000／100,000 | 0 | 沒有 trap，plugin 正常回傳 | 0 | 無掉點 | 21.1%（16.1%） | 0.2 |

- **唯一遺失的就是中毒記錄本身**。四個 trap 類 case 都是：整批（1025 行）先依 `max_retries` 重試 2 次，仍 trap 才進入有上限的二分（`[trap] exceeded retries, isolating batch 74 (1025 lines) by bounded bisection`），
  20 次探測後只拒絕 1 行（`budget_exhausted=false`），其餘 1024 行照常輸出；沒有重複、沒有遺漏其他記錄。
- **loop 最貴**：每次 trap 都要等 epoch deadline。整批 2 次 × 2 s，加上二分探測（deadline 依區段長度等比例縮短，下限 100 ms），
  parse worker 停了約 8 s；sink 連續 7 秒收到 0 筆，第 9 秒一次補送 24,087 筆積壓（channel_capacity 50,000 ≥ 8 s × 5,000 EPS，所以沒有掉資料）。
  若停頓更久或 rate 更高，積壓會超過 channel 容量而對 generator 施加回壓。
- **mem**：`ResourceLimiter` 擋下 `memory.grow`，guest 印出 `memory allocation of 33554432 bytes failed` 後 abort；每次探測都很快失敗，所以只掉 1 秒。
- **panic／oob**：trap 立即發生，整批 2 次 + 20 次探測總共不到 1 s，在 1 s 解析度下看不到吞吐下降。
- **file／env**：plugin 正常回傳一筆說明訊息（`FAULT_FILE blocked: …` 或 `FAULT_ENV result: N vars visible`），host 沒有任何錯誤行。
  **但 sink 在這個實驗沒有保存 body**（`-parse-ts=false`，只有每秒計數），所以「開檔失敗」與「0 個環境變數」**沒有被資料直接證實**，只證實了 host 存活、沒有 trap、沒有遺失。
  能力限制本身由單元測試（`restricted_linker_cannot_satisfy_filesystem_import`、`default_policy_rejects_filesystem_and_names_every_import`）與載入檢查（見 `優先補強項目報告.md`）證實。

### error.txt 與 stderr

| case | `error.txt`（2 行：摘要 + 原始行） | stderr（`dut.log`） |
|---|---|---|
| loop | `[parse-isolate] batch=74 拒絕 1/1025 筆（index:原因）168:poison(trap/timeout)` + 該筆原始 JSON | `[worker0][trap 1/2]`、`[trap 2/2]`（wasm backtrace）、`exceeded retries … bounded bisection`、`[parse-isolate] … salvaged=1024 rejected=1 calls=20 trap_calls=20` |
| mem | 同上（index 75） | 每次探測一行 `memory allocation of 33554432 bytes failed`，其餘同上 |
| panic | 同上（index 75） | `panicked at src/lib.rs:67:9: parse_chaos: FAULT_PANIC marker`，其餘同上 |
| oob | 同上（index 75） | `panicked at src/lib.rs:71:20: index out of bounds: the len is 3 but the index is 1000000`，其餘同上 |
| file／env／baseline | 不存在 | 只有啟動訊息 |

### CPU／RSS 數字的可信度（重要）

`run_fault.sh` 用 `sample_stats.sh … tree` 取樣。tree 模式在 2026-10-14 之前有一個 awk bug：`keep[ppid[i]]` 這種讀取會建立空鍵，
`for (p in keep)` 因此把**系統上所有的父程序**都算進來。證據：baseline 第一個取樣的累計 CPU 時間已經是 974 s、RSS 約 3.7 GB；
對一個 `sleep` 取樣也得到 3.3 GB／1,064 s。所以：

- **RSS：本次資料無效**（CSV 欄名帶 `_INVALID`，只保留供稽核）。mem case 的 RSS 變化（+7.7 MiB）與 baseline 的波動（±30 MiB）分不開。
  記憶體上限的證據改用 stderr：`memory allocation … failed` 表示 `ResourceLimiter` 在 `mem_limit_mb: 128` 擋下了成長。
- **CPU：只採相對值**。原始 CPU% = DUT + 背景程序；背景程序進出取樣集合時會出現 −139% 或 +170% 的假值。
  `summarize_fault.py` 排除 `cpu_pct < 0`、`> 150` 以及取樣間隔 > 2.2 s（疑似 wall-clock 跳躍）的點，並以「故障視窗 − 故障前中位數」表示故障帶來的額外 CPU。
  loop 的 8.2 核心·秒與「約 8 s 單核滿載自旋」吻合。
- `sample_stats.sh` 已修正（改用 `(x in keep)`），之後重跑的資料不會有這個問題。

### 時鐘

- 吞吐與恢復時間來自 sink 的 `recv_per_sec.csv`，sink 用 Go 的 `time.Since`（monotonic clock）分秒，**不受 WSL2 wall-clock 跳躍影響**。
- 本實驗沒有逐筆延遲檔，所以 `stepdetect.py` 不適用。`stats.csv` 用 `EPOCHREALTIME`（wall clock）計算取樣間隔，
  每個 case 有 1–2 個間隔 2.3–3.3 s 的取樣（正常約 1.4 s），疑似時鐘跳躍；這些點已從 CPU 統計中排除（見 `cpu_samples_excluded`）。

## 附：現有 plugin 在新能力政策下能否載入（`plugin_load_check.csv`）

以 release binary 的 `--check-plugin <stage> <plugin.wasm> [inert-fs] [inert-sockets|endpoint-sockets]` 逐一檢查（載入期 import 檢查 + `instantiate_pre`，不執行 plugin）：

- **預設（全部 deny）即可載入：** Rust parse／filter／format／transport（所有正式實驗用的那一組）、C noop parser、C latency filter。
- **需要 `filesystem: inert`：** 所有經 wasip1 adapter 編出的 C／Go／C# plugin（C latency parser／format／transport、C access parser、C http transport、Go parser×4／format、C# parser×3）。
- **需要 `filesystem: inert` + `sockets: inert`：** `parse_chaos.wasm`、C# filter（runtime 會 import tcp／udp 型別）。
- **舊的 raw-socket C transport**（`test-plugins/c-plugin/transport/latency_c_transport.wasm`）：`sockets: inert` 就能載入，但要真正連線必須 `sockets: endpoint`（只放行 endpoint map 的 host:port）。
- **任何授權都拒絕：** `parser_c_bad.wasm`（import `local:log-process/forbidden-capability`，這是刻意的負面測試）；`test-plugins/c-plugin/parse/parser_c_json.wasm`（**不是 wasm**，內容是 WIT 文字，修改前就無法載入）。
- 因此 `test/ours/latency-benchmark-C/forwarder.yaml`、根目錄的 `forwarder.yaml`／`forwarder_size.yaml` 在沒有加上 `capabilities:` 區段前，**會在啟動時被拒絕**。

重現：見 `優先補強項目報告.md` 的 Item 3 指令（對每個 plugin 執行上面的 `--check-plugin`）。

## 限制與缺漏

- **每個 case 只跑 1 次**，原生執行（無容器、無 cgroup），DUT 釘 core 4。沒有容器版。
- **沒有網路故障 case**（例如 `FAULT_NET` 嘗試 TCP connect）。chaos plugin 以 `sockets: inert` 載入，「連結但位址全拒」這條路徑**沒有被任何實驗或單元測試在執行期驗證**；
  單元測試只驗證載入期的 import 分類（`sockets_denied_everywhere_by_default_including_transport`）與 wasi:http 白名單（`http_guard_only_allows_endpoint_map`）。
- file／env 的結果內容沒有被 sink 保存（見上）。
- RSS 無效、CPU 只有相對值（見上）。
- 恢復時間的解析度是 1 s；故障注入時刻在 sink 時間軸上只能近似對齊（generator 與 sink 各自從 0 起算）。
- 只測了 parse stage；filter／format／transport 的故障沒有測。

## 重現方式

### 環境

- Windows + WSL2：Linux 6.6.87.2-microsoft-standard-WSL2、Ubuntu 24.04，12 個邏輯核心，7.5 GiB 記憶體；clocksource=tsc（wall clock 每 30–40 s 會跳 0.6–1.2 s）。
- Rust 1.93.1（host 與 plugin）、`wasm32-wasip2` target；Go（建置 `test/compare/capacity/httpsink_bin`，不存在時腳本會自動編譯）；Python 3.12。
- 釘核：DUT core 4、sink core 8、generator core 9,10；每個 case 在 `flock /home/ckt1022/.wcm-bench.lock` 內獨占執行。
- 本次使用的 host binary：`target/release/wcm-base-log-forwarder`，sha256 `e28f6cd49277a31f62a37c7bc32443c5505fe3dc3ac7c64336d82e24a977b157`（每個 case 的 `meta.txt` 都有記錄）。
- plugin sha256：

  | 檔案 | sha256 |
  |---|---|
  | `parse_chaos.wasm` | `2558f7246df70e7fe4cf10c6f15b0b102969adcca040fb0aeb7aeee9f4b72594` |
  | `latency_rust_filter.wasm` | `28c8086345847e73a0465a479a42f0befdd36eddbcf7af301771012c6caeb3f9` |
  | `latency_rust_format.wasm` | `096462091f2cd9b0d2775172e85b4e5d05fb047243c6d6bbb92c88c5f6235421` |
  | `transport.wasm` | `51529f848173cf696ab01aee376009ae58288a3b4e82362412071885baca51a4` |

### 建置

```bash
cd /home/ckt1022/wcm-base-log-forwarder
cargo build --release                      # host
# chaos parser（wit/ 是 repo wit/log_plugin.wit 加上 parser-plugin-chaos world 的複本）
cd test-plugins/rust-plugin/parse_chaos
rustup target add wasm32-wasip2            # 只需一次
cargo build --release --target wasm32-wasip2
cp target/wasm32-wasip2/release/parse_chaos.wasm parse_chaos.wasm   # plugin/parse_chaos.wasm 是指向這裡的 symlink
cd -
sha256sum target/release/wcm-base-log-forwarder test/ours/fault_isolation/plugin/*.wasm
# 確認 chaos plugin 只在 inert 授權下能載入（預設應被拒絕）
target/release/wcm-base-log-forwarder --check-plugin parse test/ours/fault_isolation/plugin/parse_chaos.wasm
target/release/wcm-base-log-forwarder --check-plugin parse test/ours/fault_isolation/plugin/parse_chaos.wasm inert-fs inert-sockets
```

filter／format／transport 是 `test/ours/latency-benchmark-Rust/plugin/` 的既有 Rust plugin（`plugin/` 內是 symlink），不需重建。

### 執行（每個 case 約 1 分鐘）

```bash
RUN_ID=native_faults bash test/ours/fault_isolation/scripts/run_all_faults.sh
# 或單一 case：
CASE=loop MARKER=FAULT_LOOP RUN_ID=native_faults bash test/ours/fault_isolation/scripts/run_fault.sh
# 可調：EPS=5000 SECS=40 FAULT_AT=15 MARKER_COUNT=1 DUT_CORE=4 SINK_CORE=8 GEN_CORE=9,10
```

### 分析

```bash
test/tools/venv/bin/python test/ours/fault_isolation/scripts/summarize_fault.py test/ours/fault_isolation/runs/native_faults
# → runs/native_faults/summary.csv、throughput_timeline.csv、charts/throughput_timeline.svg
# （系統 python3 沒有 matplotlib 時仍會產生 CSV，只略過圖表）
```
