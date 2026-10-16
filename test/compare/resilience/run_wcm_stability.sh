#!/bin/bash
# One-hour constant-load stability pilot for WCM.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
CORRECT_DIR="$REPO_ROOT/test/compare/correctness"
WCM_DIR="$REPO_ROOT/test/ours/latency-benchmark-Rust"
CONFIG="$WCM_DIR/forwarder_membound_b1.yaml"
IMAGE="${IMAGE:-wcm-base-log-forwarder:memguard}"
DOCKER="${DOCKER:-docker}"
RUN_ID="${RUN_ID:-wcm_stability_$(date +%Y%m%d_%H%M%S)}"
RUN_ROOT="$SCRIPT_DIR/runs/$RUN_ID"
GEN_BIN="$CORRECT_DIR/correctgen_bin"
SINK_BIN="$CORRECT_DIR/validationsink_bin"
WARM_SECS="${WARM_SECS:-180}"
STEADY_SECS="${STEADY_SECS:-3600}"
DRAIN_SECS="${DRAIN_SECS:-120}"
RATE="${RATE:-17800}"
LIVE_MONITOR="${LIVE_MONITOR:-1}"
MONITOR_INTERVAL="${MONITOR_INTERVAL:-5}"
INPUT_WIRE_BYTES="${INPUT_WIRE_BYTES:-1025}"

mkdir -p "$RUN_ROOT"
(cd "$CORRECT_DIR/generator" && go build -o "$GEN_BIN" main.go)
(cd "$CORRECT_DIR/validationsink" && go build -o "$SINK_BIN" main.go)

out="$RUN_ROOT/S1_rep1"
container="wcm-stability-${RUN_ID}"
total_count=$(( RATE * (WARM_SECS + STEADY_SECS) ))
phase_spec="warmup:${RATE}:${WARM_SECS},steady:${RATE}:${STEADY_SECS}"
mkdir -p "$out"
printf '%s\n' "$total_count" > "$out/input_count.txt"
printf '%s\n' "$RATE" > "$out/rate.txt"
printf 'false\n' > "$out/malformed.txt"
printf '%s\n' "$phase_spec" > "$out/phase_spec.txt"

sink_pid=""; stats_pid=""; logs_pid=""; monitor_pid=""; active_container=""
cleanup() {
    [[ -n "$monitor_pid" ]] && kill "$monitor_pid" 2>/dev/null || true
    [[ -n "$stats_pid" ]] && kill "$stats_pid" 2>/dev/null || true
    [[ -n "$sink_pid" ]] && kill -TERM "$sink_pid" 2>/dev/null || true
    [[ -n "$active_container" ]] && $DOCKER stop -t 10 "$active_container" >/dev/null 2>&1 || true
    [[ -n "$active_container" ]] && $DOCKER rm "$active_container" >/dev/null 2>&1 || true
    [[ -n "$logs_pid" ]] && kill "$logs_pid" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

echo "[stability] rate=$RATE warm=${WARM_SECS}s steady=${STEADY_SECS}s total=$total_count"
taskset -c 8 "$SINK_BIN" -port 8080 -out-dir "$out" -input-count "$total_count" \
    -require-unique-id=false > "$out/sink.log" 2>&1 &
sink_pid=$!

$DOCKER run -d --name "$container" --network host --cpus 1 --memory 512m --memory-swap 512m \
    -e MEM_GUARD_HIGH=0.70 -e MEM_GUARD_LOW=0.55 \
    -v "$CONFIG:/app/forwarder.yaml:ro" -v "$WCM_DIR/plugin:/app/plugin:ro" "$IMAGE" >/dev/null
active_container="$container"
$DOCKER inspect -f '{{json .Config.Image}} {{.Image}}' "$container" > "$out/image.txt"
$DOCKER logs -f "$container" > "$out/dut.log" 2>&1 &
logs_pid=$!

ready=false
for _ in $(seq 1 120); do
    if timeout 1 bash -c 'echo -n > /dev/tcp/127.0.0.1/5140' 2>/dev/null; then ready=true; break; fi
    sleep 1
done
[[ "$ready" == true ]] || { echo "[stability] readiness timeout" >&2; exit 1; }
sleep 3

dut_pid=$($DOCKER inspect -f '{{.State.Pid}}' "$container")
date +%s.%N > "$out/stats_epoch.txt"
"$REPO_ROOT/test/compare/sample_cgroup_stats.sh" "$dut_pid" "$out/stats.csv" 512m 1 &
stats_pid=$!

if [[ "$LIVE_MONITOR" == "1" ]]; then
    echo "[stability] live monitor every ${MONITOR_INTERVAL}s (also saved to $out/live_monitor.log)"
    python3 -u "$SCRIPT_DIR/monitor_stability_live.py" \
        --run-dir "$out" --interval "$MONITOR_INTERVAL" \
        --wire-bytes "$INPUT_WIRE_BYTES" --log-file "$out/live_monitor.log" &
    monitor_pid=$!
fi

timeout -k 10 $(( WARM_SECS + STEADY_SECS + 600 )) taskset -c 9,10 "$GEN_BIN" \
    -phases "$phase_spec" -tcp-addr 127.0.0.1:5140 -phase-csv "$out/phases.csv" \
    -per-sec-csv "$out/generator_per_sec.csv" -summary "$out/generator_summary.json" | tee "$out/generator.log"
echo "[stability] generator completed; drain ${DRAIN_SECS}s"
sleep "$DRAIN_SECS"

[[ -n "$monitor_pid" ]] && kill "$monitor_pid" 2>/dev/null || true
[[ -n "$monitor_pid" ]] && wait "$monitor_pid" 2>/dev/null || true
monitor_pid=""
kill "$stats_pid" 2>/dev/null || true; wait "$stats_pid" 2>/dev/null || true; stats_pid=""
$DOCKER inspect -f '{{.State.OOMKilled}}' "$container" > "$out/oom.txt"
$DOCKER inspect -f '{{json .State}}' "$container" > "$out/pre_stop_state.json"
$DOCKER stop -t 10 "$container" >/dev/null
$DOCKER inspect -f '{{json .State}}' "$container" > "$out/container_state.json"
$DOCKER rm "$container" >/dev/null; active_container=""
kill "$logs_pid" 2>/dev/null || true; wait "$logs_pid" 2>/dev/null || true; logs_pid=""
kill -TERM "$sink_pid" 2>/dev/null || true; wait "$sink_pid"; sink_pid=""

python3 "$CORRECT_DIR/evaluate.py" "$out" | tee "$out/delivery_verdict.txt" || true
python3 "$SCRIPT_DIR/analyze_stability.py" "$out" | tee "$out/stability_verdict.txt"
echo "[stability] completed: $out"
