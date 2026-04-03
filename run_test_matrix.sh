#!/usr/bin/env bash
set -euo pipefail

BINARY="/home/ckt1022/wcm-base-log-forwarder/target/release/wcm-base-log-forwarder"
DURATION=120  # 2 minutes per scenario
RESULTS_DIR="/home/ckt1022/wcm-base-log-forwarder/test_results"

mkdir -p "$RESULTS_DIR"

# ─────────────────────────────────────────────
# Generator: small line (~90 bytes)
# ─────────────────────────────────────────────
gen_small() {
python3 -c "
import sys, time, itertools

end = time.time() + $DURATION
levels = ['info', 'debug', 'warn', 'error']
i = 0
for level in itertools.cycle(levels):
    if time.time() > end:
        break
    line = '{\"ts\":\"2024-01-01T00:00:00Z\",\"level\":\"' + level + '\",\"msg\":\"ok ' + str(i) + '\",\"att\":{}}'
    sys.stdout.buffer.write(line.encode() + b'\n')
    i += 1
sys.stdout.buffer.flush()
"
}

# ─────────────────────────────────────────────
# Generator: medium line (~500 bytes)
# ─────────────────────────────────────────────
gen_medium() {
python3 -c "
import sys, time, itertools, json

end = time.time() + $DURATION
levels = ['info', 'debug', 'warn', 'error']
i = 0
for level in itertools.cycle(levels):
    if time.time() > end:
        break
    obj = {
        'ts': '2024-01-01T00:00:00Z',
        'level': level,
        'msg': 'processing request ' + str(i),
        'att': {
            'service': 'api-gateway',
            'host': 'node-42.prod.example.com',
            'trace_id': 'abc123def456' + str(i % 9999),
            'span_id': 'span-' + str(i),
            'http_method': 'POST',
            'http_path': '/api/v2/logs/ingest',
            'status_code': '200',
            'latency_ms': str(i % 500),
        }
    }
    line = json.dumps(obj)
    sys.stdout.buffer.write(line.encode() + b'\n')
    i += 1
sys.stdout.buffer.flush()
"
}

# ─────────────────────────────────────────────
# Generator: large line (~2000 bytes)
# ─────────────────────────────────────────────
gen_large() {
python3 -c "
import sys, time, itertools, json

end = time.time() + $DURATION
levels = ['info', 'debug', 'warn', 'error']
i = 0
for level in itertools.cycle(levels):
    if time.time() > end:
        break
    att = {('key_' + str(k)): ('value_' + 'x' * 20 + str(k)) for k in range(40)}
    att['service'] = 'heavy-worker'
    att['trace_id'] = 'trace-' + str(i)
    att['request_body'] = 'BODY-' + 'A' * 100 + str(i % 9999)
    obj = {
        'ts': '2024-01-01T00:00:00Z',
        'level': level,
        'msg': 'heavy log line with many attributes for memory pressure testing ' + str(i),
        'att': att
    }
    line = json.dumps(obj)
    sys.stdout.buffer.write(line.encode() + b'\n')
    i += 1
sys.stdout.buffer.flush()
"
}

# ─────────────────────────────────────────────
# Generator: slow input (1 line / 200ms) -> tests time-based flush
# ─────────────────────────────────────────────
gen_slow() {
python3 -c "
import sys, time, itertools

end = time.time() + $DURATION
levels = ['info', 'debug', 'warn', 'error']
i = 0
for level in itertools.cycle(levels):
    if time.time() > end:
        break
    line = '{\"ts\":\"2024-01-01T00:00:00Z\",\"level\":\"' + level + '\",\"msg\":\"slow tick ' + str(i) + '\",\"att\":{\"i\":\"' + str(i) + '\"}}'
    sys.stdout.buffer.write(line.encode() + b'\n')
    sys.stdout.buffer.flush()
    time.sleep(0.2)
    i += 1
"
}

# ─────────────────────────────────────────────
# Run one scenario, capture stdout+stderr
# ─────────────────────────────────────────────
run_scenario() {
    local name="$1"
    local gen_fn="$2"
    local out="$RESULTS_DIR/${name}.txt"

    echo ""
    echo "============================================================"
    echo "  SCENARIO: $name  (${DURATION}s)"
    echo "============================================================"

    # Run generator piped to binary; timeout kills the pipeline after DURATION+5s
    if $gen_fn | timeout $(( DURATION + 5 )) "$BINARY" > "$out" 2>&1; then
        echo "[OK] finished normally"
    else
        echo "[WARN] exited with code $? (expected for timeout-based runs)"
    fi

    echo "Output saved to: $out"
}

echo "=== WCM Log Forwarder - Test Matrix ==="
echo "Binary : $BINARY"
echo "Duration per scenario: ${DURATION}s"
echo "Results: $RESULTS_DIR"
echo ""

run_scenario "A_small_lines_full_speed" gen_small
run_scenario "B_medium_lines_full_speed" gen_medium
run_scenario "C_large_lines_full_speed" gen_large
run_scenario "D_slow_input_time_flush" gen_slow

echo ""
echo "=== All scenarios done ==="
