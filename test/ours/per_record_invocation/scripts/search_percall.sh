#!/bin/bash
# search_percall.sh — test/compare/capacity/search.sh 的複本（演算法逐字相同：
# 粗掃 ×2 → 二分收斂到 ±5% → 容量點確認重跑），差異：
#   - 每個 trial 呼叫 capacity_percall.sh，且「每個 trial 單獨持有」共用量測鎖
#     /home/ckt1022/.wcm-bench.lock（讓其他 agent 可以在 trial 之間插隊）
#   - 每個 trial 用唯一 RUN_VARIANT（search_t<N>），不覆寫任何既有結果
#   - 固定 GEN_MODE=json-fields1k（1024 B / 36 欄位，與正式五產品容量點相同）
#
# 用法: search_percall.sh <dut> [start_rate]
set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
STUDY_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

DUT="${1:?usage: search_percall.sh <dut> [start_rate]}"
R="${2:-2000}"
MAX_RATE=1024000
export GEN_MODE="${GEN_MODE:-json-fields1k}"
export SKIP_CLOCK_CHECK="${SKIP_CLOCK_CHECK:-1}"

RESULT="$STUDY_DIR/system/runs/$DUT/search_result.txt"
mkdir -p "$STUDY_DIR/system/runs/$DUT"
[[ -s "$RESULT" ]] && { echo "existing $RESULT; refusing to overwrite" >&2; exit 2; }
: > "$RESULT"
TRIAL_N=0

trial() {
    local rate=$1
    TRIAL_N=$(( TRIAL_N + 1 ))
    echo "[search] === $DUT @ $rate eps (trial $TRIAL_N) ==="
    if RUN_VARIANT="search_t${TRIAL_N}" flock /home/ckt1022/.wcm-bench.lock \
        "$SCRIPT_DIR/capacity_percall.sh" "$DUT" "$rate" cap; then
        echo "trial rate=$rate PASS t=$TRIAL_N" >> "$RESULT"
        return 0
    else
        echo "trial rate=$rate FAIL t=$TRIAL_N" >> "$RESULT"
        return 1
    fi
}

LAST_PASS=0
FIRST_FAIL=0

while (( R <= MAX_RATE )); do
    if trial "$R"; then
        LAST_PASS=$R
        R=$(( R * 2 ))
    else
        FIRST_FAIL=$R
        break
    fi
done

if (( LAST_PASS == 0 )); then
    echo "CAPACITY dut=$DUT eps=<$FIRST_FAIL (起始速率即失敗)" | tee -a "$RESULT"
    exit 1
fi
if (( FIRST_FAIL == 0 )); then
    echo "CAPACITY dut=$DUT eps=>=$LAST_PASS (達搜尋上限仍 PASS)" | tee -a "$RESULT"
    exit 0
fi

while (( (FIRST_FAIL - LAST_PASS) * 20 > LAST_PASS )); do
    MID=$(( (LAST_PASS + FIRST_FAIL) / 2 ))
    MID=$(( MID / 100 * 100 ))
    (( MID <= LAST_PASS )) && break
    if trial "$MID"; then
        LAST_PASS=$MID
    else
        FIRST_FAIL=$MID
    fi
done

echo "[search] === 容量點確認重跑: $DUT @ $LAST_PASS ==="
if trial "$LAST_PASS"; then
    echo "CAPACITY dut=$DUT eps=$LAST_PASS first_fail=$FIRST_FAIL confirmed=yes" | tee -a "$RESULT"
else
    echo "CAPACITY dut=$DUT eps=$LAST_PASS first_fail=$FIRST_FAIL confirmed=NO(重跑失敗,需人工檢視)" | tee -a "$RESULT"
fi
