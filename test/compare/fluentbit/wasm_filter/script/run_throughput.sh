#!/usr/bin/env bash
# 純吞吐量基線測試 — 無 crash 注入，量測 WASM filter 正常運作時的最大吞吐量。
#
# Usage: ./run_throughput.sh [rate] [duration]
#   rate     lines/sec  (default: 5000)
#   duration seconds    (default: 120)
#
# 輸出:
#   stats_throughput.csv      — docker stats（CPU / 記憶體，每秒一筆）
#   stats_throughput_sink.csv — sink_server 每秒接收量

RATE="${1:-5000}"
DURATION="${2:-120}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
WASM_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
LOG_FILE="$(realpath "$WASM_DIR/../test-logs.log")"
WASM_FILE_DIR="$(realpath "$WASM_DIR/wasm_file")"
TOOLS_DIR="$(realpath "$WASM_DIR/../../../tools")"

CONTAINER_NAME="fb-wasm-throughput"
STATS_FILE="$WASM_DIR/stats_throughput.csv"
SINK_CSV="$WASM_DIR/stats_throughput_sink.csv"
IMAGE="cr.fluentbit.io/fluent/fluent-bit:5.0.7"

# 基線測試使用 crash_loop.wasm（不會觸發 loop，因為沒有 "loop" trigger 字串）
WASM_NAME="crash_loop"
WASM_PATH="$WASM_FILE_DIR/${WASM_NAME}.wasm"
CONF="$(realpath "$WASM_DIR/conf/fluentbit_throughput_crash.conf")"

if [ ! -f "$WASM_PATH" ]; then
    echo "[run_throughput] ERROR: $WASM_PATH not found"
    echo "[run_throughput] Run  wasm_file/build.sh  first."
    exit 1
fi

echo "[run_throughput] rate=${RATE} lps  duration=${DURATION}s"

STATS_PID=""
LOGGEN_PID=""
SINK_PID=""

cleanup() {
    echo ""
    echo "[run_throughput] cleaning up..."
    sudo docker stop "$CONTAINER_NAME" 2>/dev/null || true
    sudo docker rm   "$CONTAINER_NAME" 2>/dev/null || true
    [[ -n "$STATS_PID" ]]  && kill "$STATS_PID"  2>/dev/null || true
    [[ -n "$LOGGEN_PID" ]] && kill "$LOGGEN_PID" 2>/dev/null || true
    [[ -n "$SINK_PID" ]]   && kill "$SINK_PID"   2>/dev/null || true
    truncate -s 0 "$LOG_FILE" 2>/dev/null || true
    echo "[run_throughput] log file cleared"
}
trap cleanup EXIT INT TERM

sudo docker rm -f "$CONTAINER_NAME" 2>/dev/null || true

# 確保 conf 指向 crash_loop.wasm（不觸發任何 crash）
sed -i "s|wasm_path .*|wasm_path         /fluent-bit/wasm/${WASM_NAME}.wasm|" "$CONF"

echo "timestamp,cpu_pct,mem_usage,mem_pct" > "$STATS_FILE"
truncate -s 0 "$LOG_FILE" 2>/dev/null || true

echo "[run_throughput] starting sink server (port 8080) -> $SINK_CSV"
python3 "$TOOLS_DIR/sink_server.py" \
    --port 8080 --report-interval 1 --out "$SINK_CSV" &
SINK_PID=$!
sleep 1

echo "[run_throughput] launching container..."
sudo docker run -d \
    --name "$CONTAINER_NAME" \
    --network host \
    -v "${CONF}:/fluent-bit/etc/fluent-bit.conf:ro" \
    -v "${WASM_FILE_DIR}:/fluent-bit/wasm:ro" \
    -v "${LOG_FILE}:/fluent-bit/test-logs.log" \
    "$IMAGE"

# docker stats → CSV
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

# loggen
sleep 2
echo "[run_throughput] starting loggen ${RATE} lps for ${DURATION}s ..."
python3 "$TOOLS_DIR/loggen.py" \
    --mode steady --rate "$RATE" --duration "$DURATION" \
    --output "$LOG_FILE" &
LOGGEN_PID=$!

# wait for loggen then stop container
wait "$LOGGEN_PID" 2>/dev/null || true
echo "[run_throughput] loggen done, stopping container..."
sudo docker stop "$CONTAINER_NAME" 2>/dev/null || true

echo ""
echo "[run_throughput] complete"
echo "[run_throughput] docker stats  -> $STATS_FILE"
echo "[run_throughput] sink throughput -> $SINK_CSV"
