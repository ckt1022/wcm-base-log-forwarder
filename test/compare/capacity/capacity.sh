#!/bin/bash
# capacity.sh — 單一容量/延遲試驗:啟動 sink → DUT → gen 定速打流量 → 判定
#
# 用法: ./capacity.sh <dut> <rate> [模式]
#   dut : fluentd | fbit-lua | fbit-wasm | vector
#   rate: gen 輸入速率(lines/s)
#   模式: cap(預設,60s 負載、sink 不解析 ts)| lat(300s 負載、sink 記錄延遲)
#
# 環境變數:
#   DOCKER(預設 docker;無權限時可 DOCKER="sudo docker")
#   CPU_LIMIT(預設 1) MEM_LIMIT(預設 512m) FLUSH_INTERVAL(預設 1)
#   LOAD_SECS(覆寫負載秒數) SINK_CORE(預設 8) GEN_CORES(預設 9,10)
#
# 固定變因(所有 DUT 相同):
#   輸入 TCP :5140、json-fixed5、gen batch 500 行/20ms flush、單連線、
#   HTTP sink :8080、docker --network host --cpus/--memory/--memory-swap、
#   各產品用自己的 latency conf(不修改;fluent bit 的 Flush 由本腳本 sed,
#   與 run_all_*.sh 相同機制)
#
# 判定(cap 模式,eval_trial.py):
#   1. gen 未被 backpressure 卡住:avg ≥ 0.995R 且 send_lag p99 ≤ 10ms
#   2. 負載結束瞬間 DUT 內部積壓 ≤ 3 秒份(0.4R×3)
#   3. drain 後 sink 收到 ≥ 99.9% 的預期行數(0.4×gen_total,無掉資料)
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
COMPARE_DIR="$REPO_ROOT/test/compare"

DUT="${1:?usage: capacity.sh <dut> <rate> [cap|lat]}"
RATE="${2:?usage: capacity.sh <dut> <rate> [cap|lat]}"
MODE="${3:-cap}"

DOCKER="${DOCKER:-docker}"
CPU_LIMIT="${CPU_LIMIT:-1}"
MEM_LIMIT="${MEM_LIMIT:-512m}"
FLUSH_INTERVAL="${FLUSH_INTERVAL:-1}"
SINK_CORE="${SINK_CORE:-8}"
GEN_CORES="${GEN_CORES:-9,10}"
GEN_MODE="${GEN_MODE:-json-fixed5}"
RUN_VARIANT="${RUN_VARIANT:-}"

if [[ "$MODE" == "lat" ]]; then
    LOAD_SECS="${LOAD_SECS:-300}"
    PARSE_TS="true"
    # 預配置延遲記錄容量:0.4×rate×secs×1.1,避免高流量時 slice 成長的記憶體尖峰
    PREALLOC=$(( RATE * 4 / 10 * LOAD_SECS * 11 / 10 ))
else
    LOAD_SECS="${LOAD_SECS:-60}"
    PARSE_TS="false"
    PREALLOC=0
fi
DRAIN_SECS="${DRAIN_SECS:-20}"

TCP_PORT=5140
SINK_PORT=8080
CONTAINER="cap-$DUT"

# ─── 各 DUT 的 conf 與啟動參數 ───────────────────────────────────────────────
FB_LAT="$COMPARE_DIR/fluentbit/latency"
FD_LAT="$COMPARE_DIR/fluentd/latency"
VT_LAT="$COMPARE_DIR/vector/latency"
WCM_LAT="$REPO_ROOT/test/ours/latency-benchmark-Rust"

case "$DUT" in
  fluentd)
    IMAGE="fluent/fluentd:v1.16-debian-1" ;;
  fbit-lua|fbit-wasm)
    IMAGE="fluent/fluent-bit:5.0.9" ;;
  vector)
    IMAGE="timberio/vector:0.49.0-alpine" ;;
  wcm|wcm-v2|wcm-tw128|wcm-flush10ms|wcm-b*)
    # WCM forwarder:flush 窗固定在 forwarder.yaml 的 max_wait_ms=1000,
    # 不由環境變數注入;FLUSH_INTERVAL != 1 時擋下,避免與其他 DUT 比較失準
    # wcm    = 原版(image latest + forwarder.yaml,buffer 128MB)
    # wcm-v2 = memory-bounded 版(image memguard + forwarder_membound.yaml:
    #          buffer 32MB + app.rs RSS 水位線保險絲)
    if [[ "$DUT" == "wcm-v2" ]]; then
        IMAGE="wcm-base-log-forwarder:memguard"
        WCM_YAML="$WCM_LAT/forwarder_membound.yaml"
    elif [[ "$DUT" == "wcm-tw128" ]]; then
        # 對照組:與 wcm-v2 唯一差異 = buffer 8→128MB(時間窗主導,對齊其他產品節奏)
        IMAGE="wcm-base-log-forwarder:memguard"
        WCM_YAML="$WCM_LAT/forwarder_membound_tw128.yaml"
    elif [[ "$DUT" == "wcm-flush10ms" ]]; then
        # 對照組:與 tw128 唯一差異 = max_wait_ms 1000→10(對齊其他產品 flush=0.01s)
        IMAGE="wcm-base-log-forwarder:memguard"
        WCM_YAML="$WCM_LAT/forwarder_membound_flush10ms.yaml"
    elif [[ "$DUT" == wcm-b* ]]; then
        # buffer 大小掃描變體(wcm-b<N> → forwarder_membound_b<N>.yaml),其餘與 wcm-v2 相同
        IMAGE="wcm-base-log-forwarder:memguard"
        WCM_YAML="$WCM_LAT/forwarder_membound_${DUT#wcm-}.yaml"
    else
        IMAGE="wcm-base-log-forwarder:latest"
        WCM_YAML="$WCM_LAT/forwarder.yaml"
    fi
    if [[ "$FLUSH_INTERVAL" != "1" ]]; then
        echo "[cap] ERROR: wcm 的 flush 窗固定 1s(forwarder.yaml max_wait_ms=1000),FLUSH_INTERVAL=$FLUSH_INTERVAL 不支援" >&2
        exit 2
    fi
    ;;
  *) echo "unknown dut: $DUT" >&2; exit 2 ;;
esac

RUN_SUFFIX=""
[[ -n "$RUN_VARIANT" ]] && RUN_SUFFIX="_${RUN_VARIANT}"
OUT_DIR="$SCRIPT_DIR/runs/$DUT/${MODE}_rate_${RATE}${RUN_SUFFIX}"
mkdir -p "$OUT_DIR"
rm -f "$OUT_DIR"/*.log "$OUT_DIR"/*.csv "$OUT_DIR"/verdict.txt 2>/dev/null || true

# ─── 時鐘檢查(同 latency.sh)──────────────────────────────────────────────────
cs=$(cat /sys/devices/system/clocksource/clocksource0/current_clocksource 2>/dev/null || echo unknown)
tsd=$(systemctl is-active systemd-timesyncd 2>/dev/null || true)
if [[ "$cs" != "hyperv_clocksource_tsc_page" || "$tsd" == "active" ]]; then
    echo "[cap] WARNING: 時鐘狀態異常 clocksource=$cs timesyncd=$tsd" >&2
    [[ "${SKIP_CLOCK_CHECK:-0}" == "1" ]] || exit 1
fi

SINK_PID=""
LOGS_PID=""
PROG_PID=""
STATS_PID=""
_DONE=0
cleanup() {
    (( _DONE )) && return; _DONE=1
    [[ -n "$PROG_PID" ]] && kill "$PROG_PID" 2>/dev/null || true
    if [[ -n "$STATS_PID" ]]; then
        kill "$STATS_PID" 2>/dev/null || true
        wait "$STATS_PID" 2>/dev/null || true
    fi
    $DOCKER stop -t 10 "$CONTAINER" >/dev/null 2>&1 || true
    # OOM 檢查要在 rm 之前
    OOM=$($DOCKER inspect -f '{{.State.OOMKilled}}' "$CONTAINER" 2>/dev/null || echo unknown)
    echo "$OOM" > "$OUT_DIR/oom.txt"
    $DOCKER rm "$CONTAINER" >/dev/null 2>&1 || true
    [[ -n "$LOGS_PID" ]] && kill -TERM "$LOGS_PID" 2>/dev/null || true
    if [[ -n "$SINK_PID" ]] && kill -0 "$SINK_PID" 2>/dev/null; then
        kill -TERM "$SINK_PID" 2>/dev/null || true
        for _ in $(seq 40); do kill -0 "$SINK_PID" 2>/dev/null || break; sleep 0.5; done
        kill -9 "$SINK_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT INT TERM

$DOCKER rm -f "$CONTAINER" >/dev/null 2>&1 || true

# ─── 1. sink ─────────────────────────────────────────────────────────────────
SINK_BIN="$SCRIPT_DIR/httpsink_bin"
[[ -x "$SINK_BIN" ]] || ( cd "$SCRIPT_DIR/httpsink" && go build -o "$SINK_BIN" main.go )
SINK_EPOCH=$(date +%s.%N)
taskset -c "$SINK_CORE" "$SINK_BIN" -port "$SINK_PORT" -out-dir "$OUT_DIR" -parse-ts="$PARSE_TS" \
    -prealloc "$PREALLOC" \
    > "$OUT_DIR/sink.log" 2>&1 &
SINK_PID=$!
echo "$SINK_EPOCH" > "$OUT_DIR/sink_epoch.txt"
echo "[cap] sink 已啟動(:$SINK_PORT,log=$OUT_DIR/sink.log)"

# ─── 2. DUT ──────────────────────────────────────────────────────────────────
echo "[cap] dut=$DUT rate=$RATE mode=$MODE load=${LOAD_SECS}s flush=${FLUSH_INTERVAL}s cpus=$CPU_LIMIT mem=$MEM_LIMIT"
case "$DUT" in
  fluentd)
    FD_CONF="${CONF_OVERRIDE:-$FD_LAT/conf/latency.conf}"
    $DOCKER run -d --name "$CONTAINER" --network host \
        --cpus "$CPU_LIMIT" --memory "$MEM_LIMIT" --memory-swap "$MEM_LIMIT" \
        -e FLUSH_INTERVAL="$FLUSH_INTERVAL" \
        -v "$FD_CONF:/fluentd/etc/fluent.conf:ro" \
        -v "$FD_LAT/plugins:/fluentd/plugins:ro" \
        "$IMAGE" >/dev/null
    ;;
  fbit-lua)
    # Flush 由 sed 注入(與 run_all_lua.sh 同機制);CONF_OVERRIDE 可指向 tuned 複本,
    # 避免 sed 就地修改主要實驗一直沿用的 conf 檔
    FB_CONF="${CONF_OVERRIDE:-$FB_LAT/conf/fluentbit_latency.conf}"
    sed -i -E "s|^( *Flush +).*|\1${FLUSH_INTERVAL}|" "$FB_CONF"
    $DOCKER run -d --name "$CONTAINER" --network host \
        --cpus "$CPU_LIMIT" --memory "$MEM_LIMIT" --memory-swap "$MEM_LIMIT" \
        -v "$FB_LAT:$FB_LAT:ro" -w "$FB_LAT/conf" \
        "$IMAGE" -c "$FB_CONF" >/dev/null
    ;;
  fbit-wasm)
    FB_CONF="${CONF_OVERRIDE:-$FB_LAT/conf/fluentbit_latency_wasm.conf}"
    sed -i -E "s|^( *Flush +).*|\1${FLUSH_INTERVAL}|" "$FB_CONF"
    $DOCKER run -d --name "$CONTAINER" --network host \
        --cpus "$CPU_LIMIT" --memory "$MEM_LIMIT" --memory-swap "$MEM_LIMIT" \
        -v "$FB_LAT:$FB_LAT:ro" -w "$FB_LAT/conf" \
        "$IMAGE" -c "$FB_CONF" >/dev/null
    ;;
  vector)
    VT_CONF="${CONF_OVERRIDE:-$VT_LAT/conf/latency.yaml}"
    $DOCKER run -d --name "$CONTAINER" --network host \
        --cpus "$CPU_LIMIT" --memory "$MEM_LIMIT" --memory-swap "$MEM_LIMIT" \
        -e FLUSH_INTERVAL="$FLUSH_INTERVAL" \
        -e VECTOR_BATCH_MAX_BYTES="${VECTOR_BATCH_MAX_BYTES:-134217728}" \
        -v "$VT_CONF:/etc/vector/vector.yaml:ro" \
        "$IMAGE" --config /etc/vector/vector.yaml >/dev/null
    ;;
  wcm|wcm-v2|wcm-tw128|wcm-flush10ms|wcm-b*)
    # 同 test/ours/latency-benchmark-Rust/latency.sh 的掛載方式:
    # 設定檔 + plugin 目錄唯讀掛載,plugin 路徑以「設定檔目錄/plugin/」解析
    $DOCKER run -d --name "$CONTAINER" --network host \
        --cpus "$CPU_LIMIT" --memory "$MEM_LIMIT" --memory-swap "$MEM_LIMIT" \
        -e MEM_GUARD_HIGH="${MEM_GUARD_HIGH:-0.70}" \
        -e MEM_GUARD_LOW="${MEM_GUARD_LOW:-0.55}" \
        -v "$WCM_YAML:/app/forwarder.yaml:ro" \
        -v "$WCM_LAT/plugin:/app/plugin:ro" \
        "$IMAGE" >/dev/null
    ;;
esac
$DOCKER logs -f "$CONTAINER" > "$OUT_DIR/dut.log" 2>&1 &
LOGS_PID=$!

# 就緒偵測:等 TCP :5140 可連(所有 DUT 一致)
for i in $(seq 120); do
    if timeout 1 bash -c "echo -n > /dev/tcp/127.0.0.1/$TCP_PORT" 2>/dev/null; then break; fi
    if (( i == 120 )); then echo "[cap] ERROR: DUT 未在 120s 內就緒" >&2; exit 1; fi
    sleep 1
done
echo "[cap] DUT 就緒(TCP :$TCP_PORT 可連),3 秒後開始打流量"
sleep 3

# ─── 3. gen ──────────────────────────────────────────────────────────────────
GEN_BIN="$SCRIPT_DIR/gen_bin"
[[ -x "$GEN_BIN" ]] || ( cd "$REPO_ROOT/tools/gen" && go build -o "$GEN_BIN" main.go )
echo "[cap] gen 開始:$RATE eps × ${LOAD_SECS}s mode=$GEN_MODE(詳細進度:tail -f $OUT_DIR/gen.log / $OUT_DIR/sink.log)"
# 每 30 秒把 sink 最新的每秒統計吐到終端,證明實驗活著
( while sleep 30; do tail -1 "$OUT_DIR/sink.log" 2>/dev/null | sed 's/^/[cap] 進度 /'; done ) &
PROG_PID=$!
GEN_START=$(date +%s.%N)
# latency mode 從 generator 啟動時開始每秒採樣 DUT 主程序，使 stats.csv
# 的 elapsed_s 可與延遲窗口使用相同的秒數範圍。
if [[ "$MODE" == "lat" ]]; then
    DUT_PID=$($DOCKER inspect -f '{{.State.Pid}}' "$CONTAINER")
    # 容器 cgroup scope 包含 init/supervisor/worker 全部程序，並避免
    # Fluentd 多個 Ruby 程序的 shared page 被 RSS 重複計數。
    "$COMPARE_DIR/sample_cgroup_stats.sh" "$DUT_PID" "$OUT_DIR/stats.csv" "$MEM_LIMIT" "$CPU_LIMIT" &
    STATS_PID=$!
fi
# timeout 保險:DUT 若永久 backpressure(如 mem-guard 暫停不恢復),gen 會被卡死;
# 超過 LOAD_SECS+90s 強制終止,讓本試驗以 FAIL 收場而不是吊死整個搜尋
timeout -k 5 $(( LOAD_SECS + 90 )) taskset -c "$GEN_CORES" "$GEN_BIN" \
    -rate "$RATE" -duration "$LOAD_SECS" -traffic flat -mode "$GEN_MODE" \
    -ts-mode sched -output tcp -tcp-addr "127.0.0.1:$TCP_PORT" \
    -send-unit batch -batch-lines 500 -flush-ms 20 -conns 1 \
    -log-file "$OUT_DIR/gen.log"
GEN_END=$(date +%s.%N)
echo "$GEN_START $GEN_END" > "$OUT_DIR/gen_epoch.txt"

# ─── 4. drain + 收尾 ─────────────────────────────────────────────────────────
kill "$PROG_PID" 2>/dev/null || true; PROG_PID=""
echo "[cap] gen 完成,drain ${DRAIN_SECS}s 後關閉並判定"
sleep "$DRAIN_SECS"
cleanup

# ─── 5. 判定 ─────────────────────────────────────────────────────────────────
python3 "$SCRIPT_DIR/eval_trial.py" "$OUT_DIR" "$RATE" | tee "$OUT_DIR/verdict.txt"
grep -q "^PASS" "$OUT_DIR/verdict.txt"
