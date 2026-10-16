#!/bin/bash
# Fault isolation 實驗：對 WCM host 注入單一 marker 故障，觀察
#   host 是否存活、遺失幾筆、故障到吞吐恢復的時間、故障期間 CPU/RSS、
#   以及 error.txt 與 stderr 的內容。
#
# 固定穩定負載 5,000 EPS × 1 KiB，故障在第 FAULT_AT 秒注入。
# 目前為「原生（無容器、無 cgroup）」執行，以 taskset 釘核心；Docker 可用時
# 另有容器版（見本目錄 README）。每一個 case 在 /home/ckt1022/.wcm-bench.lock 內獨占執行。
#
# 用法：
#   CASE=loop  MARKER=FAULT_LOOP  bash run_fault.sh
#   CASE=mem   MARKER=FAULT_MEM   bash run_fault.sh
#   CASE=panic MARKER=FAULT_PANIC bash run_fault.sh
#   CASE=oob   MARKER=FAULT_OOB   bash run_fault.sh
#   CASE=file  MARKER=FAULT_FILE  bash run_fault.sh
#   CASE=env   MARKER=FAULT_ENV   bash run_fault.sh
#   CASE=baseline MARKER="" bash run_fault.sh   # 無故障對照
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
FI_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$FI_DIR/../../.." && pwd)"
BIN="${BIN:-$REPO_ROOT/target/release/wcm-base-log-forwarder}"
SINK_BIN="$REPO_ROOT/test/compare/capacity/httpsink_bin"
CONFIG="$FI_DIR/forwarder_fault.yaml"
CASE="${CASE:-baseline}"
MARKER="${MARKER:-}"
MARKER_COUNT="${MARKER_COUNT:-1}"
EPS="${EPS:-5000}"
SECS="${SECS:-40}"
FAULT_AT="${FAULT_AT:-15}"
RUN_ID="${RUN_ID:-$(date +%Y%m%d_%H%M%S)}"
OUT="$FI_DIR/runs/${RUN_ID}/${CASE}"
DUT_CORE="${DUT_CORE:-4}"
SINK_CORE="${SINK_CORE:-8}"
GEN_CORE="${GEN_CORE:-9,10}"
LOCK="${LOCK:-/home/ckt1022/.wcm-bench.lock}"

[[ -x "$SINK_BIN" ]] || ( cd "$REPO_ROOT/test/compare/capacity/httpsink" && go build -o "$SINK_BIN" main.go )
mkdir -p "$OUT"

run_case() {
    taskset -c "$SINK_CORE" "$SINK_BIN" -port 8080 -out-dir "$OUT" -parse-ts=false > "$OUT/sink.log" 2>&1 &
    local sink_pid=$!
    WCM_ERROR_FILE="$OUT/error.txt" taskset -c "$DUT_CORE" "$BIN" "$CONFIG" > "$OUT/dut.log" 2>&1 &
    local dut_pid=$!

    local ready=false
    for _ in $(seq 1 60); do
        if timeout 1 bash -c 'echo -n > /dev/tcp/127.0.0.1/5140' 2>/dev/null; then ready=true; break; fi
        sleep 1
    done
    if [[ "$ready" != true ]]; then echo "DUT not ready" >&2; kill "$dut_pid" "$sink_pid" 2>/dev/null; return 1; fi
    sleep 2
    "$REPO_ROOT/test/compare/sample_stats.sh" "$dut_pid" "$OUT/stats.csv" 512m 1 tree &
    local stats_pid=$!

    local gen_args=(--addr 127.0.0.1:5140 --eps "$EPS" --secs "$SECS" --fault-at "$FAULT_AT" --summary "$OUT/generator_summary.json")
    [[ -n "$MARKER" ]] && gen_args+=(--marker "$MARKER" --marker-count "$MARKER_COUNT")
    python3 -I "$SCRIPT_DIR/fault_gen.py" "${gen_args[@]}" | tee "$OUT/generator.log"

    sleep 8   # drain
    kill "$stats_pid" 2>/dev/null || true; wait "$stats_pid" 2>/dev/null || true
    if kill -0 "$dut_pid" 2>/dev/null; then echo alive > "$OUT/host_status.txt"; else echo "dead(exit)" > "$OUT/host_status.txt"; fi
    kill -TERM "$dut_pid" 2>/dev/null || true; wait "$dut_pid" 2>/dev/null || true
    kill -TERM "$sink_pid" 2>/dev/null || true; wait "$sink_pid" 2>/dev/null || true
    {
        echo "case=$CASE marker=$MARKER marker_count=$MARKER_COUNT eps=$EPS secs=$SECS fault_at=$FAULT_AT"
        echo "mode=native (no container/cgroup) pinning: dut=core$DUT_CORE sink=core$SINK_CORE gen=core$GEN_CORE"
        echo "binary_sha256=$(sha256sum "$BIN" | cut -d' ' -f1)"
        echo "date=$(date -Is)"
    } > "$OUT/meta.txt"
}

flock "$LOCK" bash -c "$(declare -f run_case); OUT='$OUT' CASE='$CASE' MARKER='$MARKER' MARKER_COUNT='$MARKER_COUNT' \
    EPS='$EPS' SECS='$SECS' FAULT_AT='$FAULT_AT' BIN='$BIN' SINK_BIN='$SINK_BIN' CONFIG='$CONFIG' \
    REPO_ROOT='$REPO_ROOT' SCRIPT_DIR='$SCRIPT_DIR' DUT_CORE='$DUT_CORE' SINK_CORE='$SINK_CORE' GEN_CORE='$GEN_CORE' run_case"
echo "[fault] $CASE done → $OUT (host: $(cat "$OUT/host_status.txt"))"
