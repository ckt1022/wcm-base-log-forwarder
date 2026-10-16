# streaming_validation — streaming 處理模式驗證

## 目的

batch 模式（預設）在入口累積「原始輸入」直到 `buffer_size_mb` 或 `max_wait_ms`（W）到期，才整批跑
parse → filter → format → transport，因此延遲 ≈ W + 整批處理時間，超額延遲（p99 − 0.99·W）隨 W 增加
（23 / 62 / 166 / 216 ms @ W = 0.01 / 0.1 / 0.5 / 1 s），W = 1 s 時原始輸入累積造成 OOM
（`../batch_alignment/README.md`）。Fluent Bit／Fluentd／Vector 則是「記錄一到就處理、只把處理後的輸出
緩衝到 flush 計時器」，超額延遲大致固定（17–28 ms）。

新增的 `processing_mode: streaming` 改為同樣的結構，本實驗驗證：
1. 正確性不變（C1–C4 無遺失／重複／亂序）；
2. 超額延遲不再隨 W 成長、W = 1 s 不再 OOM；
3. 容量（PASS 上限）不因 micro-batch 而下降（`../../ours/per_record_invocation/README.md`：k ≥ 16 時每次呼叫的成本可忽略）。

## streaming 模式設計（src/runtime.rs、src/config.rs）

```yaml
batch:
  processing_mode: streaming   # 預設 batch（行為完全不變）
  micro_batch_records: 64      # micro-batch 筆數上限
  micro_batch_max_wait_ms: 5   # micro-batch 自「第一筆到達」起的等待上限
  max_wait_ms: 100             # streaming 時 = 輸出 flush 週期 W
```

- **輸入端**：`parse_dispatcher`（`dispatch_loop` + `DispatchParams`）在 micro-batch 滿 64 筆、或第一筆到達後 5 ms
  即送 parse；`buffer_size_mb` 仍是 byte 上限。parse／filter／format 每個 micro-batch 跑一次（instance 照舊重用），
  parse 失敗的二分隔離（isolation.rs）只作用在該 micro-batch。`PARSE_WORKERS = 1` 且 seq 單調遞增，
  host id = seq × 100,000 + index 仍全域唯一、順序與輸入相同。
- **輸出端**：`transport_router` 在 streaming 時改走 `streaming_router`：formatted 記錄進 per-endpoint buffer，
  (a) 全域週期計時器每 W 觸發一次、所有 buffer 一起送出；(b) 單一 buffer 再加一行會超過 `max_transport_bytes`
  時立即送出（沿用 batch 模式同一套 `route_batch` 規則）。(a) 採固定週期（不是每個 buffer 從第一筆起算），
  與 Fluent Bit `[SERVICE] Flush` 相同；記錄在 buffer 中停留 ∈ [0, W)。錯過的 tick 不補發。
- `processing_mode` 啟動後固定（dispatcher 與 router 必須一致）；`micro_batch_*` 與 `max_wait_ms` 可熱重載。
  memory guard、回壓（sync_channel）、stage timeout、capability 檢查、熱抽換路徑皆未改動。

## 設定檔（`conf/`）

| 檔案 | 來源 | 差異 |
|---|---|---|
| `wcm_stream_W0.01.yaml` | `../batch_alignment/conf/wcm_tw128_wait10.yaml` | 只加 `processing_mode: streaming`、`micro_batch_records: 64`、`micro_batch_max_wait_ms: 5` |
| `wcm_stream_W0.1.yaml` | `wcm_tw128_wait100.yaml` | 同上 |
| `wcm_stream_W0.5.yaml` | `wcm_tw128_wait500.yaml` | 同上 |
| `wcm_stream_W1.yaml` | `wcm_tw128_wait1000.yaml` | 同上 |
| `wcm_stream_correctness_b1.yaml` | `../../ours/latency-benchmark-Rust/forwarder_membound_b1.yaml` | 同上（W = 1 s） |

`capabilities:`：四個 plugin（latency_rust_parse／filter／format、transport.wasm）以 `--check-plugin` 驗證，
在預設 deny 政策下皆 `LOAD_OK`，不需要額外授權，因此設定檔未加 `capabilities:`。

對照組沿用 batch_alignment 原檔：`../batch_alignment/conf/wcm_tw128_wait100.yaml`（image `memguard`）、
`../batch_alignment/conf/fbit_lua_flush_0.1.conf`。

## 腳本

| 腳本 | 說明 |
|---|---|
| `run_all.sh` | 主流程（下方順序），可續傳、支援 STOP |
| `capacity_align.sh` | batch_alignment 複本；WCM image 可用 `WCM_IMAGE` 覆寫，輸出到本目錄 `runs/` |
| `run_point.sh` | batch_alignment 複本；flock 內跑一個完整試驗 + `correct_latency.py`，記錄到 `logs/runs.log` |
| `search_align.sh` | batch_alignment 複本；conf 可給絕對路徑或相對本目錄；trial 之間檢查 STOP |
| `run_wcm_correctness_stream.sh` | correctness/run_wcm_correctness_image.sh 複本；`CONFIG`、`RUN_ROOT`、`REPS_LIST`、`SKIP_BUILD` 可覆寫 |

batch_alignment／correctness 原始腳本未修改；helper（`clock_drift_logger.py`、`eval_trial_align.py`、
`correct_latency.py`、`evaluate.py`、gen／sink binary）直接取自原目錄。

## 執行順序

0. preflight：docker 可連線；`wcm-base-log-forwarder:streaming` 不存在時於鎖內 `docker build`；image ID 記錄到 `logs/images.txt`。
1. 正確性 C1–C4 × 3 reps（streaming image + `wcm_stream_correctness_b1.yaml`）→ `runs/correctness/stream/<C>_rep<n>/`。
2. 方法 3 延遲 @5,000 EPS、streaming、W ∈ {0.01, 0.1, 0.5, 1} → `runs/wcm/lat_rate_5000_stream_W<W>/`。
3. 同 session 對照 @5,000 EPS、W = 0.1：舊模式 WCM（`memguard`）→ `runs/wcm/lat_rate_5000_ctrl_batch_W0.1/`；
   FB Lua → `runs/fbit-lua/lat_rate_5000_ctrl_W0.1/`。
4. 容量搜尋、streaming、W ∈ {0.01, 0.1, 0.5, 1}，起點 22,000 EPS → `runs/wcm/search_stream_W<W>.txt`。
5. 寫 `DONE`（一行摘要）並印到 log。

每個量測單位（一次完整 run，含 container 啟停）都在 `flock /home/ckt1022/.wcm-bench.lock` 內；
每個延遲 run 由 `capacity_align.sh` 全程記錄 `clock_drift.csv` 並以 `correct_latency.py` 校正。

## 預估時間

正確性 12 runs × ~1.2 min ≈ 15 min；延遲 6 runs × ~3.6 min ≈ 22 min；容量搜尋 4 × ~6–8 trials × ~1.5 min ≈ 40–50 min。
合計約 **1.3–1.5 小時**（不含 image 建置；若與其他實驗搶鎖會更久）。

## 重現方式

```bash
cd /home/ckt1022/wcm-base-log-forwarder
# 1. 建置與單元測試（cargo 一律在鎖內）
flock /home/ckt1022/.wcm-bench.lock cargo build --release -j 4
flock /home/ckt1022/.wcm-bench.lock cargo test --release
# 2. image
flock /home/ckt1022/.wcm-bench.lock docker build -t wcm-base-log-forwarder:streaming .
docker image inspect --format '{{.Id}}' wcm-base-log-forwarder:streaming
docker image inspect --format '{{.Id}}' wcm-base-log-forwarder:memguard
#    （本次實際使用的 ID 見 logs/images.txt 與各 run 的 run_config.txt / image.txt）
# 3. plugin capability 檢查
for s in parse:latency_rust_parse filter:latency_rust_filter format:latency_rust_format transport:transport; do
  ./target/release/wcm-base-log-forwarder --check-plugin ${s%%:*} test/ours/latency-benchmark-Rust/plugin/${s#*:}.wasm
done
# 4. 啟動（完全脫離終端）
setsid nohup bash test/compare/streaming_validation/run_all.sh > test/compare/streaming_validation/run_all.log 2>&1 < /dev/null &
sleep 1; pgrep -f 'streaming_validation/run_all.sh' | head -1 > test/compare/streaming_validation/run_all.pid
# 5. 分析
cd test/compare/streaming_validation
cat DONE
cat logs/correctness_summary.txt
for d in runs/*/lat_rate_5000_*; do echo "$d: $(head -1 $d/verdict.txt)"; grep -E 'p50|p99 ' $d/latency_stats_30_150.txt; done
grep -h CAPACITY runs/wcm/search_stream_W*.txt
cat runs/*/lat_rate_5000_*/subwindows_clockfix.txt     # 每 30 s 子窗 p50/p99 與時鐘階梯
```

## 監看

```bash
cd /home/ckt1022/wcm-base-log-forwarder/test/compare/streaming_validation
tail -f run_all.log            # 主流程
tail -f logs/runs.log          # 每個量測單位完成一行（rc、耗時、verdict）
tail -f logs/lat.out logs/search.out logs/correctness.out   # 各步驟詳細輸出
kill -0 $(cat run_all.pid) && echo running
touch STOP                     # 於下一個點之前停止（目前的點會跑完）；rm STOP 後重跑即續傳
kill $(cat run_all.pid)        # 立即停止（目前 run 的 container 由 trap 清理；必要時 docker rm -f align-wcm）
```
