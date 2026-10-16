# 正確性重跑：逐筆錯誤隔離（Item 4）前後比較

比較 parse 階段「整批丟棄」與「host 二分隔離」（`src/isolation.rs`、`src/runtime.rs`）在正確性情境 C1–C4 的結果，
並確認新程式碼在 C1、C2、C4 沒有退步。

情境（`../generator`、`../validationsink`、`../evaluate.py`）：1024 B／36 欄位，filter 保留 warn+（40%），
C1 合法輸入 2,500 EPS；C2 合法輸入 17,500 EPS；C3 確定性 5% malformed JSON（每 run 5,000 筆）；C4 布林／整數／浮點／null／Unicode／引號／反斜線邊界值。
每個情境 3 次。PASS = 收到數 = 預期數，且沒有重複、損毀、亂序。

## 資料來源

| 組別 | 目錄 | 環境 | 程式 |
|---|---|---|---|
| container_before | `runs/wcm_correctness_formal_20260804`（C1–C3）、`runs/wcm_correctness_edge_20260804`（C4） | 容器 1 CPU／512 MiB | 修改前（`memguard` image） |
| native_legacy | `runs/native_legacy_c3` | 原生程序，taskset：DUT core 4、sink 8、gen 9,10 | 新 binary，`parse_isolation: false`（舊行為） |
| native_isolation | `runs/native_isolation_c1c3c4` | 同上 | 新 binary，`parse_isolation: true` |
| container_isolation | `runs/isolation_docker_20261014_002704` | 容器 1 CPU／512 MiB | `wcm-base-log-forwarder:isolation` image（`sha256:02ee1a41…5e3`，新程式碼） |

整理檔：[`comparison.csv`](comparison.csv)（每 run 一列）、[`comparison_by_scenario.csv`](comparison_by_scenario.csv)（每組 × 情境）、
圖 [`charts/c3_before_after.svg`](charts/c3_before_after.svg)。

## 結果

| 情境 | 組別 | PASS | 收到／預期（3 run 合計） | 遺失 | 被拒絕行數／run | 二分 parse 呼叫數／run | CPU 平均 | 記憶體峰值 |
|---|---|---:|---:|---:|---:|---:|---:|---:|
| **C3** | container_before | 0/3 | 0／114,000 | 114,000 | — | — | 2.5% | 13.0 MiB |
| **C3** | native_legacy | 0/3 | 0／114,000 | 114,000 | 0 | 0 | 2.4% | 26.8 MiB |
| **C3** | native_isolation | **3/3** | **114,000／114,000** | 0 | 5,000 | 52,308 | 29.9% | 42.1 MiB |
| **C3** | container_isolation | **3/3** | **114,000／114,000** | 0 | 5,000 | 52,308／52,308／52,312 | 18.7% | 33.2 MiB |
| C1 | container_before | 3/3 | 120,000／120,000 | 0 | | | 14.3% | 29.9 MiB |
| C1 | native_isolation | 3/3 | 120,000／120,000 | 0 | 0 | 0 | 19.9% | 41.9 MiB |
| C1 | container_isolation | 3/3 | 120,000／120,000 | 0 | 0 | 0 | 13.0% | 30.2 MiB |
| C2 | container_before | 3/3 | 1,200,000／1,200,000 | 0 | | | 78.5% | 30.9 MiB |
| C2 | container_isolation | 3/3 | 1,200,000／1,200,000 | 0 | 0 | 0 | 77.2% | 31.0 MiB |
| C4 | container_before | 3/3 | 120,000／120,000 | 0 | | | 13.7% | 29.2 MiB |
| C4 | native_isolation | 3/3 | 120,000／120,000 | 0 | 0 | 0 | 21.7% | 41.5 MiB |
| C4 | container_isolation | 3/3 | 120,000／120,000 | 0 | 0 | 0 | 16.0% | 30.1 MiB |

- **C3 修正前（容器、原生舊模式）：每 run 99 個批次全部回 parse-error，114,000 筆預期輸出全數遺失。** 原生舊模式重現了容器的結果，證明 0 筆是 host 行為造成的，與容器無關。
- **C3 修正後（原生、容器）：3/3 PASS。** 99 個受污染批次全部被二分；每 run 拒絕的 5,000 行正好等於注入的 5,000 筆 malformed，
  保留 95,000 筆 parse 結果，filter 後 38,000 筆全數送達，沒有重複、損毀或亂序。被拒絕的行連同 index 寫入 `error.txt`（原生版每 run 一個；容器版沒有掛出 error.txt）。
- **代價：** 每個受污染批次（約 1,024 行、約 51 行 malformed）平均要 528 次 parse 呼叫，每 run 共 52,308 次。
  在 2,500 EPS 下，CPU 從 C1 的 13.0% 升到 18.7%（容器），原生從 19.9% 升到 29.9%。5% 是很密的錯誤率；錯誤越稀疏，呼叫數越接近 2·log2(n)·錯誤數。
- **C1、C2、C4 沒有退步：** 全部 3/3 PASS；容器版 CPU 與記憶體和 08-04 的正式值相近（C1 13.0% vs 14.3%、C2 77.2% vs 78.5%、C4 16.0% vs 13.7%）。
- 原生與容器的 CPU／記憶體**不可並列**：原生用 `/proc` 程序取樣、沒有 cgroup，容器用 cgroup 取樣。原生版只用於「同腳本前後比較」。
- 原生版沒有跑 C2（`native_isolation_c1c3c4` 只有 C1、C3、C4）；C2 只在容器版重跑。

### 修正的說明文字

`runs/native_isolation_c1c3c4/summary.md` 原本寫「every contaminated batch was discarded」，但表格是 38,000／38,000 全數送達。
原因是 `../summarize.py` 把這句寫死。已改成依 `dut.log` 的 `[parse-isolate]` 行判斷，並輸出 salvaged／rejected／呼叫數；
同時依 `meta.txt` 區分原生與容器的環境描述。三個新目錄的 `summary.md`／`summary.csv` 都已重新產生；
08-04 的正式目錄**沒有**重新產生（內容不變）。

## 限制

- 每組 3 次；原生版缺 C2。
- 容器版沒有保存 `error.txt`（無法逐行核對被拒絕的行，只能用 `dut.log` 的計數）。
- 二分只在 parse 回 `Err(parse-error)` 或 trap 時啟動；plugin 若「靜默」產生錯誤輸出，不在本機制範圍內。
- 長期較乾淨的作法是把 WIT 改為逐筆結果 `list<result<parsed-entry, parse-error>>`，一次呼叫即可回報每筆成敗（見 `src/isolation.rs` 開頭說明）；本次刻意不改簽名以保持既有 plugin 相容。
- `correctgen_bin`／`validationsink_bin` 在 git 中顯示為已修改：`run_wcm_correctness_image.sh` 每次執行都會用目前的 Go 工具鏈重新 `go build`，
  所以二進位大小與 commit 中的不同（原始碼沒有改）。

## 重現方式

### 環境

- Windows + WSL2：Linux 6.6.87.2-microsoft-standard-WSL2、Ubuntu 24.04，12 邏輯核心；Docker 29.8.0；Go（每次執行時重新編譯 generator／sink）；Python 3.12。
- 原生版：binary sha256 `e28f6cd49277a31f62a37c7bc32443c5505fe3dc3ac7c64336d82e24a977b157`，DUT core 4、sink core 8、generator core 9,10（見各目錄 `meta.txt`）。
- 容器版：image `wcm-base-log-forwarder:isolation`，Id `sha256:02ee1a41333117d5468e5060378332bb17cf076d66bc860427c2e5686999e5f3`（`test/tools/isolation_image_id.txt`），1 CPU／512 MiB。
- plugin（兩版相同，`native/plugin/`）：Rust parse／filter／format／transport，sha256 見 `sha256sum test/compare/correctness/native/plugin/*`。
- 每一輪都在 `flock /home/ckt1022/.wcm-bench.lock` 內獨占執行。

### 建置

```bash
cd /home/ckt1022/wcm-base-log-forwarder
cargo build --release                                            # 原生版用的 host
docker build -t wcm-base-log-forwarder:isolation .               # 容器版 image（從目前的 src/、wit/ 建置）
docker image inspect --format '{{.Id}}' wcm-base-log-forwarder:isolation
```

### 執行

```bash
# 原生，修正前行為（parse_isolation: false）
VARIANT=legacy RUN_ID=native_legacy_c3 SCENARIOS=C3 REPEATS=3 \
    bash test/compare/correctness/run_wcm_correctness_native.sh
# 原生，修正後（parse_isolation: true）
VARIANT=isolation RUN_ID=native_isolation_c1c3c4 SCENARIOS="C1 C3 C4" REPEATS=3 \
    bash test/compare/correctness/run_wcm_correctness_native.sh
# 容器（與 test/tools/run_remaining.sh 步驟 3 相同）
COR_ID=isolation_docker_$(date +%Y%m%d_%H%M%S)
for sc in C3 C1 C2 C4; do
  flock /home/ckt1022/.wcm-bench.lock env IMAGE=wcm-base-log-forwarder:isolation RUN_ID="$COR_ID" REPEATS=3 SCENARIOS="$sc" \
      bash test/compare/correctness/run_wcm_correctness_image.sh
done
```

### 分析

```bash
python3 test/compare/correctness/summarize.py test/compare/correctness/runs/native_legacy_c3
python3 test/compare/correctness/summarize.py test/compare/correctness/runs/native_isolation_c1c3c4
python3 test/compare/correctness/summarize.py test/compare/correctness/runs/isolation_docker_20261014_002704
test/tools/venv/bin/python test/compare/correctness/isolation_comparison/compare.py
```
