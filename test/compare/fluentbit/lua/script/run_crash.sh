#!/usr/bin/env bash
# Usage: ./run_test.sh <test>
#   test: loop | io | cpu | mem | parse   (default: loop)
#
# Automatically updates the conf's `call` line, injects 3 log lines
# with delays, and runs Fluent Bit so the before/trigger/after sequence
# is visible in stdout.

TEST="${1:-loop}"

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
LUA_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
LOG_FILE="$LUA_DIR/../test-logs.log"
CONF="$LUA_DIR/conf/fluentbit_error_tests.conf"

# Map test name to Lua function and trigger payload
case "$TEST" in
  loop)  FUNC="test_infinite_loop";    TRIGGER='{"level":"ERROR","msg":"trigger","seq":2,"inject":"loop"}' ;;
  io)    FUNC="test_io_blocking";      TRIGGER='{"level":"ERROR","msg":"trigger","seq":2,"inject":"io"}' ;;
  cpu)   FUNC="test_cpu_exhaustion";   TRIGGER='{"level":"ERROR","msg":"trigger","seq":2,"inject":"cpu"}' ;;
  mem)   FUNC="test_memory_exhaustion"; TRIGGER='{"level":"ERROR","msg":"trigger","seq":2,"inject":"mem"}' ;;
  parse) FUNC="test_parse_error";      TRIGGER='{broken json -- this line is malformed' ;;
  *)
    echo "Unknown test: $TEST"
    echo "Usage: $0 loop|io|cpu|mem|parse"
    exit 1
    ;;
esac

echo "[run_test] test=$TEST  func=$FUNC"

# Kill any previous instance
pkill -f "fluent-bit.*fluentbit_error_tests" 2>/dev/null || true
sleep 1

# Update the `call` line in conf
sed -i "s/^    call   .*/    call   $FUNC/" "$CONF"

# mem test needs protected_mode off so OOM actually kills the process
# all other tests use protected_mode on
if [ "$TEST" = "mem" ]; then
    sed -i "s/protected_mode .*/protected_mode true/" "$CONF"
else
    sed -i "s/protected_mode .*/protected_mode true/" "$CONF"
fi

echo "[run_test] cleaning up..."
truncate -s 0 "$LOG_FILE" 2>/dev/null || true

# Inject lines in background after Fluent Bit is ready
(
  sleep 3
  echo "[inject] seq:1 -- normal record before trigger"
  echo '{"level":"INFO","msg":"before trigger","seq":1}' >> "$LOG_FILE"

  sleep 3
  echo "[inject] seq:2 -- TRIGGER ($TEST)"
  echo "$TRIGGER" >> "$LOG_FILE"

  sleep 3
  echo "[inject] seq:3 -- after trigger (should NOT appear for loop/io/cpu/mem)"
  echo '{"level":"INFO","msg":"after trigger","seq":3}' >> "$LOG_FILE"

  sleep 3
  echo "[inject] starting loggen for 30s -- recovery test"
  python3 "$LUA_DIR/../../../tools/loggen.py" \
    --mode steady --rate 5 --duration 30 \
    --output "$LOG_FILE" &
  LOGGEN_PID=$!

  wait "$LOGGEN_PID"
  echo "[inject] loggen done -- press Ctrl+C to stop fluent-bit"
) &
INJECT_PID=$!

echo "[run_test] starting fluent-bit..."
echo "---"
cd "$LUA_DIR"
/opt/fluent-bit/bin/fluent-bit -c "$CONF" || true

kill "$INJECT_PID" 2>/dev/null || true
