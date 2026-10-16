#!/bin/bash
# 本機（非容器）功能 smoke test：以 C3 型態的輸入（5% malformed）驗證 plugin_call_size。
# 使用非正式埠 15140/18080，避免與正式量測衝突；整段在共用量測鎖內執行。
# 用法: smoke_native.sh <config.yaml> <out_dir> [count] [rate]
set -uo pipefail
CFG="$1"; OUT="$2"; COUNT="${3:-5000}"; RATE="${4:-2500}"
WT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
BIN="$WT/target/release/wcm-base-log-forwarder"
CORR=/home/ckt1022/wcm-base-log-forwarder/test/compare/correctness
mkdir -p "$OUT"
"$CORR/validationsink_bin" -port 18080 -out-dir "$OUT" -input-count "$COUNT" -malformed > "$OUT/sink.log" 2>&1 &
SINK=$!
"$BIN" "$CFG" > "$OUT/dut.log" 2>&1 &
DUT=$!
for _ in $(seq 60); do
    timeout 1 bash -c 'echo -n > /dev/tcp/127.0.0.1/15140' 2>/dev/null && break
    sleep 0.5
done
sleep 1
"$CORR/correctgen_bin" -rate "$RATE" -count "$COUNT" -malformed -tcp-addr 127.0.0.1:15140 \
    -summary "$OUT/generator_summary.json" > "$OUT/generator.log" 2>&1
sleep 5
kill -TERM "$DUT" 2>/dev/null; wait "$DUT" 2>/dev/null
kill -TERM "$SINK" 2>/dev/null; wait "$SINK" 2>/dev/null
python3 - "$OUT" <<'EOF'
import json, sys
s = json.load(open(sys.argv[1] + "/validation_summary.json"))
print({k: s[k] for k in ("expected", "received", "unique", "missing", "duplicate", "corrupt", "unexpected", "order_inversions", "bad_output_json")})
EOF
grep -c "parse-subcall\|parse-error" "$OUT/dut.log"
