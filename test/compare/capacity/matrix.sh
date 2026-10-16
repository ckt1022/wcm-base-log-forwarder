#!/bin/bash
# matrix.sh — 固定變因矩陣實驗:五系統在共同絕對吞吐量下,各自調到最小可用批次窗
#
# 固定變因(全系統相同,不可調):
#   concurrency=1(fluentd flush_thread_count/fluent-bit Workers/
#                 vector request.concurrency/wcm transport_workers 皆已是 1)
#   retry 全部關閉、timeout 30s(見 *_tuned.conf/yaml)
#   硬體 1 core/512MB/無 swap、輸出目的地 127.0.0.1:8080/ingest(不變)
#   arrival trace:gen json-fixed5(內容非隨機)+ -ts-mode sched(排程時間戳)
#   負載 90s、穩態窗 20~80s
#
# 自由變因(各系統自行調,以下為策略):
#   fluentd    : 無 fallback(flush_thread_interval 寫死 0.1,無法更低)
#   fbit-lua/wasm/vector : 從 0.01 起,失敗則階梯放大 flush 直到 PASS
#   wcm        : 先試 max_wait_ms=10(buffer=128MB);失敗則退到 buffer=8MB/
#                max_wait_ms=1000(已知 132k 前皆可行的安全設定)
#
# 吞吐量點(輸入 eps,超過該系統 flush=1s 已知容量者直接標 N/A,不測):
#   10000 20000 25000 50000 100000 125000 150000
set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
COMPARE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"

export SKIP_CLOCK_CHECK="${SKIP_CLOCK_CHECK:-0}"
export LOAD_SECS=90
export DRAIN_SECS=10
CALC="$SCRIPT_DIR/calc_latency_stream.py"

RESULT="$SCRIPT_DIR/runs/matrix_result.txt"
: > "$RESULT"

log() { echo "[matrix] $*"; }

# 回傳 latency_worker_1.csv 的 p50/p99(20~80s 窗),失敗回空字串
calc_stats() {
    local csv="$1"
    [[ -f "$csv" ]] || { echo ""; return; }
    python3 "$CALC" "$csv" 20 80 2>/dev/null | awk '
        /p50 / {p50=$4} /p99 / && !/p99\./ {p99=$4}
        END{printf "%s %s", p50, p99}'
}

# ── Fluent Bit(Lua/WASM 共用):flush 階梯 0.01→0.05→0.1→0.5→1 ─────────────
run_fbit() {
    local dut="$1" conf="$2" rate="$3"
    for f in 0.01 0.05 0.1 0.5 1; do
        log "=== $dut @ $rate  flush=$f ==="
        if CONF_OVERRIDE="$conf" FLUSH_INTERVAL="$f" "$SCRIPT_DIR/capacity.sh" "$dut" "$rate" lat \
            > "$SCRIPT_DIR/runs/${dut}_${rate}_f${f}.log" 2>&1; then
            local stats
            stats=$(calc_stats "$SCRIPT_DIR/runs/$dut/lat_rate_$rate/latency_worker_1.csv")
            echo "MATRIX dut=$dut rate=$rate flush=$f status=PASS p50=${stats%% *} p99=${stats##* }" | tee -a "$RESULT"
            return 0
        fi
        echo "MATRIX dut=$dut rate=$rate flush=$f status=FAIL" | tee -a "$RESULT"
    done
    echo "MATRIX dut=$dut rate=$rate status=EXCEEDS_CAPACITY" | tee -a "$RESULT"
}

# ── Vector:flush 階梯同上,conf 走 latency_tuned.yaml ──────────────────────
run_vector() {
    local rate="$1"
    local conf="$COMPARE_DIR/vector/latency/conf/latency_tuned.yaml"
    for f in 0.01 0.05 0.1 0.5 1; do
        log "=== vector @ $rate  flush=$f ==="
        if CONF_OVERRIDE="$conf" FLUSH_INTERVAL="$f" "$SCRIPT_DIR/capacity.sh" vector "$rate" lat \
            > "$SCRIPT_DIR/runs/vector_${rate}_f${f}.log" 2>&1; then
            local stats
            stats=$(calc_stats "$SCRIPT_DIR/runs/vector/lat_rate_$rate/latency_worker_1.csv")
            echo "MATRIX dut=vector rate=$rate flush=$f status=PASS p50=${stats%% *} p99=${stats##* }" | tee -a "$RESULT"
            return 0
        fi
        echo "MATRIX dut=vector rate=$rate flush=$f status=FAIL" | tee -a "$RESULT"
    done
    echo "MATRIX dut=vector rate=$rate status=EXCEEDS_CAPACITY" | tee -a "$RESULT"
}

# ── Fluentd:無 fallback,flush_thread_interval 寫死 0.1 ───────────────────
run_fluentd() {
    local rate="$1"
    local conf="$COMPARE_DIR/fluentd/latency/conf/latency_tuned.conf"
    log "=== fluentd @ $rate  flush=0.1(floor) ==="
    if CONF_OVERRIDE="$conf" FLUSH_INTERVAL="0.1" "$SCRIPT_DIR/capacity.sh" fluentd "$rate" lat \
        > "$SCRIPT_DIR/runs/fluentd_${rate}.log" 2>&1; then
        local stats
        stats=$(calc_stats "$SCRIPT_DIR/runs/fluentd/lat_rate_$rate/latency_worker_1.csv")
        echo "MATRIX dut=fluentd rate=$rate flush=0.1 status=PASS p50=${stats%% *} p99=${stats##* }" | tee -a "$RESULT"
    else
        echo "MATRIX dut=fluentd rate=$rate flush=0.1 status=EXCEEDS_CAPACITY" | tee -a "$RESULT"
    fi
}

# ── WCM:先試 max_wait_ms=10(buffer=128MB),失敗退回 buffer=8MB/max_wait=1000 ──
run_wcm() {
    local rate="$1"
    log "=== wcm-flush10ms @ $rate  max_wait_ms=10 ==="
    if MEM_GUARD_HIGH=0.60 MEM_GUARD_LOW=0.45 "$SCRIPT_DIR/capacity.sh" wcm-flush10ms "$rate" lat \
        > "$SCRIPT_DIR/runs/wcm_${rate}_flush10ms.log" 2>&1; then
        local stats
        stats=$(calc_stats "$SCRIPT_DIR/runs/wcm-flush10ms/lat_rate_$rate/latency_worker_1.csv")
        echo "MATRIX dut=wcm rate=$rate knob=max_wait_ms=10 status=PASS p50=${stats%% *} p99=${stats##* }" | tee -a "$RESULT"
        return 0
    fi
    echo "MATRIX dut=wcm rate=$rate knob=max_wait_ms=10 status=FAIL" | tee -a "$RESULT"
    log "=== wcm-v2(fallback) @ $rate  buffer=8MB/max_wait=1000 ==="
    if MEM_GUARD_HIGH=0.60 MEM_GUARD_LOW=0.45 "$SCRIPT_DIR/capacity.sh" wcm-v2 "$rate" lat \
        > "$SCRIPT_DIR/runs/wcm_${rate}_v2fallback.log" 2>&1; then
        local stats
        stats=$(calc_stats "$SCRIPT_DIR/runs/wcm-v2/lat_rate_$rate/latency_worker_1.csv")
        echo "MATRIX dut=wcm rate=$rate knob=buffer8MB/max_wait1000 status=PASS p50=${stats%% *} p99=${stats##* }" | tee -a "$RESULT"
        return 0
    fi
    echo "MATRIX dut=wcm rate=$rate status=EXCEEDS_CAPACITY" | tee -a "$RESULT"
}

# ── 吞吐量點(輸入 eps)× 各系統有效範圍(依 flush=1s 已知容量預篩) ──────────
POINTS="10000 20000 25000 50000 100000 125000 150000"

FB_LUA_CONF="$COMPARE_DIR/fluentbit/latency/conf/fluentbit_latency_tuned.conf"
FB_WASM_CONF="$COMPARE_DIR/fluentbit/latency/conf/fluentbit_latency_wasm_tuned.conf"

for R in $POINTS; do
    log "═══════════════ 吞吐量點 $R eps ═══════════════"

    # Fluentd(容量 176,000):全部 7 點
    run_fluentd "$R"

    # Fluent Bit Lua(容量 296,000):全部 7 點
    run_fbit fbit-lua "$FB_LUA_CONF" "$R"

    # WCM(容量 ~132,000):到 125,000
    if (( R <= 125000 )); then
        run_wcm "$R"
    else
        echo "MATRIX dut=wcm rate=$R status=NA_EXCEEDS_KNOWN_CAPACITY" | tee -a "$RESULT"
    fi

    # Vector(容量 62,000):到 50,000
    if (( R <= 50000 )); then
        run_vector "$R"
    else
        echo "MATRIX dut=vector rate=$R status=NA_EXCEEDS_KNOWN_CAPACITY" | tee -a "$RESULT"
    fi

    # Fluent Bit WASM(容量 24,000):到 20,000
    if (( R <= 20000 )); then
        run_fbit fbit-wasm "$FB_WASM_CONF" "$R"
    else
        echo "MATRIX dut=fbit-wasm rate=$R status=NA_EXCEEDS_KNOWN_CAPACITY" | tee -a "$RESULT"
    fi
done

log "全部完成,結果見 $RESULT"
