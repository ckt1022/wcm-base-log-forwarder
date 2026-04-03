#!/usr/bin/env bash
# run.sh — 對 wcm-base-log-forwarder 執行多組 benchmark 並輸出 CSV
#
# 使用方法:
#   cd <project-root>
#   bash tools/bench/run.sh
#
# 輸出:
#   tools/bench/results/results_<timestamp>.csv
#   tools/bench/results/<系統>_<rate>_<run>.txt  (原始 /usr/bin/time 輸出)

set -euo pipefail

# ── 設定 ─────────────────────────────────────────────────────────
PROJECT_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BINARY="$PROJECT_ROOT/target/release/wcm-base-log-forwarder"
GEN="$PROJECT_ROOT/tools/gen/main.go"
RESULTS_DIR="$PROJECT_ROOT/tools/bench/results"

RATES=(1000 5000 10000 20000)   # logs/sec
MODES=("simple" "complex")      # 格式複雜度
RUNS=2                          # 每組重複次數
DURATION=12                     # 每次跑幾秒
WARMUP=2                        # warmup 秒數（不計入）

mkdir -p "$RESULTS_DIR"
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
CSV="$RESULTS_DIR/results_${TIMESTAMP}.csv"

# ── 前置檢查 ──────────────────────────────────────────────────────
if ! command -v go &>/dev/null; then
    echo "[error] go not found" >&2; exit 1
fi
if [[ ! -f "$BINARY" ]]; then
    echo "[info] binary not found, building release..."
    cargo build --release --manifest-path "$PROJECT_ROOT/Cargo.toml"
fi
if ! /usr/bin/time --version &>/dev/null 2>&1; then
    echo "[error] /usr/bin/time not found (need GNU time for -v flag)" >&2; exit 1
fi

# ── CSV header ───────────────────────────────────────────────────
echo "system,mode,rate_target,run,duration_s,lines_processed,avg_rate,peak_rss_kb,wall_clock_s,user_cpu_s,sys_cpu_s" > "$CSV"
echo "[bench] results → $CSV"
echo "[bench] rates=${RATES[*]} modes=${MODES[*]} runs=$RUNS duration=${DURATION}s"
echo ""

# ── 解析 /usr/bin/time -v 輸出的 helper ──────────────────────────
# 從 raw txt 抽出 peak RSS (KB) 和 wall clock
parse_time_output() {
    local file="$1"
    peak_rss=$(grep "Maximum resident set size" "$file" | awk '{print $NF}')
    wall_s=$(grep "Elapsed (wall clock) time" "$file" | awk '{print $NF}' | awk -F: '{
        if (NF==3) printf "%.3f", ($1*3600)+($2*60)+$3
        else printf "%.3f", ($1*60)+$2
    }')
    user_s=$(grep "User time (seconds)" "$file" | awk '{print $NF}')
    sys_s=$(grep "System time (seconds)" "$file" | awk '{print $NF}')
}

# 從 forwarder 的 stderr 抽出 total lines processed
parse_forwarder_output() {
    local file="$1"
    total_lines=$(grep "total_lines_processed=" "$file" | tail -1 | sed 's/.*total_lines_processed=//')
    total_lines=${total_lines:-0}
}

# ── 主迴圈 ───────────────────────────────────────────────────────
for MODE in "${MODES[@]}"; do
    for RATE in "${RATES[@]}"; do
        for RUN in $(seq 1 "$RUNS"); do
            TAG="${MODE}_${RATE}_${RUN}"
            RAW_FILE="$RESULTS_DIR/wcm_${TAG}.txt"
            FWD_FILE="$RESULTS_DIR/wcm_${TAG}_fwd.txt"

            echo -n "[bench] wcm mode=$MODE rate=$RATE run=$RUN/$RUNS ... "

            # warmup（丟棄輸出）
            go run "$GEN" -rate "$RATE" -duration "$WARMUP" -mode "$MODE" 2>/dev/null \
                | "$BINARY" > /dev/null 2>/dev/null || true

            # 正式跑：forwarder stderr → fwd file，time stderr → raw file
            /usr/bin/time -v bash -c "
                go run '$GEN' -rate '$RATE' -duration '$DURATION' -mode '$MODE' 2>/dev/null \
                    | '$BINARY' > '$FWD_FILE' 2>>'$FWD_FILE'
            " 2>"$RAW_FILE"

            parse_time_output "$RAW_FILE"
            parse_forwarder_output "$FWD_FILE"

            avg_rate=0
            if (( $(echo "$wall_s > 0" | bc -l) )); then
                avg_rate=$(echo "scale=0; $total_lines / $wall_s" | bc 2>/dev/null || echo 0)
            fi

            echo "$total_lines lines, ${avg_rate}/s, RSS=${peak_rss}KB"

            echo "wcm,$MODE,$RATE,$RUN,$DURATION,$total_lines,$avg_rate,$peak_rss,$wall_s,$user_s,$sys_s" >> "$CSV"

            sleep 3  # 冷卻
        done
    done
done

echo ""
echo "[bench] done → $CSV"
