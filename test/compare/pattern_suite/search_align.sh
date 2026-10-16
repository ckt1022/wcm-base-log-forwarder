#!/bin/bash
# [pattern_suite] streaming_validation/search_align.sh 的複本（呼叫本目錄 run_point.sh；trial 之間檢查 pattern_suite/STOP）。
# [streaming_validation] batch_alignment/search_align.sh 的複本；conf 路徑改為「絕對路徑或相對 streaming_validation」，
# 呼叫本目錄 run_point.sh。
# search_align.sh — 粗略容量搜尋（約 5% 解析度），PASS 判定與正式容量實驗相同
# （capacity.sh cap 模式：60 s 負載、20 s drain、eval_trial.py C1–C4）。
#
# 用法: search_align.sh <dut> <tag> <conf相對batch_alignment> <start_rate> [VAR=VAL ...]
# 策略：從 start_rate 出發；PASS 就 ×1.10 往上、FAIL 就 ×0.88 往下，直到夾住
# (last_pass, first_fail)，再二分到 (first_fail-last_pass)/last_pass ≤ 5%。
# 每個 trial = 一個 run_point.sh（各自持共享鎖）。不做確認重跑（單次結果）。
# 結果: runs/<dut>/search_<tag>.txt
set -uo pipefail
SV_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNS_DIR="${RUNS_DIR:-$SV_DIR/runs}"; export RUNS_DIR; mkdir -p "$RUNS_DIR" "$RUNS_DIR/../logs${RUNS_SUFFIX:-}"
DUT="$1"; TAG="$2"; CONF="$3"; [[ "$CONF" = /* ]] || CONF="$SV_DIR/$CONF"; START="$4"; shift 4
EXTRA=("$@")
RESULT="$RUNS_DIR/$DUT/search_${TAG}.txt"
mkdir -p "$RUNS_DIR/$DUT"

rnd() {  # 依量級取整，避免奇怪的速率值
    local r=$1
    if   (( r < 10000 )); then echo $(( (r + 50) / 100 * 100 ))
    elif (( r < 30000 )); then echo $(( (r + 125) / 250 * 250 ))
    else echo $(( (r + 250) / 500 * 500 )); fi
}

trial() {
    local rate=$1
    # [streaming_validation] trial 之間檢查 STOP 檔
    if [[ -f "$SV_DIR/STOP" ]]; then echo "# STOP $(date -Is)" >> "$RESULT"; exit 3; fi
    local out="$RUNS_DIR/$DUT/cap_rate_${rate}_${TAG}"
    if [[ ! -s "$out/verdict.txt" ]]; then
        env CONF="$CONF" TAG="$TAG" "${EXTRA[@]}" bash "$SV_DIR/run_point.sh" "$DUT" "$rate" cap < /dev/null >> "$RUNS_DIR/../logs${RUNS_SUFFIX:-}/search_${DUT}_${TAG}.out" 2>&1
    fi
    local v; v=$(head -1 "$out/verdict.txt" 2>/dev/null)
    echo "trial rate=$rate ${v%% *} :: $v" >> "$RESULT"
    [[ "$v" == PASS* ]]
}

[[ -f "$RESULT" ]] && echo "# --- resumed $(date -Is)" >> "$RESULT"
LAST_PASS=0; FIRST_FAIL=0; R=$(rnd "$START")
if trial "$R"; then
    LAST_PASS=$R
    for _ in 1 2 3 4 5 6; do
        R=$(rnd $(( R * 110 / 100 )))
        if trial "$R"; then LAST_PASS=$R; else FIRST_FAIL=$R; break; fi
    done
else
    FIRST_FAIL=$R
    for _ in 1 2 3 4 5 6 7 8 9 10; do
        R=$(rnd $(( R * 88 / 100 )))
        if trial "$R"; then LAST_PASS=$R; break; else FIRST_FAIL=$R; fi
    done
fi
if (( LAST_PASS == 0 || FIRST_FAIL == 0 )); then
    echo "CAPACITY dut=$DUT tag=$TAG eps=unbracketed last_pass=$LAST_PASS first_fail=$FIRST_FAIL" | tee -a "$RESULT"
    exit 1
fi
while (( (FIRST_FAIL - LAST_PASS) * 20 > LAST_PASS )); do
    MID=$(rnd $(( (LAST_PASS + FIRST_FAIL) / 2 )))
    (( MID <= LAST_PASS || MID >= FIRST_FAIL )) && break
    if trial "$MID"; then LAST_PASS=$MID; else FIRST_FAIL=$MID; fi
done
echo "CAPACITY dut=$DUT tag=$TAG eps=$LAST_PASS first_fail=$FIRST_FAIL" | tee -a "$RESULT"
