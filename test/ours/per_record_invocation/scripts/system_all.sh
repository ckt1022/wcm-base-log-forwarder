#!/bin/bash
# system_all.sh — Part B 系統層量測的執行順序（每個 trial / rep 單獨持有共用量測鎖）。
# 1. C3 正確性：k=1 三輪、k=unlimited 一輪（正式 wcm-b1 已有三輪 0/38,000 的結果）
# 2. 最大可持續 EPS 搜尋：kinf → k1 → k16 → k256
# 3. 5,000 EPS 延遲／資源：四個變體
# 較高速率的延遲點在搜尋結果出來後另行指定（見 reproduce.sh 的 HIGH_RATE）。
set -uo pipefail
S="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
Y="$S/../system/yaml"
STAGE="${1:-all}"

if [[ "$STAGE" == all || "$STAGE" == c3 ]]; then
    REPEATS=3 SCENARIOS=C3 "$S/correctness_percall.sh" "$Y/forwarder_membound_b1_k1.yaml" c3_k1
    REPEATS=1 SCENARIOS=C3 "$S/correctness_percall.sh" "$Y/forwarder_membound_b1_kinf.yaml" c3_kinf
fi
if [[ "$STAGE" == all || "$STAGE" == search ]]; then
    "$S/search_percall.sh" wcm-percall-kinf 12500
    "$S/search_percall.sh" wcm-percall-k1 3125
    "$S/search_percall.sh" wcm-percall-k16 12500
    "$S/search_percall.sh" wcm-percall-k256 12500
fi
if [[ "$STAGE" == all || "$STAGE" == lat5000 ]]; then
    for d in wcm-percall-kinf wcm-percall-k1 wcm-percall-k16 wcm-percall-k256; do
        "$S/latency_percall.sh" "$d" 5000
    done
fi
echo "[system_all] done stage=$STAGE"
