#!/bin/bash
# run_all_vector.sh — 一次跑完 Vector + VRL remap 的所有 flush time 延遲實驗
# （對照 test/compare/fluentd/latency/run_all_fluentd.sh、
#   test/compare/fluentbit/latency/script/run_all_lua.sh,同清單、同輸出格式）
#
# 用法：
#   ./run_all_vector.sh            # 跑下方 FLUSH_TIMES 預設清單
#   ./run_all_vector.sh 0.1 1 5    # 只跑指定的 flush time
#
# 每輪直接以參數呼叫 latency.sh（它自己會把 FLUSH_INTERVAL 注入
# conf/latency.yaml 的 batch.timeout_secs、命名 vector_time_<flush>_500 資料夾，
# 不需要代改檔案），一輪約 5~6 分鐘。
# latency.sh 的 CSV 留在 latency_worker_1.csv,本腳本每輪結束改名為
# <資料夾名>.csv（對齊 fluentd/fluent-bit 慣例,make_summary.py 才找得到）。
# 全部跑完後，重新輸出每個 flush time 的 30~230 秒穩態延遲統計，並產生彙整檔。

set -uo pipefail

# ── 要跑的 flush time（秒）：想跑哪些就改這裡 ────────────────────────────────
FLUSH_TIMES=(0.1 0.5 1 2 5 10 20)
if (( $# > 0 )); then FLUSH_TIMES=("$@"); fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LAT_DIR="$SCRIPT_DIR"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../../.." && pwd)"

RUN_SH="$SCRIPT_DIR/latency.sh"
CALC="$REPO_ROOT/test/compare/calc_latency.py"

FAILED=()

echo "[all] 將依序執行 ${#FLUSH_TIMES[@]} 輪實驗：flush = ${FLUSH_TIMES[*]}"
echo ""

for F in "${FLUSH_TIMES[@]}"; do
    RUN_NAME="vector_time_${F}_500"
    RUN_DIR="$LAT_DIR/vector_time/${RUN_NAME}"
    echo "[all] ════════════════ flush=${F}s（${RUN_NAME}）════════════════"

    # 執行單輪實驗；失敗記下來，繼續跑下一輪
    if ! "$RUN_SH" "$F"; then
        echo "[all] WARNING: flush=${F}s 該輪失敗，跳過並繼續"
        FAILED+=("$F")
    fi

    # CSV 改名對齊 fluentd/fluent-bit 慣例（make_summary.py 需要 <資料夾名>.csv）
    if [[ -f "$RUN_DIR/latency_worker_1.csv" ]]; then
        mv "$RUN_DIR/latency_worker_1.csv" "$RUN_DIR/${RUN_NAME}.csv"
    fi

    # 讓 5140/8080 埠與背景程序完全釋放再進下一輪
    sleep 5
    echo ""
done

# ── 彙總：重新計算每個 flush time 的穩態延遲統計 ─────────────────────────────
echo ""
echo "[all] ══════════════════ 全部完成，彙總結果 ══════════════════"
for F in "${FLUSH_TIMES[@]}"; do
    RUN_NAME="vector_time_${F}_500"
    CSV_FILE="$LAT_DIR/vector_time/${RUN_NAME}/${RUN_NAME}.csv"
    echo ""
    echo "[all] ── flush=${F}s ──"
    if [[ -f "$CSV_FILE" ]]; then
        python3 "$CALC" "$CSV_FILE" 30 230
    else
        echo "[all]   （無結果：找不到 $CSV_FILE）"
    fi
done

# 產生彙整檔：vector_time/vector_time_summary.txt
echo ""
python3 "$REPO_ROOT/test/compare/make_summary.py" "$LAT_DIR/vector_time" \
    "docker timberio/vector:0.49.0-alpine（--network host）rate=500/s duration=300s VRL remap"

if (( ${#FAILED[@]} > 0 )); then
    echo ""
    echo "[all] 失敗的輪次：flush = ${FAILED[*]}"
    exit 1
fi
