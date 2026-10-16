#!/bin/bash
# run_all_lua.sh — 一次跑完 Fluent Bit + Lua filter 的所有 flush time 延遲實驗
#
# 用法：
#   ./run_all_lua.sh              # 跑下方 FLUSH_TIMES 預設清單
#   ./run_all_lua.sh 0.1 1 5      # 只跑指定的 flush time
#
# 每輪流程（不動 latency_lua.sh 的邏輯，只代改參數再呼叫它）：
#   1. 改 conf/fluentbit_latency.conf 的 [SERVICE] Flush
#   2. 改 latency_lua.sh 的 LOG_DIR（fluentbit_lua_time_<flush>_500）
#   3. 執行 latency_lua.sh（一輪約 5~6 分鐘）
# 全部跑完後，重新輸出每個 flush time 的 30~230 秒穩態延遲統計。
# 注意：conf 與 latency_lua.sh 會停留在最後一輪的值。

set -uo pipefail

# ── 要跑的 flush time（秒）：想跑哪些就改這裡 ────────────────────────────────
FLUSH_TIMES=(0.1 0.5 1 2 5 10 20)
if (( $# > 0 )); then FLUSH_TIMES=("$@"); fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../../.." && pwd)"

CONF="$LAT_DIR/conf/fluentbit_latency.conf"
RUN_SH="$SCRIPT_DIR/latency_lua.sh"
CALC="$REPO_ROOT/test/compare/calc_latency.py"

FAILED=()

echo "[all] 將依序執行 ${#FLUSH_TIMES[@]} 輪實驗：flush = ${FLUSH_TIMES[*]}"
echo ""

for F in "${FLUSH_TIMES[@]}"; do
    RUN_NAME="fluentbit_lua_time_${F}_500"
    echo "[all] ════════════════ flush=${F}s（${RUN_NAME}）════════════════"

    # 1. conf 的 [SERVICE] Flush（conf 內只有 SERVICE 區塊有這個 key）
    sed -i -E "s|^( *Flush +).*|\1${F}|" "$CONF"
    # 2. latency_lua.sh 的 LOG_DIR 資料夾名稱
    sed -i -E "s|^LOG_DIR=.*|LOG_DIR=\"\$LAT_DIR/fluentbit_lua_time/${RUN_NAME}\"|" "$RUN_SH"

    grep -E "^ *Flush " "$CONF" | sed 's/^/[all]   conf  → /'
    grep -E "^LOG_DIR="  "$RUN_SH" | sed 's/^/[all]   sh    → /'

    # 3. 執行單輪實驗；失敗記下來，繼續跑下一輪
    if ! "$RUN_SH"; then
        echo "[all] WARNING: flush=${F}s 該輪失敗，跳過並繼續"
        FAILED+=("$F")
    fi

    # 讓 5140/8080 埠與背景程序完全釋放再進下一輪
    sleep 5
    echo ""
done

# ── 彙總：重新計算每個 flush time 的穩態延遲統計 ─────────────────────────────
echo ""
echo "[all] ══════════════════ 全部完成，彙總結果 ══════════════════"
for F in "${FLUSH_TIMES[@]}"; do
    RUN_NAME="fluentbit_lua_time_${F}_500"
    CSV_FILE="$LAT_DIR/fluentbit_lua_time/${RUN_NAME}/${RUN_NAME}.csv"
    echo ""
    echo "[all] ── flush=${F}s ──"
    if [[ -f "$CSV_FILE" ]]; then
        python3 "$CALC" "$CSV_FILE" 30 230
    else
        echo "[all]   （無結果：找不到 $CSV_FILE）"
    fi
done

# 產生彙整檔：fluentbit_lua_time/fluentbit_lua_time_summary.txt
echo ""
python3 "$REPO_ROOT/test/compare/make_summary.py" "$LAT_DIR/fluentbit_lua_time" \
    "docker fluent/fluent-bit:5.0.9（--network host）rate=500/s duration=300s Lua filter"

if (( ${#FAILED[@]} > 0 )); then
    echo ""
    echo "[all] 失敗的輪次：flush = ${FAILED[*]}"
    exit 1
fi
