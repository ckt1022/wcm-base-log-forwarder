#!/bin/bash
# [streaming_validation] batch_alignment/run_point.sh 的複本；改呼叫本目錄的 capacity_align.sh（WCM_IMAGE 可覆寫），
# runs.log 寫到 streaming_validation/logs/runs.log。
# run_point.sh — 在共享量測鎖內跑「一個完整試驗」(含 container 啟停與分析)。
# 用法: CONF=... TAG=... [其他 env] ./run_point.sh <dut> <rate> <cap|lat>
# lat 模式額外以正式 sweep 相同的窗口（30–150 s）計算延遲與 cgroup 資源（含時鐘跳躍校正，見 correct_latency.py）。
# 結果行附加到 batch_alignment/logs/runs.log。
set -uo pipefail
SV_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ALIGN_DIR="$(cd "$SV_DIR/../batch_alignment" && pwd)"
RUNS_DIR="${RUNS_DIR:-$SV_DIR/runs}"; export RUNS_DIR; mkdir -p "$RUNS_DIR" "$RUNS_DIR/../logs${RUNS_SUFFIX:-}"
CAP_DIR="$ALIGN_DIR/../capacity"
LOCK="${LOCK:-/home/ckt1022/.wcm-bench.lock}"
DUT="$1"; RATE="$2"; MODE="$3"
OUT_DIR="$RUNS_DIR/$DUT/${MODE}_rate_${RATE}_${TAG}"
export SV_DIR ALIGN_DIR CAP_DIR DUT RATE MODE OUT_DIR

flock "$LOCK" bash -c '
    t0=$(date +%s)
    SKIP_CLOCK_CHECK=1 bash "$SV_DIR/capacity_align.sh" "$DUT" "$RATE" "$MODE"
    rc=$?
    if [[ "$MODE" == "lat" && -s "$OUT_DIR/latency_worker_1.csv" ]]; then
        # 時鐘跳躍校正後的 30–150 s 延遲 / 資源統計（未校正版另存 *_raw.txt）
        python3 "$ALIGN_DIR/correct_latency.py" "$OUT_DIR"
    fi
    echo "$(date -Is) rc=$rc dur=$(( $(date +%s) - t0 ))s dut=$DUT rate=$RATE mode=$MODE tag=$TAG :: $(cat "$OUT_DIR/verdict.txt" 2>/dev/null | head -1)" \
        >> "$RUNS_DIR/../logs${RUNS_SUFFIX:-}/runs.log"
    exit $rc
'
