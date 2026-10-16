#!/bin/bash
# 依序跑所有 fault isolation case（每個 case 內部各自 flock 整個 run）。
set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUN_ID="${RUN_ID:-native_$(date +%Y%m%d_%H%M%S)}"
run() { CASE="$1" MARKER="$2" RUN_ID="$RUN_ID" bash "$SCRIPT_DIR/run_fault.sh"; }
run baseline ""
run loop  FAULT_LOOP
run mem   FAULT_MEM
run panic FAULT_PANIC
run oob   FAULT_OOB
run file  FAULT_FILE
run env   FAULT_ENV
echo "[fault] all cases done: $SCRIPT_DIR/../runs/$RUN_ID"
