#!/usr/bin/env bash
# Usage: ./run_crash.sh <test>
#   test: loop | io | cpu | mem | parse   (default: loop)
#
# loop / io / cpu / mem : 流量監測模式
#   先跑 loggen 正常轉發 10 秒 → 注入 trigger → 觀察 sink 流量/資源變化
#   輸出: stats_<test>.csv（docker stats）+ stats_<test>_sink.csv（接收端每秒流量）
#   loop/io/cpu : 3 分鐘後自動停止容器
#   mem         : OOM kill 自然結束，3 分鐘為安全上限
# parse : stdout 模式，等待容器自然結束
#
# 每秒採樣一次 docker stats，結果存入 stats_<test>.csv

TEST="${1:-loop}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
LUA_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
LOG_FILE="$(realpath "$LUA_DIR/../test-logs.log")"
LUA_SCRIPT="$(realpath "$LUA_DIR/test_errors.lua")"
TOOLS_DIR="$(realpath "$LUA_DIR/../../../tools")"

CONTAINER_NAME="fb-lua-${TEST}"
STATS_FILE="$LUA_DIR/stats_${TEST}.csv"
IMAGE="cr.fluentbit.io/fluent/fluent-bit:5.0.7"
TEST_DURATION=180   # seconds — for loop / io / cpu / mem sustained tests
# mem needs high rate to fill 512 MB within TEST_DURATION; others use low rate for clarity
if [ "${1:-loop}" = "mem" ]; then
    LOGGEN_RATE=10000
else
    LOGGEN_RATE=10000
fi

case "$TEST" in
  loop)  FUNC="test_infinite_loop";     TRIGGER='{"level":"ERROR","msg":"trigger","seq":2,"inject":"loop"}' ;;
  io)    FUNC="test_io_blocking";       TRIGGER='{"level":"ERROR","msg":"trigger","seq":2,"inject":"io"}' ;;
  cpu)   FUNC="test_cpu_exhaustion";    TRIGGER='{"level":"ERROR","msg":"trigger","seq":2,"inject":"cpu"}' ;;
  mem)   FUNC="test_memory_exhaustion"; TRIGGER='{"level":"ERROR","msg":"trigger","seq":2,"inject":"mem"}' ;;
  parse) FUNC="test_parse_error";       TRIGGER='{broken json -- this line is malformed' ;;
  *)
    echo "Unknown test: $TEST"
    echo "Usage: $0 loop|io|cpu|mem|parse"
    exit 1
    ;;
esac

# loop/io/cpu/mem: throughput conf (HTTP output to sink)
# parse: error conf (stdout only)
if [[ "$TEST" == "loop" || "$TEST" == "io" || "$TEST" == "cpu" || "$TEST" == "mem" ]]; then
    CONF="$(realpath "$LUA_DIR/conf/fluentbit_throughput_crash.conf")"
    SINK_CSV="$LUA_DIR/stats_${TEST}_sink.csv"
else
    CONF="$(realpath "$LUA_DIR/conf/fluentbit_error_tests.conf")"
    SINK_CSV=""
fi

echo "[run_crash] test=$TEST  func=$FUNC"

SINK_PID=""

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

# Patch conf: switch to target function
sed -i "s/^    call   .*/    call   $FUNC/" "$CONF"

# mem needs protected_mode off so OOM actually terminates the process
if [ "$TEST" = "mem" ]; then
    sed -i "s/protected_mode .*/protected_mode false/" "$CONF"
else
    sed -i "s/protected_mode .*/protected_mode true/" "$CONF"
fi

# Reset docker stats CSV and log file
echo "timestamp,cpu_pct,mem_usage,mem_pct" > "$STATS_FILE"
truncate -s 0 "$LOG_FILE" 2>/dev/null || true

# loop/io/cpu/mem: start sink server before container
if [[ -n "$SINK_CSV" ]]; then
    echo "[run_crash] starting sink server (port 8080) -> $SINK_CSV"
    python3 "$TOOLS_DIR/sink_server.py" \
        --port 8080 --report-interval 1 --out "$SINK_CSV" &
    SINK_PID=$!
    sleep 1
fi

echo "[run_crash] launching container..."

# Volume mounts:
#   conf   -> /fluent-bit/etc/fluent-bit.conf
#   script -> /fluent-bit/test_errors.lua   (conf's "Script ../test_errors.lua" resolves here)
#   log    -> /fluent-bit/test-logs.log     (conf's "Path   ../test-logs.log"   resolves here)
DOCKER_ARGS=(
    -d
    --name "$CONTAINER_NAME"
    -v "${CONF}:/fluent-bit/etc/fluent-bit.conf:ro"
    -v "${LUA_SCRIPT}:/fluent-bit/test_errors.lua:ro"
    -v "${LOG_FILE}:/fluent-bit/test-logs.log"
)

if [ "$TEST" = "mem" ]; then
    DOCKER_ARGS+=(--memory=512m --memory-swap=512m)
fi

# loop/io/cpu/mem: host network lets container reach sink server at localhost:8080
if [[ -n "$SINK_CSV" ]]; then
    DOCKER_ARGS+=(--network host)
fi

# io: -i keeps stdin open as a pipe so io.read() blocks instead of returning EOF
if [ "$TEST" = "io" ]; then
    DOCKER_ARGS+=(-i)
fi

sudo docker run "${DOCKER_ARGS[@]}" "$IMAGE"

# Background: record docker stats every second -> CSV
# fix: sudo on both docker ps and docker stats
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
    if [[ "$TEST" == "loop" || "$TEST" == "io" || "$TEST" == "cpu" || "$TEST" == "mem" ]]; then
        # Phase 1: normal traffic (10s) so sink records a baseline
        sleep 2
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
    else
        sleep 3
        echo "[inject] seq:1  before trigger"
        echo '{"level":"INFO","msg":"before trigger","seq":1}' >> "$LOG_FILE"

        sleep 3
        echo "[inject] seq:2  TRIGGER ($TEST)"
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
