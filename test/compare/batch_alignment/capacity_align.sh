#!/bin/bash
# capacity_align.sh — batch-alignment 實驗用的單點試驗腳本。
#
# 這是 test/compare/capacity/capacity.sh 的複本，量測流程逐行保持一致
# （sink → DUT → gen 定速 → drain → eval_trial.py 判定；相同 1 CPU / 512 MiB /
# --memory-swap、相同 gen 參數 json-fields1k、batch 500 行 / 20 ms、單連線、
# -ts-mode sched、相同 sink、相同 cgroup 取樣、相同 PASS 判定）。
# 唯一差異：
#   1. 每個 DUT 的設定檔由 CONF 指定（batch_alignment/conf/ 下的複本，只改批次參數）；
#      不再 sed 就地修改任何設定檔。
#   2. 輸出寫到 batch_alignment/runs/<dut>/<mode>_rate_<rate>_<TAG>/。
#   3. container 名稱改為 align-<dut>。
#   4. WCM 不再擋 FLUSH_INTERVAL（WCM 的時間窗由 CONF 指向的 yaml max_wait_ms 決定）。
#   5. Fluent Bit 額外以唯讀方式掛載 batch_alignment/conf/。
#   6. 在 run 目錄寫一份設定快照 run_config.txt。
#   7. 時鐘：本機 WSL2 目前 clocksource=tsc，wall clock 每 ~32 s 向前跳 ~1.6 s。
#      全程以 clock_drift_logger.py 記錄 D = wall − monotonic（clock_drift.csv），
#      判定改用 eval_trial_align.py（C1–C4 與 eval_trial.py 相同，只把 wall 時刻換到
#      monotonic 時間軸）；原版 eval_trial.py 的結果另存 verdict_orig_eval.txt 供對照。
#
# 用法: CONF=<conf> TAG=<tag> ./capacity_align.sh <dut> <rate> [cap|lat]
#   dut: fluentd | fbit-lua | fbit-wasm | vector | wcm
# 其他環境變數同 capacity.sh（GEN_MODE、LOAD_SECS、DRAIN_SECS、FLUSH_INTERVAL、
# VECTOR_BATCH_MAX_BYTES、VECTOR_BATCH_MAX_EVENTS …）。
set -euo pipefail
ALIGN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNS_DIR="${RUNS_DIR:-$ALIGN_DIR/runs}"; export RUNS_DIR; mkdir -p "$RUNS_DIR" "$RUNS_DIR/../logs${RUNS_SUFFIX:-}"
REPO_ROOT="$(cd "$ALIGN_DIR/../../.." && pwd)"
COMPARE_DIR="$REPO_ROOT/test/compare"
CAP_DIR="$COMPARE_DIR/capacity"      # 只讀：gen_bin / httpsink_bin / eval_trial.py

DUT="${1:?usage: capacity_align.sh <dut> <rate> [cap|lat]}"
RATE="${2:?usage}"
MODE="${3:-cap}"
CONF="${CONF:?CONF must point to the DUT config}"
TAG="${TAG:?TAG must be set}"

DOCKER="${DOCKER:-docker}"
CPU_LIMIT="${CPU_LIMIT:-1}"
MEM_LIMIT="${MEM_LIMIT:-512m}"
FLUSH_INTERVAL="${FLUSH_INTERVAL:-1}"
SINK_CORE="${SINK_CORE:-8}"
GEN_CORES="${GEN_CORES:-9,10}"
GEN_MODE="${GEN_MODE:-json-fields1k}"

if [[ "$MODE" == "lat" ]]; then
    LOAD_SECS="${LOAD_SECS:-180}"
    PARSE_TS="true"
    PREALLOC=$(( RATE * 4 / 10 * LOAD_SECS * 11 / 10 ))
else
    LOAD_SECS="${LOAD_SECS:-60}"
    PARSE_TS="false"
    PREALLOC=0
fi
DRAIN_SECS="${DRAIN_SECS:-20}"

TCP_PORT=5140
SINK_PORT=8080
CONTAINER="align-$DUT"

FB_LAT="$COMPARE_DIR/fluentbit/latency"
FD_LAT="$COMPARE_DIR/fluentd/latency"
WCM_LAT="$REPO_ROOT/test/ours/latency-benchmark-Rust"

case "$DUT" in
  fluentd)       IMAGE="fluent/fluentd:v1.16-debian-1" ;;
  fbit-lua|fbit-wasm) IMAGE="fluent/fluent-bit:5.0.9" ;;
  vector)        IMAGE="timberio/vector:0.49.0-alpine" ;;
  wcm)           IMAGE="wcm-base-log-forwarder:memguard" ;;
  *) echo "unknown dut: $DUT" >&2; exit 2 ;;
esac

OUT_DIR="$RUNS_DIR/$DUT/${MODE}_rate_${RATE}_${TAG}"
mkdir -p "$OUT_DIR"
rm -f "$OUT_DIR"/*.log "$OUT_DIR"/*.csv "$OUT_DIR"/verdict.txt "$OUT_DIR"/*_stats_30_150.txt 2>/dev/null || true

cs=$(cat /sys/devices/system/clocksource/clocksource0/current_clocksource 2>/dev/null || echo unknown)
tsd=$(systemctl is-active systemd-timesyncd 2>/dev/null || true)
if [[ "$cs" != "hyperv_clocksource_tsc_page" || "$tsd" == "active" ]]; then
    echo "[align] WARNING: 時鐘狀態異常 clocksource=$cs timesyncd=$tsd" >&2
    [[ "${SKIP_CLOCK_CHECK:-0}" == "1" ]] || exit 1
fi

{
    echo "date=$(date -Is)"
    echo "dut=$DUT rate=$RATE mode=$MODE tag=$TAG image=$IMAGE"
    echo "image_id=$($DOCKER image inspect -f '{{.Id}}' "$IMAGE" 2>/dev/null || echo ?)"
    echo "conf=$CONF"
    echo "load_secs=$LOAD_SECS drain_secs=$DRAIN_SECS gen_mode=$GEN_MODE cpus=$CPU_LIMIT mem=$MEM_LIMIT"
    echo "FLUSH_INTERVAL=$FLUSH_INTERVAL VECTOR_BATCH_MAX_BYTES=${VECTOR_BATCH_MAX_BYTES:-} VECTOR_BATCH_MAX_EVENTS=${VECTOR_BATCH_MAX_EVENTS:-}"
    echo "clocksource=$cs timesyncd=$tsd"
    echo "----- conf -----"
    cat "$CONF"
} > "$OUT_DIR/run_config.txt"

python3 "$ALIGN_DIR/clock_drift_logger.py" "$OUT_DIR/clock_drift.csv" 0.02 &
DRIFT_PID=$!
SINK_PID=""; LOGS_PID=""; PROG_PID=""; STATS_PID=""; _DONE=0
cleanup() {
    (( _DONE )) && return; _DONE=1
    [[ -n "$PROG_PID" ]] && kill "$PROG_PID" 2>/dev/null || true
    if [[ -n "$STATS_PID" ]]; then
        kill "$STATS_PID" 2>/dev/null || true
        wait "$STATS_PID" 2>/dev/null || true
    fi
    $DOCKER stop -t 10 "$CONTAINER" >/dev/null 2>&1 || true
    OOM=$($DOCKER inspect -f '{{.State.OOMKilled}}' "$CONTAINER" 2>/dev/null || echo unknown)
    echo "$OOM" > "$OUT_DIR/oom.txt"
    $DOCKER rm "$CONTAINER" >/dev/null 2>&1 || true
    [[ -n "$LOGS_PID" ]] && kill -TERM "$LOGS_PID" 2>/dev/null || true
    if [[ -n "$SINK_PID" ]] && kill -0 "$SINK_PID" 2>/dev/null; then
        kill -TERM "$SINK_PID" 2>/dev/null || true
        for _ in $(seq 40); do kill -0 "$SINK_PID" 2>/dev/null || break; sleep 0.5; done
        kill -9 "$SINK_PID" 2>/dev/null || true
    fi
    kill "$DRIFT_PID" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

$DOCKER rm -f "$CONTAINER" >/dev/null 2>&1 || true

# ─── 1. sink ─────────────────────────────────────────────────────────────────
SINK_BIN="$CAP_DIR/httpsink_bin"
SINK_EPOCH=$(date +%s.%N)
taskset -c "$SINK_CORE" "$SINK_BIN" -port "$SINK_PORT" -out-dir "$OUT_DIR" -parse-ts="$PARSE_TS" \
    -prealloc "$PREALLOC" > "$OUT_DIR/sink.log" 2>&1 &
SINK_PID=$!
echo "$SINK_EPOCH" > "$OUT_DIR/sink_epoch.txt"

# ─── 2. DUT ──────────────────────────────────────────────────────────────────
echo "[align] dut=$DUT rate=$RATE mode=$MODE tag=$TAG load=${LOAD_SECS}s conf=$(basename "$CONF")"
case "$DUT" in
  fluentd)
    $DOCKER run -d --name "$CONTAINER" --network host \
        --cpus "$CPU_LIMIT" --memory "$MEM_LIMIT" --memory-swap "$MEM_LIMIT" \
        -e FLUSH_INTERVAL="$FLUSH_INTERVAL" \
        -v "$CONF:/fluentd/etc/fluent.conf:ro" \
        -v "$FD_LAT/plugins:/fluentd/plugins:ro" \
        "$IMAGE" >/dev/null
    ;;
  fbit-lua|fbit-wasm)
    $DOCKER run -d --name "$CONTAINER" --network host \
        --cpus "$CPU_LIMIT" --memory "$MEM_LIMIT" --memory-swap "$MEM_LIMIT" \
        -v "$FB_LAT:$FB_LAT:ro" -v "$ALIGN_DIR/conf:$ALIGN_DIR/conf:ro" -w "$FB_LAT/conf" \
        "$IMAGE" -c "$CONF" >/dev/null
    ;;
  vector)
    $DOCKER run -d --name "$CONTAINER" --network host \
        --cpus "$CPU_LIMIT" --memory "$MEM_LIMIT" --memory-swap "$MEM_LIMIT" \
        -e FLUSH_INTERVAL="$FLUSH_INTERVAL" \
        -e VECTOR_BATCH_MAX_BYTES="${VECTOR_BATCH_MAX_BYTES:-134217728}" \
        -e VECTOR_BATCH_MAX_EVENTS="${VECTOR_BATCH_MAX_EVENTS:-100000000}" \
        -v "$CONF:/etc/vector/vector.yaml:ro" \
        "$IMAGE" --config /etc/vector/vector.yaml >/dev/null
    ;;
  wcm)
    $DOCKER run -d --name "$CONTAINER" --network host \
        --cpus "$CPU_LIMIT" --memory "$MEM_LIMIT" --memory-swap "$MEM_LIMIT" \
        -e MEM_GUARD_HIGH="${MEM_GUARD_HIGH:-0.70}" \
        -e MEM_GUARD_LOW="${MEM_GUARD_LOW:-0.55}" \
        -v "$CONF:/app/forwarder.yaml:ro" \
        -v "$WCM_LAT/plugin:/app/plugin:ro" \
        "$IMAGE" >/dev/null
    ;;
esac
$DOCKER logs -f "$CONTAINER" > "$OUT_DIR/dut.log" 2>&1 &
LOGS_PID=$!

for i in $(seq 120); do
    if timeout 1 bash -c "echo -n > /dev/tcp/127.0.0.1/$TCP_PORT" 2>/dev/null; then break; fi
    if (( i == 120 )); then echo "[align] ERROR: DUT 未在 120s 內就緒" >&2; exit 1; fi
    sleep 1
done
sleep 3

# ─── 3. gen ──────────────────────────────────────────────────────────────────
GEN_BIN="$CAP_DIR/gen_bin"
( while sleep 30; do tail -1 "$OUT_DIR/sink.log" 2>/dev/null | sed 's/^/[align] 進度 /'; done ) &
PROG_PID=$!
GEN_START=$(date +%s.%N)
if [[ "$MODE" == "lat" ]]; then
    DUT_PID=$($DOCKER inspect -f '{{.State.Pid}}' "$CONTAINER")
    "$COMPARE_DIR/sample_cgroup_stats.sh" "$DUT_PID" "$OUT_DIR/stats.csv" "$MEM_LIMIT" "$CPU_LIMIT" &
    STATS_PID=$!
fi
timeout -k 5 $(( LOAD_SECS + 90 )) taskset -c "$GEN_CORES" "$GEN_BIN" \
    -rate "$RATE" -duration "$LOAD_SECS" -traffic flat -mode "$GEN_MODE" \
    -ts-mode sched -output tcp -tcp-addr "127.0.0.1:$TCP_PORT" \
    -send-unit batch -batch-lines 500 -flush-ms 20 -conns 1 \
    -log-file "$OUT_DIR/gen.log" || true
GEN_END=$(date +%s.%N)
echo "$GEN_START $GEN_END" > "$OUT_DIR/gen_epoch.txt"

# ─── 4. drain + 收尾 ─────────────────────────────────────────────────────────
kill "$PROG_PID" 2>/dev/null || true; PROG_PID=""
sleep "$DRAIN_SECS"
cleanup

# ─── 5. 判定 ─────────────────────────────────────────────────────────────────
python3 "$CAP_DIR/eval_trial.py" "$OUT_DIR" "$RATE" > "$OUT_DIR/verdict_orig_eval.txt" || true
python3 "$ALIGN_DIR/eval_trial_align.py" "$OUT_DIR" "$RATE" | tee "$OUT_DIR/verdict.txt" || true
grep -q "^PASS" "$OUT_DIR/verdict.txt"
