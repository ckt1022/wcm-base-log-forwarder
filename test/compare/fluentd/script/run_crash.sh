#!/usr/bin/env bash
# Usage: ./run_crash.sh <test>
#   test: loop | io | cpu | mem   (default: loop)
#
# loop / cpu / mem : 流量監測模式
#   先跑 loggen 正常轉發 10 秒 → 注入 trigger → 觀察 sink 流量/資源變化
#   輸出: stats_<test>.csv（docker stats）+ stats_<test>_sink.csv（接收端每秒流量）
#   loop / cpu : 3 分鐘後自動停止容器
#   mem        : OOM kill 自然結束，3 分鐘為安全上限
#
# io : stdout 模式，觀察 filter 能存取哪些外部敏感檔案
#
# 每秒採樣一次 docker stats，結果存入 stats_<test>.csv

TEST="${1:-loop}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
FLUENTD_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
LOG_FILE="$(realpath "$FLUENTD_DIR/test-logs.log")"
TOOLS_DIR="$(realpath "$FLUENTD_DIR/../../tools")"

PLUGINS_DIR="$(realpath "$FLUENTD_DIR/plugins")"

IMAGE="fluent/fluentd:v1.16-debian-1"
CONTAINER_NAME="fd-${TEST}"
STATS_FILE="$FLUENTD_DIR/stats_${TEST}.csv"
TEST_DURATION=180
LOGGEN_RATE=10000

case "$TEST" in
  loop) TRIGGER='{"inject":"loop"}';     CONF="$(realpath "$FLUENTD_DIR/conf/fluentd_throughput_crash.conf")" ;;
  io)   TRIGGER='{"inject":"filetest"}'; CONF="$(realpath "$FLUENTD_DIR/conf/fluentd_error_tests.conf")" ;;
  cpu)  TRIGGER='{"inject":"cputest"}';  CONF="$(realpath "$FLUENTD_DIR/conf/fluentd_throughput_crash.conf")" ;;
  mem)  TRIGGER='{"inject":"memtest"}';  CONF="$(realpath "$FLUENTD_DIR/conf/fluentd_throughput_crash.conf")" ;;
  *)
    echo "Unknown test: $TEST"
    echo "Usage: $0 loop|io|cpu|mem"
    exit 1
    ;;
esac

if [[ "$TEST" != "io" ]]; then
    SINK_CSV="$FLUENTD_DIR/stats_${TEST}_sink.csv"
else
    SINK_CSV=""
fi

echo "[run_crash] test=$TEST"

SINK_PID=""
STATS_PID=""
INJECT_PID=""

cleanup() {
    echo ""
    echo "[run_crash] cleaning up..."
    sudo docker stop "$CONTAINER_NAME" 2>/dev/null || true
    sudo docker rm   "$CONTAINER_NAME" 2>/dev/null || true
    kill "$STATS_PID"  2>/dev/null || true
    kill "$INJECT_PID" 2>/dev/null || true
    pkill -P "$INJECT_PID" 2>/dev/null || true
    [[ -n "$SINK_PID" ]] && kill "$SINK_PID" 2>/dev/null || true
    truncate -s 0 "$LOG_FILE" 2>/dev/null || true
    echo "[run_crash] log file cleared"
}
trap cleanup EXIT INT TERM

# Remove any leftover container from a previous run
sudo docker rm -f "$CONTAINER_NAME" 2>/dev/null || true

# Reset docker stats CSV and log file
echo "timestamp,cpu_pct,mem_usage,mem_pct" > "$STATS_FILE"
truncate -s 0 "$LOG_FILE" 2>/dev/null || true
touch "$LOG_FILE"

# Start sink server for throughput tests
if [[ -n "$SINK_CSV" ]]; then
    echo "[run_crash] starting sink server (port 8080) -> $SINK_CSV"
    python3 "$TOOLS_DIR/sink_server.py" \
        --port 8080 --report-interval 1 --out "$SINK_CSV" &
    SINK_PID=$!
    sleep 1
fi

echo "[run_crash] launching container..."

DOCKER_ARGS=(
    -d
    --name "$CONTAINER_NAME"
    -v "${CONF}:/fluentd/etc/fluent.conf:ro"
    -v "${PLUGINS_DIR}:/fluentd/plugins:ro"
    -v "${LOG_FILE}:/fluentd/log/input.log"
)

if [ "$TEST" = "mem" ]; then
    DOCKER_ARGS+=(--memory=512m --memory-swap=512m)
fi

# host network lets container reach sink server at localhost:8080
if [[ -n "$SINK_CSV" ]]; then
    DOCKER_ARGS+=(--network host)
fi

sudo docker run "${DOCKER_ARGS[@]}" "$IMAGE"

# Background: record docker stats every second -> CSV
(
    while sudo docker ps --format '{{.Names}}' 2>/dev/null | grep -q "^${CONTAINER_NAME}$"; do
        LINE=$(sudo docker stats "$CONTAINER_NAME" --no-stream \
            --format "{{.CPUPerc}},{{.MemUsage}},{{.MemPerc}}" 2>/dev/null) || break
        [[ -z "$LINE" ]] && continue
        echo "$(date -Iseconds),$LINE" >> "$STATS_FILE"
        sleep 1
    done
) &
STATS_PID=$!

# Background: inject lifecycle
(
    # Wait until fluentd worker is ready before sending any logs
    echo "[inject] waiting for fluentd to be ready..."
    WAIT=0
    until sudo docker logs "$CONTAINER_NAME" 2>&1 | grep -q "fluentd worker is now running"; do
        sleep 1
        WAIT=$((WAIT + 1))
        if [ "$WAIT" -ge 30 ]; then
            echo "[inject] ERROR: fluentd did not become ready within 30s, aborting"
            exit 1
        fi
    done
    echo "[inject] fluentd is ready (${WAIT}s)"

    if [[ "$TEST" == "loop" || "$TEST" == "cpu" ]]; then
        # Phase 1: normal traffic (10s) so sink records a baseline
        echo "[inject] starting loggen ${LOGGEN_RATE} lps for ${TEST_DURATION}s ..."
        python3 "$TOOLS_DIR/loggen.py" \
            --mode steady --rate "$LOGGEN_RATE" --duration "$TEST_DURATION" \
            --output "$LOG_FILE" &
        LOGGEN_SUB=$!

        # Phase 2: inject trigger after 10s of normal traffic
        sleep 10
        echo "[inject] TRIGGER ($TEST) — traffic should drop to 0 at sink"
        echo "$TRIGGER" >> "$LOG_FILE"

        # Phase 3: wait out the rest of the test duration then stop
        sleep $((TEST_DURATION - 10))
        echo "[inject] ${TEST_DURATION}s done, stopping container"
        sudo docker stop "$CONTAINER_NAME" 2>/dev/null || true
        kill "$LOGGEN_SUB" 2>/dev/null || true
    elif [[ "$TEST" == "mem" ]]; then
        # Phase 1: normal traffic (10s) so sink records a baseline
        echo "[inject] starting loggen 30000 lps for ${TEST_DURATION}s ..."
        python3 "$TOOLS_DIR/loggen.py" \
            --mode steady --rate 80000 --duration "$TEST_DURATION" \
            --output "$LOG_FILE" &
        LOGGEN_SUB=$!

        # Phase 2: inject trigger after 10s of normal traffic
        sleep 10
        echo "[inject] TRIGGER ($TEST) — traffic should drop to 0 at sink"
        echo "$TRIGGER" >> "$LOG_FILE"

        # Phase 3: wait out the rest of the test duration then stop
        sleep $((TEST_DURATION - 10))
        echo "[inject] ${TEST_DURATION}s done, stopping container"
        sudo docker stop "$CONTAINER_NAME" 2>/dev/null || true
        kill "$LOGGEN_SUB" 2>/dev/null || true
    else
        # io test: stdout mode, observe file access results
        echo "[inject] seq:1  before trigger"
        echo '{"level":"INFO","msg":"before trigger","seq":1}' >> "$LOG_FILE"

        sleep 3
        echo "[inject] TRIGGER ($TEST) — checking file access"
        echo "$TRIGGER" >> "$LOG_FILE"

        sleep 10
        echo "[inject] seq:3  after trigger"
        echo '{"level":"INFO","msg":"after trigger","seq":3}' >> "$LOG_FILE"
        sleep 5
        python3 "$TOOLS_DIR/loggen.py" \
            --mode steady --rate 5 --duration 30 \
            --output "$LOG_FILE" 2>/dev/null &
        wait $!
        echo "[inject] loggen done"
    fi
) &
INJECT_PID=$!

echo "[run_crash] streaming container output (Ctrl+C to stop)..."
echo "---"
sudo docker logs -f "$CONTAINER_NAME" 2>&1 || true

echo "---"
echo "[run_crash] container exited"
echo "[run_crash] docker stats -> $STATS_FILE"
[[ -n "$SINK_CSV" ]] && echo "[run_crash] sink throughput -> $SINK_CSV"
