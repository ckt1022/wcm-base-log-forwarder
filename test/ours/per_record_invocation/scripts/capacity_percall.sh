#!/bin/bash
# capacity_percall.sh — test/compare/capacity/capacity.sh 的複本，只保留 WCM，
# 並新增 per-record invocation 研究用的 DUT 變體。其餘流程、參數與判定逐字沿用：
#   sink → DUT（docker, --cpus 1 --memory 512m, network host）→ gen 定速 → drain → eval_trial.py
#
# 用法: capacity_percall.sh <dut> <rate> [cap|lat]
#   dut:
#     wcm-percall-k1     image wcm-base-log-forwarder:percall + forwarder_membound_b1_k1.yaml
#     wcm-percall-k16    ...                                   + forwarder_membound_b1_k16.yaml
#     wcm-percall-k256   ...                                   + forwarder_membound_b1_k256.yaml
#     wcm-percall-kinf   ...  （對照組：不設 plugin_call_size，= 原 wcm-b1 設定逐字相同）
#   YAML 皆由 test/ours/latency-benchmark-Rust/forwarder_membound_b1.yaml（正式 wcm-b1）
#   以 make_yaml.py 產生，唯一差異是 plugin_call_size。
#
# 與原 capacity.sh 的差異（僅限路徑/命名，不影響量測條件）：
#   - 結果寫到本研究目錄 system/runs/<dut>/...，不寫入 test/compare/capacity/runs
#   - container 名稱前綴 percall-
#   - 不再 fallback 編譯 gen/sink（直接使用 test/compare/capacity 既有 binary，唯讀）
#   - 移除 fluentd / fluent-bit / vector 分支（避免 sed 就地修改他們的 conf）
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STUDY_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
MAIN_REPO="/home/ckt1022/wcm-base-log-forwarder"
CAP_DIR="$MAIN_REPO/test/compare/capacity"     # 正式 harness（唯讀使用）
COMPARE_DIR="$MAIN_REPO/test/compare"

DUT="${1:?usage: capacity_percall.sh <dut> <rate> [cap|lat]}"
RATE="${2:?usage: capacity_percall.sh <dut> <rate> [cap|lat]}"
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
    PREALLOC=$(( RATE * 4 / 10 * LOAD_SECS * 11 / 10 ))
else
    LOAD_SECS="${LOAD_SECS:-60}"
    PARSE_TS="false"
    PREALLOC=0
fi
DRAIN_SECS="${DRAIN_SECS:-20}"

TCP_PORT=5140
SINK_PORT=8080
CONTAINER="percall-cap-$DUT"

WCM_LAT="$MAIN_REPO/test/ours/latency-benchmark-Rust"   # plugin 目錄（唯讀掛載）
YAML_DIR="$STUDY_DIR/system/yaml"
IMAGE="wcm-base-log-forwarder:percall"
case "$DUT" in
  wcm-percall-k1)   WCM_YAML="$YAML_DIR/forwarder_membound_b1_k1.yaml" ;;
  wcm-percall-k16)  WCM_YAML="$YAML_DIR/forwarder_membound_b1_k16.yaml" ;;
  wcm-percall-k256) WCM_YAML="$YAML_DIR/forwarder_membound_b1_k256.yaml" ;;
  wcm-percall-kinf) WCM_YAML="$YAML_DIR/forwarder_membound_b1_kinf.yaml" ;;
  *) echo "unknown dut: $DUT" >&2; exit 2 ;;
esac
[[ -f "$WCM_YAML" ]] || { echo "missing $WCM_YAML" >&2; exit 2; }
if [[ "$FLUSH_INTERVAL" != "1" ]]; then
    echo "[cap] ERROR: wcm 的 flush 窗固定 1s(forwarder.yaml max_wait_ms=1000),FLUSH_INTERVAL=$FLUSH_INTERVAL 不支援" >&2
    exit 2
fi

RUN_SUFFIX=""
[[ -n "$RUN_VARIANT" ]] && RUN_SUFFIX="_${RUN_VARIANT}"
OUT_DIR="$STUDY_DIR/system/runs/$DUT/${MODE}_rate_${RATE}${RUN_SUFFIX}"
if [[ -e "$OUT_DIR/verdict.txt" ]]; then
    echo "[cap] ERROR: $OUT_DIR 已有結果，拒絕覆寫（請換 RUN_VARIANT）" >&2
    exit 2
fi
mkdir -p "$OUT_DIR"

# ─── 時鐘檢查(同 capacity.sh)──────────────────────────────────────────────────
cs=$(cat /sys/devices/system/clocksource/clocksource0/current_clocksource 2>/dev/null || echo unknown)
tsd=$(systemctl is-active systemd-timesyncd 2>/dev/null || true)
if [[ "$cs" != "hyperv_clocksource_tsc_page" || "$tsd" == "active" ]]; then
    echo "[cap] WARNING: 時鐘狀態異常 clocksource=$cs timesyncd=$tsd" >&2
    [[ "${SKIP_CLOCK_CHECK:-0}" == "1" ]] || exit 1
fi
echo "clocksource=$cs timesyncd=$tsd" > "$OUT_DIR/clock.txt"

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
SINK_BIN="$CAP_DIR/httpsink_bin"
SINK_EPOCH=$(date +%s.%N)
taskset -c "$SINK_CORE" "$SINK_BIN" -port "$SINK_PORT" -out-dir "$OUT_DIR" -parse-ts="$PARSE_TS" \
    -prealloc "$PREALLOC" \
    > "$OUT_DIR/sink.log" 2>&1 &
SINK_PID=$!
echo "$SINK_EPOCH" > "$OUT_DIR/sink_epoch.txt"
echo "[cap] sink 已啟動(:$SINK_PORT,log=$OUT_DIR/sink.log)"

# ─── 2. DUT ──────────────────────────────────────────────────────────────────
echo "[cap] dut=$DUT rate=$RATE mode=$MODE load=${LOAD_SECS}s flush=${FLUSH_INTERVAL}s cpus=$CPU_LIMIT mem=$MEM_LIMIT"
cp "$WCM_YAML" "$OUT_DIR/forwarder.yaml"
$DOCKER run -d --name "$CONTAINER" --network host \
    --cpus "$CPU_LIMIT" --memory "$MEM_LIMIT" --memory-swap "$MEM_LIMIT" \
    -e MEM_GUARD_HIGH="${MEM_GUARD_HIGH:-0.70}" \
    -e MEM_GUARD_LOW="${MEM_GUARD_LOW:-0.55}" \
    -v "$WCM_YAML:/app/forwarder.yaml:ro" \
    -v "$WCM_LAT/plugin:/app/plugin:ro" \
    "$IMAGE" >/dev/null
$DOCKER inspect -f '{{json .Config.Image}} {{.Image}}' "$CONTAINER" > "$OUT_DIR/image.txt"
$DOCKER logs -f "$CONTAINER" > "$OUT_DIR/dut.log" 2>&1 &
LOGS_PID=$!

for i in $(seq 120); do
    if timeout 1 bash -c "echo -n > /dev/tcp/127.0.0.1/$TCP_PORT" 2>/dev/null; then break; fi
    if (( i == 120 )); then echo "[cap] ERROR: DUT 未在 120s 內就緒" >&2; exit 1; fi
    sleep 1
done
echo "[cap] DUT 就緒(TCP :$TCP_PORT 可連),3 秒後開始打流量"
sleep 3

# ─── 3. gen ──────────────────────────────────────────────────────────────────
GEN_BIN="$CAP_DIR/gen_bin"
echo "[cap] gen 開始:$RATE eps × ${LOAD_SECS}s mode=$GEN_MODE"
( while sleep 30; do tail -1 "$OUT_DIR/sink.log" 2>/dev/null | sed 's/^/[cap] 進度 /'; done ) &
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
    -log-file "$OUT_DIR/gen.log"
GEN_END=$(date +%s.%N)
echo "$GEN_START $GEN_END" > "$OUT_DIR/gen_epoch.txt"

# ─── 4. drain + 收尾 ─────────────────────────────────────────────────────────
kill "$PROG_PID" 2>/dev/null || true; PROG_PID=""
echo "[cap] gen 完成,drain ${DRAIN_SECS}s 後關閉並判定"
sleep "$DRAIN_SECS"
cleanup

# ─── 5. 判定(正式 eval_trial.py,唯讀)──────────────────────────────────────
python3 "$CAP_DIR/eval_trial.py" "$OUT_DIR" "$RATE" | tee "$OUT_DIR/verdict.txt"
grep -q "^PASS" "$OUT_DIR/verdict.txt"
