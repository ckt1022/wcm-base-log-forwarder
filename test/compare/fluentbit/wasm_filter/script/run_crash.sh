#!/usr/bin/env bash
# Usage: ./run_crash.sh <test>
#   test: loop | io | cpu | mem   (default: loop)
#
# loop / io / cpu / mem : 流量監測模式
#   先跑 loggen 正常轉發 10 秒 → 注入 trigger → 觀察 sink 流量/資源變化
#   輸出: stats_<test>.csv（docker stats）+ stats_<test>_sink.csv（接收端每秒流量）
#   loop / io / cpu : TEST_DURATION 秒後自動停止容器
#   mem             : OOM kill 自然結束，TEST_DURATION 為安全上限
#
# WASM 版本對照 Lua 版本差異:
#   ・每個 crash 情境各自編成獨立的 .wasm，透過 wasm_path 切換
#   ・loop/io/cpu 不設 Docker 記憶體限制，wasm_heap_size=1GB 的 mmap 為 lazy，不佔實體記憶體
#   ・mem: Docker --memory=512m；WAMR heap 另外 patch 為 64MB 以通過容器初始化，
#          Rust Vec 透過 WASM memory.grow 成長，累積到 512MB 後由 Docker OOM kill
#
# 每秒採樣一次 docker stats，結果存入 stats_<test>.csv

TEST="${1:-loop}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WASM_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
LOG_FILE="$(realpath "$WASM_DIR/../test-logs.log")"
WASM_FILE_DIR="$(realpath "$WASM_DIR/wasm_file")"
TOOLS_DIR="$(realpath "$WASM_DIR/../../../tools")"

CONTAINER_NAME="fb-wasm-${TEST}"
STATS_FILE="$WASM_DIR/stats_${TEST}.csv"
IMAGE="cr.fluentbit.io/fluent/fluent-bit:5.0.7"
TEST_DURATION=180   # seconds — sustained blocking tests

# mem needs high rate to fill 512 MB within TEST_DURATION (2 KB/record × 10 000 lps = 20 MB/s)
if [ "${1:-loop}" = "mem" ]; then
    LOGGEN_RATE=30000
else
    LOGGEN_RATE=10000
fi

case "$TEST" in
  loop) WASM_NAME="crash_loop"; TRIGGER='{"level":"ERROR","msg":"trigger","seq":2,"inject":"loop"}' ;;
  io)   WASM_NAME="crash_io";   TRIGGER='{"level":"ERROR","msg":"trigger","seq":2,"inject":"io"}' ;;
  cpu)  WASM_NAME="crash_cpu";  TRIGGER='{"level":"ERROR","msg":"trigger","seq":2,"inject":"cpu"}' ;;
  mem)  WASM_NAME="crash_mem";  TRIGGER='{"level":"ERROR","msg":"trigger","seq":2,"inject":"mem"}' ;;
  *)
    echo "Unknown test: $TEST"
    echo "Usage: $0 loop|io|cpu|mem"
    exit 1
    ;;
esac

WASM_PATH="$WASM_FILE_DIR/${WASM_NAME}.wasm"

# ── Pre-flight: WASM file must exist ─────────────────────────────────────────
if [ ! -f "$WASM_PATH" ]; then
    echo "[run_crash] ERROR: WASM file not found: $WASM_PATH"
    echo "[run_crash] Run  wasm_file/build.sh  first to compile the Rust sources."
    exit 1
fi

# All 4 tests use throughput conf (HTTP output to sink_server)
CONF="$(realpath "$WASM_DIR/conf/fluentbit_throughput_crash.conf")"
SINK_CSV="$WASM_DIR/stats_${TEST}_sink.csv"

echo "[run_crash] test=$TEST  wasm=${WASM_NAME}.wasm"

STATS_PID=""
INJECT_PID=""
SINK_PID=""

cleanup() {
    echo ""
    echo "[run_crash] cleaning up..."
    sudo docker stop "$CONTAINER_NAME" 2>/dev/null || true
    sudo docker rm   "$CONTAINER_NAME" 2>/dev/null || true
    [[ -n "$STATS_PID" ]]  && kill "$STATS_PID"  2>/dev/null || true
    [[ -n "$INJECT_PID" ]] && kill "$INJECT_PID" 2>/dev/null || true
    [[ -n "$INJECT_PID" ]] && pkill -P "$INJECT_PID" 2>/dev/null || true
    [[ -n "$SINK_PID" ]]   && kill "$SINK_PID"   2>/dev/null || true
    truncate -s 0 "$LOG_FILE" 2>/dev/null || true
    echo "[run_crash] log file cleared"
}
trap cleanup EXIT INT TERM

# Remove any leftover container from a previous run
sudo docker rm -f "$CONTAINER_NAME" 2>/dev/null || true

# ── Patch conf: switch to the target WASM file ───────────────────────────────
sed -i "s|wasm_path .*|wasm_path         /fluent-bit/wasm/${WASM_NAME}.wasm|" "$CONF"

# ── mem: reduce WAMR heap so container can initialise within Docker's 512m ───
# WAMR's heap (wasm_heap_size) is mmap'd at startup. With --memory=512m the
# 1 GB default fails. 64 MB is enough for WAMR internals; Rust's Vec/Box
# allocations use WASM linear memory (memory.grow) which is separate and
# uncapped — it grows until Docker OOM-kills the container (exit 137).
if [ "$TEST" = "mem" ]; then
    sed -i "s|wasm_heap_size .*|wasm_heap_size    67108864|" "$CONF"
else
    sed -i "s|wasm_heap_size .*|wasm_heap_size    1073741824|" "$CONF"
fi

# ── Reset stats CSV and log file ─────────────────────────────────────────────
echo "timestamp,cpu_pct,mem_usage,mem_pct" > "$STATS_FILE"
truncate -s 0 "$LOG_FILE" 2>/dev/null || true

# ── Start sink server ─────────────────────────────────────────────────────────
echo "[run_crash] starting sink server (port 8080) -> $SINK_CSV"
python3 "$TOOLS_DIR/sink_server.py" \
    --port 8080 --report-interval 1 --out "$SINK_CSV" &
SINK_PID=$!
sleep 1

# ── Launch Fluent Bit container ───────────────────────────────────────────────
echo "[run_crash] launching container..."

DOCKER_ARGS=(
    -d
    --name "$CONTAINER_NAME"
    --network host                         # reach sink_server at localhost:8080
    -v "${CONF}:/fluent-bit/etc/fluent-bit.conf:ro"
    -v "${WASM_FILE_DIR}:/fluent-bit/wasm:ro"
    -v "${LOG_FILE}:/fluent-bit/test-logs.log"
)

# mem: cap container memory so Docker OOM-kills at 512 MB
if [ "$TEST" = "mem" ]; then
    DOCKER_ARGS+=(--memory=512m --memory-swap=512m)
fi

sudo docker run "${DOCKER_ARGS[@]}" "$IMAGE"

# ── Background: docker stats → CSV ───────────────────────────────────────────
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

# ── Background: inject lifecycle ─────────────────────────────────────────────
(
    # Phase 1: normal traffic for 10 s so sink records a clean baseline
    sleep 2
    echo "[inject] starting loggen ${LOGGEN_RATE} lps for ${TEST_DURATION}s ..."
    python3 "$TOOLS_DIR/loggen.py" \
        --mode steady --rate "$LOGGEN_RATE" --duration "$TEST_DURATION" \
        --output "$LOG_FILE" &
    LOGGEN_SUB=$!

    # Phase 2: inject trigger after 10 s of normal traffic
    sleep 10
    echo "[inject] TRIGGER ($TEST) — throughput at sink should drop to 0"
    echo "$TRIGGER" >> "$LOG_FILE"

    # Phase 3: wait out the rest of the test duration, then stop
    # mem: OOM kill may fire before this; that is the expected outcome.
    sleep $((TEST_DURATION - 10))
    echo "[inject] ${TEST_DURATION}s elapsed, stopping container"
    sudo docker stop "$CONTAINER_NAME" 2>/dev/null || true
    kill "$LOGGEN_SUB" 2>/dev/null || true
) &
INJECT_PID=$!

# ── Stream container stdout until it exits ───────────────────────────────────
echo "[run_crash] streaming container output (Ctrl+C to stop)..."
echo "---"
sudo docker logs -f "$CONTAINER_NAME" 2>&1 || true

echo "---"
EXIT_CODE=$(sudo docker inspect "$CONTAINER_NAME" --format='{{.State.ExitCode}}' 2>/dev/null || echo "?")
echo "[run_crash] container exited  (exit code: $EXIT_CODE)"
[[ "$EXIT_CODE" == "137" ]] && echo "[run_crash] exit 137 = OOM kill (Docker --memory limit hit)"
echo "[run_crash] docker stats    -> $STATS_FILE"
echo "[run_crash] sink throughput -> $SINK_CSV"
