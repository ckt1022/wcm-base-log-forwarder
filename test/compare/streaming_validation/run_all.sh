#!/bin/bash
# run_all.sh — streaming 模式驗證實驗（見 README.md）。
#
# 順序：0. preflight（docker 可用、image 存在；缺 streaming image 時在鎖內建置）
#       1. 正確性 C1–C4 × 3 reps（streaming image + wcm_stream_correctness_b1.yaml）
#       2. 方法 3 延遲 @5,000 EPS，streaming，W ∈ {0.01, 0.1, 0.5, 1}
#       3. 同 session 對照：舊模式 WCM（memguard + wcm_tw128_wait100）、FB Lua（flush 0.1），W=0.1 @5,000 EPS
#       4. 容量搜尋，streaming，W ∈ {0.01, 0.1, 0.5, 1}
#       5. DONE marker + 一行摘要
# 每個量測單位（一次完整 run，含 container 啟停）都在 flock $LOCK 內執行；
# 延遲 run 由 capacity_align.sh 全程記錄 clock_drift.csv。
# 已完成（verdict.txt 存在／search 已有 CAPACITY 行）的點會跳過，可直接重跑續傳。
# 步驟之間檢查 STOP 檔：touch test/compare/streaming_validation/STOP 即在下一個點之前結束。
set -uo pipefail
SV_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ALIGN_DIR="$(cd "$SV_DIR/../batch_alignment" && pwd)"
REPO_ROOT="$(cd "$SV_DIR/../../.." && pwd)"
LOCK="${LOCK:-/home/ckt1022/.wcm-bench.lock}"
export RUNS_DIR="$SV_DIR/runs"
LOG_DIR="$SV_DIR/logs"
mkdir -p "$RUNS_DIR" "$LOG_DIR"
STREAM_IMAGE="${STREAM_IMAGE:-wcm-base-log-forwarder:streaming}"
OLD_IMAGE="wcm-base-log-forwarder:memguard"
W_LIST="${W_LIST:-0.01 0.1 0.5 1}"
SEARCH_START="${SEARCH_START:-22000}"
DOCKER="${DOCKER:-docker}"

log() { echo "[$(date -Is)] $*"; }
check_stop() {
    if [[ -f "$SV_DIR/STOP" ]]; then
        log "STOP 檔存在，於步驟之間結束（刪除 STOP 後重跑即可續傳）"
        exit 0
    fi
}
done_verdict() { [[ -s "$1/verdict.txt" ]]; }

log "run_all.sh start pid=$$ stream_image=$STREAM_IMAGE"

# ── 0. preflight ─────────────────────────────────────────────────────────────
if ! $DOCKER info >/dev/null 2>&1; then
    log "ERROR: 無法連線 docker daemon（權限或 daemon 未啟動），中止"
    exit 1
fi
if ! $DOCKER image inspect "$STREAM_IMAGE" >/dev/null 2>&1; then
    log "streaming image 不存在，於鎖內建置 $STREAM_IMAGE"
    flock "$LOCK" $DOCKER build -t "$STREAM_IMAGE" "$REPO_ROOT" >> "$LOG_DIR/docker_build.out" 2>&1 \
        || { log "ERROR: docker build 失敗（見 logs/docker_build.out）"; exit 1; }
fi
for img in "$STREAM_IMAGE" "$OLD_IMAGE" fluent/fluent-bit:5.0.9; do
    echo "$(date -Is) $img $($DOCKER image inspect --format '{{.Id}}' "$img" 2>/dev/null || echo MISSING)"
done | tee -a "$LOG_DIR/images.txt"
check_stop

# ── 1. 正確性 C1–C4 × 3 ──────────────────────────────────────────────────────
CORR_ROOT="$RUNS_DIR/correctness/stream"
for sc in C1 C2 C3 C4; do
    for rep in 1 2 3; do
        check_stop
        out="$CORR_ROOT/${sc}_rep${rep}"
        if done_verdict "$out"; then log "skip correctness $sc rep$rep（已完成）"; continue; fi
        log "correctness $sc rep$rep"
        t0=$(date +%s)
        flock "$LOCK" env IMAGE="$STREAM_IMAGE" CONFIG="$SV_DIR/conf/wcm_stream_correctness_b1.yaml" \
            RUN_ID=stream RUN_ROOT="$CORR_ROOT" SCENARIOS="$sc" REPS_LIST="$rep" SKIP_BUILD=1 \
            bash "$SV_DIR/run_wcm_correctness_stream.sh" < /dev/null >> "$LOG_DIR/correctness.out" 2>&1
        rc=$?
        echo "$(date -Is) rc=$rc dur=$(( $(date +%s) - t0 ))s correctness $sc rep$rep :: $(head -1 "$out/verdict.txt" 2>/dev/null)" >> "$LOG_DIR/runs.log"
    done
done
python3 "$REPO_ROOT/test/compare/correctness/summarize.py" "$CORR_ROOT" > "$LOG_DIR/correctness_summary.txt" 2>&1 || true

# ── 2. 方法 3 延遲 @5,000 EPS（streaming）────────────────────────────────────
lat_point() {  # <dut> <tag> <conf> [VAR=VAL ...]
    local dut=$1 tag=$2 conf=$3; shift 3
    local out="$RUNS_DIR/$dut/lat_rate_5000_$tag"
    check_stop
    if done_verdict "$out"; then log "skip lat $dut $tag（已完成）"; return; fi
    log "lat $dut 5000 $tag conf=$(basename "$conf")"
    env CONF="$conf" TAG="$tag" "$@" bash "$SV_DIR/run_point.sh" "$dut" 5000 lat < /dev/null >> "$LOG_DIR/lat.out" 2>&1
    log "  -> $(head -1 "$out/verdict.txt" 2>/dev/null) | $(grep -h 'p99 ' "$out/latency_stats_30_150.txt" 2>/dev/null | tr -s ' ')"
}
for W in $W_LIST; do
    lat_point wcm "stream_W$W" "$SV_DIR/conf/wcm_stream_W$W.yaml" WCM_IMAGE="$STREAM_IMAGE"
done

# ── 3. 同 session 對照（W=0.1）──────────────────────────────────────────────
lat_point wcm ctrl_batch_W0.1 "$ALIGN_DIR/conf/wcm_tw128_wait100.yaml" WCM_IMAGE="$OLD_IMAGE"
lat_point fbit-lua ctrl_W0.1 "$ALIGN_DIR/conf/fbit_lua_flush_0.1.conf"

# ── 4. 容量搜尋（streaming）──────────────────────────────────────────────────
for W in $W_LIST; do
    check_stop
    res="$RUNS_DIR/wcm/search_stream_W$W.txt"
    if grep -q '^CAPACITY' "$res" 2>/dev/null; then log "skip search W=$W（已完成）"; continue; fi
    log "capacity search W=$W start=$SEARCH_START"
    # search_align.sh 每個 trial 都是一個 run_point.sh（各自持鎖）；trial 之間也檢查 STOP（本目錄的複本）
    env WCM_IMAGE="$STREAM_IMAGE" bash "$SV_DIR/search_align.sh" wcm "stream_W$W" "conf/wcm_stream_W$W.yaml" "$SEARCH_START" < /dev/null >> "$LOG_DIR/search.out" 2>&1
    log "  -> $(grep '^CAPACITY' "$res" 2>/dev/null | tail -1)"
done

# ── 5. DONE ──────────────────────────────────────────────────────────────────
summary="DONE $(date -Is) correctness_pass=$(cat "$CORR_ROOT"/*/verdict.txt 2>/dev/null | grep -c '^PASS')/12"
for W in $W_LIST; do
    p99=$(grep -h 'p99 ' "$RUNS_DIR/wcm/lat_rate_5000_stream_W$W/latency_stats_30_150.txt" 2>/dev/null | awk '{print $(NF-1)}')
    cap=$(grep -h '^CAPACITY' "$RUNS_DIR/wcm/search_stream_W$W.txt" 2>/dev/null | tail -1 | sed 's/.*eps=\([^ ]*\).*/\1/')
    summary+=" W$W:p99=${p99:-?}ms,cap=${cap:-?}"
done
for c in wcm/lat_rate_5000_ctrl_batch_W0.1 fbit-lua/lat_rate_5000_ctrl_W0.1; do
    summary+=" ${c%%/*}_ctrl:p99=$(grep -h 'p99 ' "$RUNS_DIR/$c/latency_stats_30_150.txt" 2>/dev/null | awk '{print $(NF-1)}')ms"
done
echo "$summary" > "$SV_DIR/DONE"
log "$summary"
