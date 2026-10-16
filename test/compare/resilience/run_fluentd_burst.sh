#!/bin/bash
# Deterministic Fluentd burst/recovery experiment, aligned with WCM/Vector.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
CORRECT_DIR="$REPO_ROOT/test/compare/correctness"
FLUENTD_DIR="$REPO_ROOT/test/compare/fluentd/latency"
CONFIG="$FLUENTD_DIR/conf/capacity_1m_1s.conf"
IMAGE="${IMAGE:-fluent/fluentd:v1.16-debian-1}"
DOCKER="${DOCKER:-docker}"
REPEATS="${REPEATS:-1}"
SCENARIOS="${SCENARIOS:-B1 B2 B3}"
RUN_ID="${RUN_ID:-fluentd_burst_$(date +%Y%m%d_%H%M%S)}"
RUN_ROOT="$SCRIPT_DIR/runs/$RUN_ID"
GEN_BIN="$CORRECT_DIR/correctgen_bin"
SINK_BIN="$CORRECT_DIR/validationsink_bin"

WARM_SECS="${WARM_SECS:-180}"
BASELINE_SECS="${BASELINE_SECS:-120}"
RECOVERY_SECS="${RECOVERY_SECS:-600}"
DRAIN_SECS="${DRAIN_SECS:-120}"
BASELINE_RATE="${BASELINE_RATE:-25000}"
BURST_RATE="${BURST_RATE:-75000}"
ANALYZER_SCRIPT="${ANALYZER_SCRIPT:-$SCRIPT_DIR/analyze_burst_forward30.py}"

mkdir -p "$RUN_ROOT"
(cd "$CORRECT_DIR/generator" && go build -o "$GEN_BIN" main.go)
(cd "$CORRECT_DIR/validationsink" && go build -o "$SINK_BIN" main.go)

active_container=""; active_sink_pid=""; active_stats_pid=""; active_logs_pid=""
cleanup() {
    [[ -n "$active_stats_pid" ]] && kill "$active_stats_pid" 2>/dev/null || true
    [[ -n "$active_sink_pid" ]] && kill -TERM "$active_sink_pid" 2>/dev/null || true
    [[ -n "$active_container" ]] && $DOCKER stop -t 10 "$active_container" >/dev/null 2>&1 || true
    [[ -n "$active_container" ]] && $DOCKER rm "$active_container" >/dev/null 2>&1 || true
    [[ -n "$active_logs_pid" ]] && kill "$active_logs_pid" 2>/dev/null || true
}
trap cleanup EXIT INT TERM

for scenario in $SCENARIOS; do
    case "$scenario" in
        B1) burst_secs="${B1_BURST_SECS:-5}" ;;
        B2) burst_secs="${B2_BURST_SECS:-15}" ;;
        B3) burst_secs="${B3_BURST_SECS:-30}" ;;
        *) echo "unknown scenario: $scenario" >&2; exit 2 ;;
    esac
    total_count=$(( BASELINE_RATE * (WARM_SECS + BASELINE_SECS + RECOVERY_SECS) + BURST_RATE * burst_secs ))
    planned_secs=$(( WARM_SECS + BASELINE_SECS + burst_secs + RECOVERY_SECS ))
    phase_spec="warmup:${BASELINE_RATE}:${WARM_SECS},baseline:${BASELINE_RATE}:${BASELINE_SECS},burst:${BURST_RATE}:${burst_secs},recovery:${BASELINE_RATE}:${RECOVERY_SECS}"

    for rep in $(seq 1 "$REPEATS"); do
        out="$RUN_ROOT/${scenario}_rep${rep}"
        if [[ "${SKIP_COMPLETED:-1}" == "1" && -s "$out/burst_analysis.json" ]]; then echo "[burst] skip completed $scenario rep=$rep"; continue; fi
        container="fluentd-burst-${RUN_ID}-${scenario,,}-${rep}"
        mkdir -p "$out"
        printf '%s\n' "$total_count" > "$out/input_count.txt"
        printf '%s\n' "$BASELINE_RATE" > "$out/rate.txt"
        printf 'false\n' > "$out/malformed.txt"
        printf '%s\n' "$phase_spec" > "$out/phase_spec.txt"
        echo "[burst] ===== $scenario rep=$rep burst=${BURST_RATE}x${burst_secs}s total=$total_count ====="

        taskset -c 8 "$SINK_BIN" -port 8080 -out-dir "$out" -input-count "$total_count" \
            -expected-lang Ruby > "$out/sink.log" 2>&1 &
        sink_pid=$!; active_sink_pid="$sink_pid"

        $DOCKER run -d --name "$container" --network host --cpus 1 --memory 512m --memory-swap 512m \
            -e FLUSH_INTERVAL=1 -v "$CONFIG:/fluentd/etc/fluent.conf:ro" \
            -v "$FLUENTD_DIR/plugins:/fluentd/plugins:ro" "$IMAGE" >/dev/null
        active_container="$container"
        $DOCKER inspect -f '{{json .Config.Image}} {{.Image}}' "$container" > "$out/image.txt"
        $DOCKER logs -f "$container" > "$out/dut.log" 2>&1 &
        logs_pid=$!; active_logs_pid="$logs_pid"

        ready=false
        for _ in $(seq 1 120); do if timeout 1 bash -c 'echo -n > /dev/tcp/127.0.0.1/5140' 2>/dev/null; then ready=true; break; fi; sleep 1; done
        [[ "$ready" == true ]] || { echo "[burst] Fluentd readiness timeout" >&2; exit 1; }
        sleep 3

        dut_pid=$($DOCKER inspect -f '{{.State.Pid}}' "$container")
        date +%s.%N > "$out/stats_epoch.txt"
        "$REPO_ROOT/test/compare/sample_cgroup_stats.sh" "$dut_pid" "$out/stats.csv" 512m 1 &
        stats_pid=$!; active_stats_pid="$stats_pid"

        timeout -k 10 $(( planned_secs + 900 )) taskset -c 9,10 "$GEN_BIN" \
            -phases "$phase_spec" -tcp-addr 127.0.0.1:5140 -phase-csv "$out/phases.csv" \
            -per-sec-csv "$out/generator_per_sec.csv" -summary "$out/generator_summary.json" | tee "$out/generator.log"
        echo "[burst] generator completed; final drain ${DRAIN_SECS}s"; sleep "$DRAIN_SECS"

        kill "$stats_pid" 2>/dev/null || true; wait "$stats_pid" 2>/dev/null || true; stats_pid=""; active_stats_pid=""
        $DOCKER inspect -f '{{.State.OOMKilled}}' "$container" > "$out/oom.txt"
        $DOCKER inspect -f '{{json .State}}' "$container" > "$out/pre_stop_state.json"
        $DOCKER stop -t 10 "$container" >/dev/null
        $DOCKER inspect -f '{{json .State}}' "$container" > "$out/container_state.json"
        $DOCKER rm "$container" >/dev/null; active_container=""
        kill "$logs_pid" 2>/dev/null || true; wait "$logs_pid" 2>/dev/null || true; active_logs_pid=""
        kill -TERM "$sink_pid" 2>/dev/null || true; wait "$sink_pid"; active_sink_pid=""

        python3 "$CORRECT_DIR/evaluate.py" "$out" | tee "$out/delivery_verdict.txt" || true
        python3 "$ANALYZER_SCRIPT" "$out" | tee "$out/recovery_verdict.txt" || true
    done
done

python3 "$SCRIPT_DIR/summarize_burst.py" "$RUN_ROOT" Fluentd
echo "[burst] completed run root: $RUN_ROOT"
