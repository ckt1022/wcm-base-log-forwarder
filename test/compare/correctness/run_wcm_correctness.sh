#!/bin/bash
# Execute deterministic WCM correctness scenarios C1/C2/C3.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
WCM_DIR="$REPO_ROOT/test/ours/latency-benchmark-Rust"
CONFIG="$WCM_DIR/forwarder_membound_b1.yaml"
IMAGE="wcm-base-log-forwarder:memguard"
DOCKER="${DOCKER:-docker}"
REPEATS="${REPEATS:-3}"
SCENARIOS="${SCENARIOS:-C1 C2 C3}"
RUN_ID="${RUN_ID:-$(date +%Y%m%d_%H%M%S)}"
RUN_ROOT="$SCRIPT_DIR/runs/$RUN_ID"
GEN_BIN="$SCRIPT_DIR/correctgen_bin"
SINK_BIN="$SCRIPT_DIR/validationsink_bin"
DRAIN_SECS="${DRAIN_SECS:-20}"

active_container=""
active_sink_pid=""
active_stats_pid=""
active_logs_pid=""

cleanup_active() {
    [[ -n "$active_stats_pid" ]] && kill "$active_stats_pid" 2>/dev/null || true
    [[ -n "$active_sink_pid" ]] && kill -TERM "$active_sink_pid" 2>/dev/null || true
    [[ -n "$active_container" ]] && $DOCKER stop -t 10 "$active_container" >/dev/null 2>&1 || true
    [[ -n "$active_container" ]] && $DOCKER rm "$active_container" >/dev/null 2>&1 || true
    [[ -n "$active_logs_pid" ]] && kill "$active_logs_pid" 2>/dev/null || true
}
trap cleanup_active EXIT INT TERM

mkdir -p "$RUN_ROOT"
(cd "$SCRIPT_DIR/generator" && go build -o "$GEN_BIN" main.go)
(cd "$SCRIPT_DIR/validationsink" && go build -o "$SINK_BIN" main.go)

cleanup_run() {
    local container="$1" sink_pid="${2:-}" stats_pid="${3:-}"
    [[ -n "$stats_pid" ]] && kill "$stats_pid" 2>/dev/null || true
    [[ -n "$stats_pid" ]] && wait "$stats_pid" 2>/dev/null || true
    $DOCKER stop -t 10 "$container" >/dev/null 2>&1 || true
    [[ -n "$sink_pid" ]] && kill -TERM "$sink_pid" 2>/dev/null || true
    [[ -n "$sink_pid" ]] && wait "$sink_pid" 2>/dev/null || true
}

for scenario in $SCENARIOS; do
    case "$scenario" in
        C1) rate="${C1_RATE_OVERRIDE:-2500}"; count="${C1_COUNT_OVERRIDE:-100000}"; malformed=false ;;
        C2) rate="${C2_RATE_OVERRIDE:-17500}"; count="${C2_COUNT_OVERRIDE:-1000000}"; malformed=false ;;
        C3) rate="${C3_RATE_OVERRIDE:-2500}"; count="${C3_COUNT_OVERRIDE:-100000}"; malformed=true ;;
        C4) rate="${C4_RATE_OVERRIDE:-2500}"; count="${C4_COUNT_OVERRIDE:-100000}"; malformed=false ;;
        *) echo "unknown scenario: $scenario" >&2; exit 2 ;;
    esac
    for rep in $(seq 1 "$REPEATS"); do
        out="$RUN_ROOT/${scenario}_rep${rep}"
        container="wcm-correct-${RUN_ID}-${scenario,,}-${rep}"
        mkdir -p "$out"
        printf '%s\n' "$count" > "$out/input_count.txt"
        printf '%s\n' "$rate" > "$out/rate.txt"
        printf '%s\n' "$malformed" > "$out/malformed.txt"
        echo "[correctness] ===== $scenario rep=$rep rate=$rate count=$count malformed=$malformed ====="

        sink_args=(-port 8080 -out-dir "$out" -input-count "$count")
        gen_args=(-rate "$rate" -count "$count" -tcp-addr 127.0.0.1:5140 -summary "$out/generator_summary.json")
        if [[ "$malformed" == true ]]; then
            sink_args+=(-malformed)
            gen_args+=(-malformed)
        fi
        if [[ "$scenario" == C4 ]]; then
            sink_args+=(-edge-types)
            gen_args+=(-edge-types)
        fi
        taskset -c 8 "$SINK_BIN" "${sink_args[@]}" > "$out/sink.log" 2>&1 &
        sink_pid=$!
        active_sink_pid="$sink_pid"

        $DOCKER run -d --name "$container" --network host \
            --cpus 1 --memory 512m --memory-swap 512m \
            -e MEM_GUARD_HIGH=0.70 -e MEM_GUARD_LOW=0.55 \
            -v "$CONFIG:/app/forwarder.yaml:ro" \
            -v "$WCM_DIR/plugin:/app/plugin:ro" \
            "$IMAGE" >/dev/null
        active_container="$container"
        $DOCKER inspect -f '{{json .Config.Image}} {{.Image}}' "$container" > "$out/image.txt"
        $DOCKER logs -f "$container" > "$out/dut.log" 2>&1 &
        logs_pid=$!
        active_logs_pid="$logs_pid"

        ready=false
        for _ in $(seq 1 120); do
            if timeout 1 bash -c 'echo -n > /dev/tcp/127.0.0.1/5140' 2>/dev/null; then ready=true; break; fi
            sleep 1
        done
        if [[ "$ready" != true ]]; then
            echo "[correctness] WCM readiness timeout" >&2
            cleanup_run "$container" "$sink_pid" ""
            exit 1
        fi
        sleep 3
        dut_pid=$($DOCKER inspect -f '{{.State.Pid}}' "$container")
        "$REPO_ROOT/test/compare/sample_cgroup_stats.sh" "$dut_pid" "$out/stats.csv" 512m 1 &
        stats_pid=$!
        active_stats_pid="$stats_pid"

        taskset -c 9,10 "$GEN_BIN" "${gen_args[@]}" | tee "$out/generator.log"
        echo "[correctness] generator done; drain ${DRAIN_SECS}s"
        sleep "$DRAIN_SECS"

        kill "$stats_pid" 2>/dev/null || true
        wait "$stats_pid" 2>/dev/null || true
        stats_pid=""
        active_stats_pid=""
        oom=$($DOCKER inspect -f '{{.State.OOMKilled}}' "$container")
        printf '%s\n' "$oom" > "$out/oom.txt"
        $DOCKER inspect -f '{{json .State}}' "$container" > "$out/pre_stop_state.json"
        $DOCKER stop -t 10 "$container" >/dev/null
        $DOCKER inspect -f '{{json .State}}' "$container" > "$out/container_state.json"
        $DOCKER rm "$container" >/dev/null
        active_container=""
        kill "$logs_pid" 2>/dev/null || true
        wait "$logs_pid" 2>/dev/null || true
        active_logs_pid=""
        kill -TERM "$sink_pid" 2>/dev/null || true
        wait "$sink_pid"
        sink_pid=""
        active_sink_pid=""

        python3 "$SCRIPT_DIR/evaluate.py" "$out" || true
    done
done

echo "[correctness] completed run root: $RUN_ROOT"
