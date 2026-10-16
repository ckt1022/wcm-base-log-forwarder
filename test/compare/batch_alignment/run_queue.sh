#!/bin/bash
# run_queue.sh — 依序執行 queue 檔中的試驗；每一行 = 一個完整試驗，各自持鎖。
# queue 行格式: <dut> <rate> <cap|lat> <tag> <conf相對batch_alignment> [VAR=VAL ...]
# 已有 verdict.txt 的 run 目錄會跳過（可中斷後續跑）。# 開頭為註解。
set -uo pipefail
ALIGN_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNS_DIR="${RUNS_DIR:-$ALIGN_DIR/runs}"; export RUNS_DIR; mkdir -p "$RUNS_DIR" "$RUNS_DIR/../logs${RUNS_SUFFIX:-}"
QUEUE="${1:?usage: run_queue.sh <queue file>}"
exec 3< "$QUEUE"
while read -r -u 3 dut rate mode tag conf rest; do
    [[ -z "${dut:-}" || "$dut" == \#* ]] && continue
    out="$RUNS_DIR/$dut/${mode}_rate_${rate}_${tag}"
    if [[ -s "$out/verdict.txt" ]]; then echo "[queue] skip (done) $dut $rate $mode $tag"; continue; fi
    echo "[queue] $(date -Is) start $dut $rate $mode $tag"
    env CONF="$ALIGN_DIR/$conf" TAG="$tag" $rest bash "$ALIGN_DIR/run_point.sh" "$dut" "$rate" "$mode" < /dev/null
    echo "[queue] $(date -Is) end rc=$? :: $(head -1 "$out/verdict.txt" 2>/dev/null)"
    [[ -f "$ALIGN_DIR/STOP" ]] && { echo "[queue] STOP file found"; exit 0; }
done
