#!/bin/bash
# search.sh — 對單一 DUT 做最大可持續吞吐搜尋(粗掃 ×2 → 二分收斂到 ±5%)
#
# 用法: ./search.sh <dut> [start_rate]
#   dut: fluentd | fbit-lua | fbit-wasm | vector
# 結果: runs/<dut>/search_result.txt(每次試驗一行 + 最後 CAPACITY 行)
set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

DUT="${1:?usage: search.sh <dut> [start_rate]}"
R="${2:-2000}"
MAX_RATE=1024000

RUN_VARIANT="${RUN_VARIANT:-}"
RESULT_SUFFIX=""
[[ -n "$RUN_VARIANT" ]] && RESULT_SUFFIX="_${RUN_VARIANT}"
RESULT="$SCRIPT_DIR/runs/$DUT/search_result${RESULT_SUFFIX}.txt"
mkdir -p "$SCRIPT_DIR/runs/$DUT"
: > "$RESULT"

trial() {
    local rate=$1
    echo "[search] === $DUT @ $rate eps ==="
    if "$SCRIPT_DIR/capacity.sh" "$DUT" "$rate" cap; then
        echo "trial rate=$rate PASS" >> "$RESULT"
        return 0
    else
        echo "trial rate=$rate FAIL" >> "$RESULT"
        return 1
    fi
}

LAST_PASS=0
FIRST_FAIL=0

# 粗掃:×2 遞增
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

# 二分:收斂到 (FIRST_FAIL-LAST_PASS)/LAST_PASS ≤ 5%
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

# 容量點重跑一次確認
echo "[search] === 容量點確認重跑: $DUT @ $LAST_PASS ==="
if trial "$LAST_PASS"; then
    echo "CAPACITY dut=$DUT eps=$LAST_PASS first_fail=$FIRST_FAIL confirmed=yes" | tee -a "$RESULT"
else
    echo "CAPACITY dut=$DUT eps=$LAST_PASS first_fail=$FIRST_FAIL confirmed=NO(重跑失敗,需人工檢視)" | tee -a "$RESULT"
fi
