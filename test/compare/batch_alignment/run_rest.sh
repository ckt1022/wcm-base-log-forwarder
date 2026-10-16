#!/bin/bash
# run_rest.sh — 方法 3 延遲佇列之後依序執行：方法 2 → 方法 3 容量粗搜尋（W=1.0 也在目前時鐘下重測；WCM W=1.0 最後）→ queue_last.txt。
set -uo pipefail
A="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNS_DIR="${RUNS_DIR:-$A/runs}"; export RUNS_DIR
[[ -n "${WAIT_PID:-}" ]] && while kill -0 "$WAIT_PID" 2>/dev/null; do sleep 20; done
stop() { [[ -f "$A/STOP" ]] && { echo "STOP"; exit 0; }; }
bash "$A/run_queue.sh" "$A/queue_m2.txt"; stop
S="$A/search_align.sh"
for spec in \
  "fbit-lua m3_W1 conf/fbit_lua_flush_1.conf 67500" \
  "fluentd m3_W1 conf/fluentd_m3_W1.conf 52000" \
  "vector m3_W1 conf/vector_align.yaml 15000 FLUSH_INTERVAL=1" \
  "fbit-wasm m3_W1 conf/fbit_wasm_flush_1.conf 5400" \
  "fbit-lua m3_W0.5 conf/fbit_lua_flush_0.5.conf 67500" \
  "fluentd m3_W0.5 conf/fluentd_m3_W0.5.conf 52000" \
  "vector m3_W0.5 conf/vector_align.yaml 15000 FLUSH_INTERVAL=0.5" \
  "wcm m3_W0.5 conf/wcm_tw128_wait500.yaml 25000" \
  "fbit-wasm m3_W0.5 conf/fbit_wasm_flush_0.5.conf 5400" \
  "fbit-lua m3_W0.1 conf/fbit_lua_flush_0.1.conf 62000" \
  "fluentd m3_W0.1 conf/fluentd_m3_W0.1.conf 50000" \
  "vector m3_W0.1 conf/vector_align.yaml 14000 FLUSH_INTERVAL=0.1" \
  "wcm m3_W0.1 conf/wcm_tw128_wait100.yaml 24000" \
  "fbit-wasm m3_W0.1 conf/fbit_wasm_flush_0.1.conf 5200" \
  "fbit-lua m3_W0.01 conf/fbit_lua_flush_0.01.conf 45000" \
  "fluentd m3_W0.01 conf/fluentd_m3_W0.01.conf 40000" \
  "vector m3_W0.01 conf/vector_align.yaml 10000 FLUSH_INTERVAL=0.01" \
  "wcm m3_W0.01 conf/wcm_tw128_wait10.yaml 18000" \
  "fbit-wasm m3_W0.01 conf/fbit_wasm_flush_0.01.conf 4500" \
  "wcm m3_W1 conf/wcm_tw128_wait1000.yaml 25000" ; do
  read -r dut tag conf start extra <<< "$spec"
  if grep -q "^CAPACITY" "$RUNS_DIR/$dut/search_${tag}.txt" 2>/dev/null; then echo "skip search $dut $tag"; continue; fi
  echo "[rest] $(date -Is) search $dut $tag start=$start"
  bash "$S" "$dut" "$tag" "$conf" "$start" $extra
  stop
done
echo "[rest] $(date -Is) all searches done"
bash "$A/run_queue.sh" "$A/queue_last.txt"
