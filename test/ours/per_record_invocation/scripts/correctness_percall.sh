#!/bin/bash
# correctness_percall.sh — test/compare/correctness/run_wcm_correctness.sh 的複本。
# 情境參數、generator/sink、評估（evaluate.py）逐字沿用；差異：
#   - image = wcm-base-log-forwarder:percall，CONFIG 由第一個參數指定（percall YAML）
#   - 結果寫到本研究目錄 system/correctness/<RUN_ID>/，container 名稱前綴 percall-
#   - 不重新 go build generator/sink（直接使用既有 binary，唯讀）
#   - 每個 rep 單獨持有共用量測鎖 /home/ckt1022/.wcm-bench.lock
#
# 用法: correctness_percall.sh <config.yaml> <run_id>
#   環境變數 SCENARIOS（預設 C3）、REPEATS（預設 3）
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STUDY_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
MAIN_REPO="/home/ckt1022/wcm-base-log-forwarder"
CORR_DIR="$MAIN_REPO/test/compare/correctness"
WCM_DIR="$MAIN_REPO/test/ours/latency-benchmark-Rust"
CONFIG="${1:?usage: correctness_percall.sh <config.yaml> <run_id>}"
RUN_ID="${2:?usage: correctness_percall.sh <config.yaml> <run_id>}"
IMAGE="wcm-base-log-forwarder:percall"
DOCKER="${DOCKER:-docker}"
REPEATS="${REPEATS:-3}"
SCENARIOS="${SCENARIOS:-C3}"
RUN_ROOT="$STUDY_DIR/system/correctness/$RUN_ID"
GEN_BIN="$CORR_DIR/correctgen_bin"
SINK_BIN="$CORR_DIR/validationsink_bin"
DRAIN_SECS="${DRAIN_SECS:-20}"
LOCK=/home/ckt1022/.wcm-bench.lock

mkdir -p "$RUN_ROOT"
cp "$CONFIG" "$RUN_ROOT/forwarder.yaml"

one_rep() {
    local scenario="$1" rep="$2" rate="$3" count="$4" malformed="$5"
    local out="$RUN_ROOT/${scenario}_rep${rep}"
    local container="percall-correct-${RUN_ID}-${scenario,,}-${rep}"
    [[ -e "$out/verdict.txt" ]] && { echo "exists: $out" >&2; return 2; }
    mkdir -p "$out"
    printf '%s\n' "$count" > "$out/input_count.txt"
    printf '%s\n' "$rate" > "$out/rate.txt"
    printf '%s\n' "$malformed" > "$out/malformed.txt"
    echo "[correctness] ===== $scenario rep=$rep rate=$rate count=$count malformed=$malformed config=$CONFIG ====="

    local sink_args=(-port 8080 -out-dir "$out" -input-count "$count")
    local gen_args=(-rate "$rate" -count "$count" -tcp-addr 127.0.0.1:5140 -summary "$out/generator_summary.json")
    if [[ "$malformed" == true ]]; then
        sink_args+=(-malformed)
        gen_args+=(-malformed)
    fi
    if [[ "$scenario" == C4 ]]; then
        sink_args+=(-edge-types)
        gen_args+=(-edge-types)
    fi
    taskset -c 8 "$SINK_BIN" "${sink_args[@]}" > "$out/sink.log" 2>&1 &
    local sink_pid=$!

    $DOCKER rm -f "$container" >/dev/null 2>&1 || true
    $DOCKER run -d --name "$container" --network host \
        --cpus 1 --memory 512m --memory-swap 512m \
        -e MEM_GUARD_HIGH=0.70 -e MEM_GUARD_LOW=0.55 \
        -v "$CONFIG:/app/forwarder.yaml:ro" \
        -v "$WCM_DIR/plugin:/app/plugin:ro" \
        "$IMAGE" >/dev/null
    $DOCKER inspect -f '{{json .Config.Image}} {{.Image}}' "$container" > "$out/image.txt"
    $DOCKER logs -f "$container" > "$out/dut.log" 2>&1 &
    local logs_pid=$!

    local ready=false
    for _ in $(seq 1 120); do
        if timeout 1 bash -c 'echo -n > /dev/tcp/127.0.0.1/5140' 2>/dev/null; then ready=true; break; fi
        sleep 1
    done
    if [[ "$ready" != true ]]; then
        echo "[correctness] WCM readiness timeout" >&2
        $DOCKER stop -t 10 "$container" >/dev/null 2>&1 || true
        $DOCKER rm "$container" >/dev/null 2>&1 || true
        kill -TERM "$sink_pid" 2>/dev/null || true
        return 1
    fi
    sleep 3
    local dut_pid
    dut_pid=$($DOCKER inspect -f '{{.State.Pid}}' "$container")
    "$MAIN_REPO/test/compare/sample_cgroup_stats.sh" "$dut_pid" "$out/stats.csv" 512m 1 &
    local stats_pid=$!

    taskset -c 9,10 "$GEN_BIN" "${gen_args[@]}" | tee "$out/generator.log"
    echo "[correctness] generator done; drain ${DRAIN_SECS}s"
    sleep "$DRAIN_SECS"

    kill "$stats_pid" 2>/dev/null || true
    wait "$stats_pid" 2>/dev/null || true
    $DOCKER inspect -f '{{.State.OOMKilled}}' "$container" > "$out/oom.txt"
    $DOCKER inspect -f '{{json .State}}' "$container" > "$out/pre_stop_state.json"
    $DOCKER stop -t 10 "$container" >/dev/null
    $DOCKER inspect -f '{{json .State}}' "$container" > "$out/container_state.json"
    $DOCKER rm "$container" >/dev/null
    kill "$logs_pid" 2>/dev/null || true
    wait "$logs_pid" 2>/dev/null || true
    kill -TERM "$sink_pid" 2>/dev/null || true
    wait "$sink_pid" || true

    python3 "$CORR_DIR/evaluate.py" "$out" || true
}

# 由 flock 子行程呼叫單一 rep
if [[ "${PERCALL_ONE_REP:-}" == "1" ]]; then
    one_rep "$3" "$4" "$5" "$6" "$7"
    exit $?
fi

for scenario in $SCENARIOS; do
    case "$scenario" in
        C1) rate=2500; count=100000; malformed=false ;;
        C2) rate=17500; count=1000000; malformed=false ;;
        C3) rate=2500; count=100000; malformed=true ;;
        C4) rate=2500; count=100000; malformed=false ;;
        *) echo "unknown scenario: $scenario" >&2; exit 2 ;;
    esac
    for rep in $(seq 1 "$REPEATS"); do
        PERCALL_ONE_REP=1 flock "$LOCK" "$0" "$CONFIG" "$RUN_ID" "$scenario" "$rep" "$rate" "$count" "$malformed" || true
    done
done

echo "[correctness] completed run root: $RUN_ROOT"
