#!/bin/bash
# run_suite.sh — 設計模式論文的整夜實驗套件（見 README.md）。
#
#   Phase 0  同 session 容量檢查 + BEST_K 決策（runs/phase0_decision.txt）
#   Phase 1  模式實驗：P1 Poison Record Bisection、P2 Batch the Crossing、
#            P4 Watchdog + Disposable Instance、P5 Capability-Scoped World
#   Phase 2  五產品方法 3（FB Lua／Fluentd／Vector／FB WASM／WCM-batch／WCM-stream）
#
# 每個量測單位 = 一次完整 run（含 container／process 啟停），各自在 flock $LOCK 內執行
# （run_point.sh、run_fault_case.sh、loadcheck.sh 內部持鎖；correctness 由本腳本以 flock 包住）。
# 延遲 run 由 capacity_align.sh 全程記錄 clock_drift.csv 並以 correct_latency.py 校正；
# fault run 由 run_fault_case.sh 記錄 clock_drift.csv。
# 可續傳：verdict.txt／CAPACITY 行／host_status.txt／refusal.txt／#DONE 已存在的單位會跳過。
# 單位之間檢查 STOP 檔（touch test/compare/pattern_suite/STOP）。結束時寫 DONE。
#
# 測試模式：
#   DRYRUN=1  不跑任何量測，每個單位只寫假結果（runs_dry/），用來檢查整個控制流程與單位數量。
#   SMOKE=1   每種單位各跑一個縮短版的真實量測（runs_smoke/），然後結束。
set -uo pipefail
PS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$PS_DIR/../../.." && pwd)"
ALIGN_DIR="$REPO_ROOT/test/compare/batch_alignment"
FI_DIR="$REPO_ROOT/test/ours/fault_isolation"
LOCK="${LOCK:-/home/ckt1022/.wcm-bench.lock}"
IMAGE="${SUITE_IMAGE:-wcm-base-log-forwarder:suite}"
BIN="$REPO_ROOT/target/release/wcm-base-log-forwarder"
DOCKER="${DOCKER:-docker}"
DRYRUN="${DRYRUN:-0}"
SMOKE="${SMOKE:-0}"
if [[ "$DRYRUN" == 1 ]]; then RUNS_DIR="$PS_DIR/runs_dry"; LOG_DIR="$PS_DIR/runs_dry/logs"
elif [[ "$SMOKE" == 1 ]]; then RUNS_DIR="$PS_DIR/runs_smoke"; LOG_DIR="$PS_DIR/runs_smoke/logs"
else RUNS_DIR="$PS_DIR/runs"; LOG_DIR="$PS_DIR/logs"; fi
export RUNS_DIR
mkdir -p "$RUNS_DIR" "$LOG_DIR"
# 縮短量測時間用（SMOKE 模式）；正式執行為空
UNIT_ENV=()
W_LIST="0.01 0.1 0.5 1"
UNITS_RUN=0; UNITS_SKIP=0

log() { echo "[$(date -Is)] $*"; }
check_stop() {
    if [[ -f "$PS_DIR/STOP" ]]; then
        log "STOP 檔存在，於單位之間結束（rm STOP 後重跑即可續傳）"
        exit 0
    fi
}
num_or_zero() { [[ "$1" =~ ^[0-9]+$ ]] && echo "$1" || echo 0; }

# ── 單位：延遲（run_point.sh lat）────────────────────────────────────────────
# lat_unit <dut> <rate> <tag> <conf> [VAR=VAL ...]
lat_unit() {
    local dut=$1 rate=$2 tag=$3 conf=$4; shift 4
    [[ "$conf" = /* ]] || conf="$PS_DIR/$conf"
    local out="$RUNS_DIR/$dut/lat_rate_${rate}_$tag"
    check_stop
    if [[ -s "$out/verdict.txt" ]]; then UNITS_SKIP=$((UNITS_SKIP+1)); return 0; fi
    UNITS_RUN=$((UNITS_RUN+1))
    log "lat $dut $rate $tag conf=$(basename "$conf") $*"
    if [[ "$DRYRUN" == 1 ]]; then
        mkdir -p "$out"; echo "PASS rate=$rate (dry)" > "$out/verdict.txt"
        echo "[calc]   p99    =    123.456 ms" > "$out/latency_stats_30_150.txt"
        echo "[stats] window=30-150s(mono) samples=1 cpu_mean=10.0% cpu_max=11.0% mem_mean=10.0MiB mem_max=11.0MiB" > "$out/resource_stats_30_150.txt"
        return 0
    fi
    env "${UNIT_ENV[@]}" CONF="$conf" TAG="$tag" WCM_IMAGE="$IMAGE" "$@" \
        bash "$PS_DIR/run_point.sh" "$dut" "$rate" lat < /dev/null >> "$LOG_DIR/lat.out" 2>&1
    log "  -> $(head -1 "$out/verdict.txt" 2>/dev/null | cut -c1-60) | $(grep -h 'p99 ' "$out/latency_stats_30_150.txt" 2>/dev/null | tr -s ' ' | sed 's/\[calc\] //') | $(grep -oE 'cpu_mean=[0-9.]+%' "$out/resource_stats_30_150.txt" 2>/dev/null)"
}
lat_verdict() { head -1 "$RUNS_DIR/$1/lat_rate_${2}_$3/verdict.txt" 2>/dev/null | awk '{print $1}'; }
lat_p99() { grep -h 'p99 ' "$RUNS_DIR/$1/lat_rate_${2}_$3/latency_stats_30_150.txt" 2>/dev/null | awk '{print $(NF-1)}'; }
lat_cpu() { grep -oE 'cpu_mean=[0-9.]+' "$RUNS_DIR/$1/lat_rate_${2}_$3/resource_stats_30_150.txt" 2>/dev/null | cut -d= -f2; }

# ── 單位：容量搜尋（search_align.sh；每個 trial 一個 run_point.sh）───────────
# search_unit <dut> <tag> <conf> <start> [VAR=VAL ...]
search_unit() {
    local dut=$1 tag=$2 conf=$3 start=$4; shift 4
    [[ "$conf" = /* ]] || conf="$PS_DIR/$conf"
    local res="$RUNS_DIR/$dut/search_$tag.txt"
    check_stop
    if grep -q '^CAPACITY' "$res" 2>/dev/null; then UNITS_SKIP=$((UNITS_SKIP+1)); return 0; fi
    UNITS_RUN=$((UNITS_RUN+1))
    log "search $dut $tag start=$start conf=$(basename "$conf") $*"
    if [[ "$DRYRUN" == 1 ]]; then
        mkdir -p "$RUNS_DIR/$dut"; echo "CAPACITY dut=$dut tag=$tag eps=$start first_fail=$((start*105/100)) (dry)" > "$res"
        return 0
    fi
    env "${UNIT_ENV[@]}" WCM_IMAGE="$IMAGE" bash "$PS_DIR/search_align.sh" "$dut" "$tag" "$conf" "$start" "$@" \
        < /dev/null >> "$LOG_DIR/search.out" 2>&1
    local rc=$?
    (( rc == 3 )) && check_stop
    log "  -> $(grep '^CAPACITY' "$res" 2>/dev/null | tail -1)"
}
# cap_of <dut> <tag>：容量（EPS）；unbracketed 時取 last_pass；沒有結果時 0
cap_of() {
    local line; line=$(grep '^CAPACITY' "$RUNS_DIR/$1/search_$2.txt" 2>/dev/null | tail -1)
    local eps; eps=$(sed -n 's/.* eps=\([^ ]*\).*/\1/p' <<< "$line")
    if [[ "$eps" =~ ^[0-9]+$ ]]; then echo "$eps"; return; fi
    num_or_zero "$(sed -n 's/.* last_pass=\([0-9]*\).*/\1/p' <<< "$line")"
}

# ── 單位：正確性（C3 + malformed rate）───────────────────────────────────────
# corr_unit <label> <conf> <rate_pct> <rep>
corr_unit() {
    local label=$1 conf=$2 mr=$3 rep=$4
    [[ "$conf" = /* ]] || conf="$PS_DIR/$conf"
    local root="$RUNS_DIR/correctness/p1_${label}_mr$mr"
    local out="$root/C3_rep$rep"
    check_stop
    if [[ -s "$out/verdict.txt" ]]; then UNITS_SKIP=$((UNITS_SKIP+1)); return 0; fi
    UNITS_RUN=$((UNITS_RUN+1))
    log "correctness $label malformed=$mr% rep$rep conf=$(basename "$conf")"
    if [[ "$DRYRUN" == 1 ]]; then mkdir -p "$out"; echo "PASS (dry)" > "$out/verdict.txt"; return 0; fi
    flock "$LOCK" env "${UNIT_ENV[@]}" IMAGE="$IMAGE" CONFIG="$conf" RUN_ID="p1_${label}_mr$mr" RUN_ROOT="$root" \
        SCENARIOS=C3 REPS_LIST="$rep" SKIP_BUILD=1 MALFORMED_RATE="$mr" \
        bash "$PS_DIR/run_correctness.sh" < /dev/null >> "$LOG_DIR/correctness.out" 2>&1
    log "  -> $(head -1 "$out/verdict.txt" 2>/dev/null | cut -c1-90) | isolate_lines=$(grep -c '\[parse-isolate\]' "$out/dut.log" 2>/dev/null)"
}

# ── 單位：原生 fault case ─────────────────────────────────────────────────────
# fault_unit <group> <case> <marker> <config> [VAR=VAL ...]
fault_unit() {
    local group=$1 case=$2 marker=$3 conf=$4; shift 4
    [[ "$conf" = /* ]] || conf="$PS_DIR/$conf"
    local out="$RUNS_DIR/fault/$group/$case"
    check_stop
    if [[ -s "$out/host_status.txt" || -s "$out/refusal.txt" ]]; then UNITS_SKIP=$((UNITS_SKIP+1)); return 0; fi
    UNITS_RUN=$((UNITS_RUN+1))
    log "fault $group/$case marker=${marker:-none} conf=$(basename "$conf") $*"
    if [[ "$DRYRUN" == 1 ]]; then mkdir -p "$out"; echo "alive (dry)" > "$out/host_status.txt"; return 0; fi
    env "${UNIT_ENV[@]}" CASE="$case" MARKER="$marker" CONFIG="$conf" OUT="$out" BIN="$BIN" "$@" \
        bash "$PS_DIR/run_fault_case.sh" < /dev/null >> "$LOG_DIR/fault.out" 2>&1
    log "  -> host=$(cat "$out/host_status.txt" 2>/dev/null) $(head -2 "$out/refusal.txt" 2>/dev/null | tr '\n' ' ') isolate=$(grep -m1 -o '\[parse-isolate\].*' "$out/dut.log" 2>/dev/null | cut -c1-100) probe=$(grep -m1 -o 'PROBE.*' "$out/dut.log" 2>/dev/null | cut -c1-120)"
}
fault_summarize() {  # <group>
    [[ "$DRYRUN" == 1 ]] && return 0
    local d="$RUNS_DIR/fault/$1"
    [[ -d "$d" ]] || return 0
    local py=python3; [[ -x "$REPO_ROOT/test/tools/venv/bin/python" ]] && py="$REPO_ROOT/test/tools/venv/bin/python"
    $py "$FI_DIR/scripts/summarize_fault.py" "$d" > "$d/summarize.out" 2>&1 || log "  (summarize_fault.py failed for $1; see $d/summarize.out)"
}
fault_status() {  # <group> <case>：一行摘要
    local out="$RUNS_DIR/fault/$1/$2"
    echo "$(cat "$out/host_status.txt" 2>/dev/null)"
}

# ── 起始速率（batch_alignment 方法 3 同 session 容量，見 ../batch_alignment/README.md Q3）──────
start_rate() {  # <product> <W>
    case "$1:$2" in
        fbit-lua:0.01) echo 57500;; fbit-lua:0.1) echo 65000;; fbit-lua:0.5) echo 61500;; fbit-lua:1) echo 51000;;
        fluentd:0.01) echo 30000;;  fluentd:0.1) echo 48500;;  fluentd:0.5) echo 46000;;  fluentd:1) echo 39500;;
        vector:0.01) echo 12750;;   vector:0.1) echo 15500;;   vector:0.5) echo 14250;;   vector:1) echo 13750;;
        fbit-wasm:0.01) echo 4300;; fbit-wasm:0.1) echo 5500;; fbit-wasm:0.5) echo 5300;; fbit-wasm:1) echo 5300;;
        wcm-batch:0.01) echo 20250;; wcm-batch:0.1) echo 24750;; wcm-batch:0.5) echo 22000;; wcm-batch:1) echo 19250;;
        *) echo 22000;;
    esac
}
# 產品 → run_point 的 dut、tag 前綴、conf、額外 env
prod_dut() { case "$1" in wcm-batch|wcm-stream) echo wcm;; *) echo "$1";; esac; }
prod_tag() {  # <product> <W>
    case "$1" in
        wcm-batch) echo "batch_W$2";;
        wcm-stream) echo "stream_k${BEST_K}_W$2";;
        *) echo "m3_W$2";;
    esac
}
prod_conf() {  # <product> <W>
    case "$1" in
        fbit-lua) echo "$ALIGN_DIR/conf/fbit_lua_flush_$2.conf";;
        fbit-wasm) echo "$ALIGN_DIR/conf/fbit_wasm_flush_$2.conf";;
        fluentd) echo "$ALIGN_DIR/conf/fluentd_m3_W$2.conf";;
        vector) echo "$ALIGN_DIR/conf/vector_align.yaml";;
        wcm-batch) echo "$PS_DIR/conf/wcm_batch_W$2.yaml";;
        wcm-stream) echo "$PS_DIR/conf/wcm_stream_k${BEST_K}_W$2.yaml";;
    esac
}
prod_env() {  # <product> <W>：Vector 的 W 由 FLUSH_INTERVAL 注入（max_bytes 128 MiB／max_events 1e8 為 capacity_align 預設）
    [[ "$1" == vector ]] && echo "FLUSH_INTERVAL=$2"
    true
}
rtag() { (( $2 > 1 )) && echo "$1_r$2" || echo "$1"; }   # search tag：rep1 不加後綴

# ── preflight ────────────────────────────────────────────────────────────────
preflight() {
    log "run_suite.sh start pid=$$ image=$IMAGE runs=$RUNS_DIR dryrun=$DRYRUN smoke=$SMOKE"
    if [[ "$DRYRUN" != 1 ]]; then
        $DOCKER info >/dev/null 2>&1 || { log "ERROR: docker daemon 無法連線，中止"; exit 1; }
        $DOCKER image inspect "$IMAGE" >/dev/null 2>&1 || { log "ERROR: image $IMAGE 不存在（先依 README 建置），中止"; exit 1; }
        [[ -x "$BIN" ]] || { log "ERROR: $BIN 不存在，中止"; exit 1; }
        for img in "$IMAGE" fluent/fluent-bit:5.0.9 fluent/fluentd:v1.16-debian-1 timberio/vector:0.49.0-alpine; do
            echo "$(date -Is) $img $($DOCKER image inspect --format '{{.Id}}' "$img" 2>/dev/null || echo MISSING)"
        done >> "$LOG_DIR/images.txt"
        {
            echo "$(date -Is) binary_sha256=$(sha256sum "$BIN" | cut -d' ' -f1)"
            for p in "$PS_DIR"/fault/plugin/*.wasm "$REPO_ROOT"/test/ours/latency-benchmark-Rust/plugin/*.wasm; do
                echo "  $(sha256sum "$(readlink -f "$p")" | cut -d' ' -f1)  ${p#$REPO_ROOT/}"
            done
            echo "  git_head=$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null) (working tree has uncommitted changes; see README)"
            echo "  clocksource=$(cat /sys/devices/system/clocksource/clocksource0/current_clocksource 2>/dev/null) nproc=$(nproc) mem=$(free -m | awk '/Mem:/{print $2"MiB total, "$7"MiB avail"}')"
        } >> "$LOG_DIR/environment.txt"
    fi
    [[ -s "$PS_DIR/conf/wcm_stream_k256_W1.yaml" && -s "$PS_DIR/fault/fault_stream_k256_sockdeny.yaml" ]] \
        || bash "$PS_DIR/gen_confs.sh"
}

# ── Phase 0 ──────────────────────────────────────────────────────────────────
phase0() {
    log "===== PHASE 0: capacity check (W=0.1) + BEST_K decision ====="
    search_unit wcm batch_W0.1 conf/wcm_batch_W0.1.yaml "$(start_rate wcm-batch 0.1)"
    search_unit wcm stream_k64_W0.1 conf/wcm_stream_k64_W0.1.yaml 22000
    search_unit wcm stream_k256_W0.1 conf/wcm_stream_k256_W0.1.yaml 22000
    local K W
    for K in 64 256; do for W in 0.5 1; do
        lat_unit wcm 5000 "stream_k${K}_W${W}_r1" "conf/wcm_stream_k${K}_W$W.yaml"
    done; done
    local cb c64 c256 best bestcap ratio meets
    cb=$(cap_of wcm batch_W0.1); c64=$(cap_of wcm stream_k64_W0.1); c256=$(cap_of wcm stream_k256_W0.1)
    if (( c256 > c64 )); then best=256; bestcap=$c256; else best=64; bestcap=$c64; fi
    if (( cb > 0 )); then ratio=$(awk -v a="$bestcap" -v b="$cb" 'BEGIN{printf "%.3f", a/b}'); else ratio=NA; fi
    meets=$(awk -v r="$ratio" 'BEGIN{ if (r=="NA") print "UNKNOWN"; else if (r+0 >= 0.95) print "YES"; else print "NO" }')
    {
        echo "BEST_K=$best"
        echo "cap_batch_W0.1=$cb"
        echo "cap_stream_k64_W0.1=$c64"
        echo "cap_stream_k256_W0.1=$c256"
        echo "best_stream_cap=$bestcap"
        echo "ratio_best_stream_over_batch=$ratio"
        echo "stream_ge_0.95x_batch=$meets"
        for K in 64 256; do for W in 0.5 1; do
            echo "lat5000_stream_k${K}_W${W}: verdict=$(lat_verdict wcm 5000 "stream_k${K}_W${W}_r1") p99_ms=$(lat_p99 wcm 5000 "stream_k${K}_W${W}_r1")"
        done; done
        echo "rule: BEST_K = streaming micro_batch_records with the higher W=0.1 capacity (tie -> 64); later phases use stream k=BEST_K as WCM-stream"
        echo "decided_at=$(date -Is)"
    } > "$RUNS_DIR/phase0_decision.txt"
    BEST_K=$best
    log "PHASE0 SUMMARY: BEST_K=$BEST_K cap batch=$cb k64=$c64 k256=$c256 ratio=$ratio (>=0.95: $meets); lat5000 p99 k64 W0.5/W1=$(lat_p99 wcm 5000 stream_k64_W0.5_r1)/$(lat_p99 wcm 5000 stream_k64_W1_r1) k256=$(lat_p99 wcm 5000 stream_k256_W0.5_r1)/$(lat_p99 wcm 5000 stream_k256_W1_r1) ms"
}

# ── Phase 1 ──────────────────────────────────────────────────────────────────
p1_timeout_secs() { case "$1" in 1000) echo 50;; 5000) echo 70;; 30000) echo 200;; *) echo 40;; esac; }
phase1() {
    log "===== PHASE 1: pattern experiments (WCM-stream = k$BEST_K) ====="
    local rep mr mode conf T K
    # P1 Poison Record Bisection — malformed-rate sweep（C3 型，2,500 EPS，100,000 筆）
    for rep in 1 2 3; do for mr in 0 0.1 1 5 20; do for mode in batch stream; do
        if [[ "$mode" == batch ]]; then conf=conf/wcm_batch_W0.1.yaml; else conf="conf/wcm_stream_k${BEST_K}_W0.1.yaml"; fi
        corr_unit "$mode" "$conf" "$mr" "$rep"
    done; done; done
    local s1="P1a:"
    for mode in batch stream; do for mr in 0 0.1 1 5 20; do
        local root="$RUNS_DIR/correctness/p1_${mode}_mr$mr"
        [[ "$DRYRUN" != 1 && -d "$root" ]] && python3 "$REPO_ROOT/test/compare/correctness/summarize.py" "$root" > "$root/summarize.out" 2>&1
        s1+=" ${mode}@$mr%=$(cat "$root"/C3_rep*/verdict.txt 2>/dev/null | grep -c '^PASS')/3"
    done; done
    # P1 — poison isolation time vs stage timeout（FAULT_LOOP，原生）
    for rep in 1 2 3; do for T in 1000 5000 30000; do
        local secs; secs=$(p1_timeout_secs "$T")
        fault_unit p1_timeout "loop_t${T}_r$rep" FAULT_LOOP "fault/fault_stream_k${BEST_K}_t$T.yaml" SECS="$secs" GEN_TIMEOUT=$((secs + 150))
    done; done
    fault_summarize p1_timeout
    log "PHASE1-P1 SUMMARY: $s1 | timeout runs host: $(for T in 1000 5000 30000; do echo -n "t$T=$(fault_status p1_timeout loop_t${T}_r1) "; done)"

    # P2 Batch the Crossing — capacity vs k（W=0.1），Phase 0 = k64／k256 的 rep1
    local c64; c64=$(cap_of wcm stream_k64_W0.1); (( c64 > 0 )) || c64=22000
    for rep in 1 2 3; do for K in 1 16 64 256; do
        local st=$c64
        [[ $K == 1 ]] && st=$(( c64 * 3 / 4 ))
        [[ $K == 256 ]] && { st=$(cap_of wcm stream_k256_W0.1); (( st > 0 )) || st=$c64; }
        search_unit wcm "$(rtag "stream_k${K}_W0.1" "$rep")" "conf/wcm_stream_k${K}_W0.1.yaml" "$st"
    done; done
    for K in 1 16 64 256; do lat_unit wcm 5000 "stream_k${K}_W0.1_r1" "conf/wcm_stream_k${K}_W0.1.yaml"; done
    local s2="P2:"
    for K in 1 16 64 256; do
        s2+=" k$K cap=$(cap_of wcm "stream_k${K}_W0.1")/$(cap_of wcm "stream_k${K}_W0.1_r2")/$(cap_of wcm "stream_k${K}_W0.1_r3") p99@5k=$(lat_p99 wcm 5000 "stream_k${K}_W0.1_r1")ms;"
    done
    log "PHASE1-P2 SUMMARY: $s2"

    # P4 Watchdog + Disposable Instance
    # epoch on（= reuse on，同一設定共用）vs epoch off vs reuse off，15,000 EPS
    for rep in 1 2 3; do
        lat_unit wcm 15000 "stream_k${BEST_K}_W0.1_r$rep" "conf/wcm_stream_k${BEST_K}_W0.1.yaml"
        lat_unit wcm 15000 "stream_k${BEST_K}_W0.1_noepoch_r$rep" "conf/wcm_stream_k${BEST_K}_W0.1_noepoch.yaml"
        lat_unit wcm 15000 "stream_k${BEST_K}_W0.1_noreuse_r$rep" "conf/wcm_stream_k${BEST_K}_W0.1_noreuse.yaml"
    done
    # watchdog 消融：停用 → 預期 parse 永久停住；注入後 60 s 由 GEN_TIMEOUT 結束
    fault_unit p4_watchdog loop_noepoch_r1 FAULT_LOOP "fault/fault_stream_k${BEST_K}_noepoch.yaml" SECS=75 GEN_TIMEOUT=75
    for rep in 1 2 3; do
        fault_unit p4_watchdog "loop_t2000_r$rep" FAULT_LOOP "fault/fault_stream_k${BEST_K}_t2000.yaml"
        fault_unit p4_reuse "loop_noreuse_r$rep" FAULT_LOOP "fault/fault_stream_k${BEST_K}_noreuse.yaml"
    done
    # FAULT_MEM 以修正後的 RSS 取樣重跑（原 fault_isolation 設定 = batch 模式，與 native_faults/mem 可直接對照）
    fault_unit p4_mem baseline_batch_r1 "" "$FI_DIR/forwarder_fault.yaml"
    for rep in 1 2 3; do fault_unit p4_mem "mem_batch_r$rep" FAULT_MEM "$FI_DIR/forwarder_fault.yaml"; done
    fault_summarize p4_watchdog; fault_summarize p4_reuse; fault_summarize p4_mem
    local s4="P4: cpu@15k epoch_on="
    for rep in 1 2 3; do s4+="$(lat_cpu wcm 15000 "stream_k${BEST_K}_W0.1_r$rep")/"; done
    s4+=" epoch_off="; for rep in 1 2 3; do s4+="$(lat_cpu wcm 15000 "stream_k${BEST_K}_W0.1_noepoch_r$rep")/"; done
    s4+=" reuse_off="; for rep in 1 2 3; do s4+="$(lat_cpu wcm 15000 "stream_k${BEST_K}_W0.1_noreuse_r$rep")/"; done
    s4+=" p99 on/off=$(lat_p99 wcm 15000 "stream_k${BEST_K}_W0.1_r1")/$(lat_p99 wcm 15000 "stream_k${BEST_K}_W0.1_noepoch_r1")ms"
    s4+=" | noepoch loop host=$(fault_status p4_watchdog loop_noepoch_r1)"
    log "PHASE1-P4 SUMMARY: $s4"

    # P5 Capability-Scoped World
    for rep in 1 2 3; do
        fault_unit p5_cap "net_sockdeny_r$rep" FAULT_NET "fault/fault_stream_k${BEST_K}_sockdeny.yaml" MODE=refusal
        fault_unit p5_cap "fileenv_capdeny_r$rep" FAULT_FILE "fault/fault_stream_k${BEST_K}_capdeny.yaml" MODE=refusal
        fault_unit p5_cap "net_inert_r$rep" FAULT_NET "fault/fault_stream_k${BEST_K}_t2000.yaml" SECS=30
        fault_unit p5_cap "file_inert_r$rep" FAULT_FILE "fault/fault_stream_k${BEST_K}_t2000.yaml" SECS=30
        fault_unit p5_cap "env_inert_r$rep" FAULT_ENV "fault/fault_stream_k${BEST_K}_t2000.yaml" SECS=30
        check_stop
        local lc="$RUNS_DIR/p5_loadcheck/loadcheck_r$rep.csv"
        if ! grep -q '^#DONE' "$lc" 2>/dev/null; then
            UNITS_RUN=$((UNITS_RUN+1)); log "loadcheck rep$rep"
            if [[ "$DRYRUN" == 1 ]]; then mkdir -p "$(dirname "$lc")"; echo "#DONE dry" > "$lc"
            else bash "$PS_DIR/loadcheck.sh" "$lc" < /dev/null >> "$LOG_DIR/loadcheck.out" 2>&1; fi
        else UNITS_SKIP=$((UNITS_SKIP+1)); fi
    done
    fault_summarize p5_cap
    local d="$RUNS_DIR/fault/p5_cap"
    log "PHASE1-P5 SUMMARY: net_sockdeny=$(grep -m1 outcome "$d/net_sockdeny_r1/refusal.txt" 2>/dev/null) capdeny=$(grep -m1 outcome "$d/fileenv_capdeny_r1/refusal.txt" 2>/dev/null) | net_inert: $(grep -m1 -o 'PROBE net.*outcome=[A-Z()a-z]*' "$d/net_inert_r1/dut.log" 2>/dev/null) | file: $(grep -m1 -o 'PROBE file.*outcome=[A-Z]*' "$d/file_inert_r1/dut.log" 2>/dev/null) | env: $(grep -m1 -o 'PROBE env.*' "$d/env_inert_r1/dut.log" 2>/dev/null) | loadcheck rows=$(grep -c ',LOAD_' "$RUNS_DIR/p5_loadcheck/loadcheck_r1.csv" 2>/dev/null)"
}

# ── Phase 2 ──────────────────────────────────────────────────────────────────
PRODUCTS="fbit-lua fluentd vector fbit-wasm wcm-batch wcm-stream"
phase2() {
    log "===== PHASE 2: five-product method 3 (WCM-stream = k$BEST_K) ====="
    local rep W p dut tag conf env_kv
    # 延遲 @5,000 EPS × 3（WCM-stream 的 W=0.1／0.5／1 rep1 = Phase 1 P2／Phase 0 的同設定 run）
    for rep in 1 2 3; do for W in $W_LIST; do for p in $PRODUCTS; do
        dut=$(prod_dut $p); tag="$(prod_tag $p $W)_r$rep"; conf=$(prod_conf $p $W); env_kv=$(prod_env $p $W)
        lat_unit "$dut" 5000 "$tag" "$conf" $env_kv
    done; done; done
    # FB WASM：W >= 0.1 時 5,000 EPS 若有任何一次 FAIL，加跑 4,000 EPS × 3
    for W in 0.1 0.5 1; do
        local nfail=0
        for rep in 1 2 3; do [[ "$(lat_verdict fbit-wasm 5000 "m3_W${W}_r$rep")" == PASS ]] || nfail=$((nfail+1)); done
        if (( nfail > 0 )); then
            for rep in 1 2 3; do lat_unit fbit-wasm 4000 "m3_W${W}_r$rep" "$(prod_conf fbit-wasm $W)"; done
        fi
    done
    local s="PHASE2 LAT p99@5k(r1):"
    for p in $PRODUCTS; do s+=" $p="; for W in $W_LIST; do s+="$(lat_p99 $(prod_dut $p) 5000 "$(prod_tag $p $W)_r1")/"; done; done
    log "$s"
    # 容量 × 1（每個 W），W=0.1 再加 2 reps
    for W in $W_LIST; do for p in $PRODUCTS; do
        local st; st=$(start_rate $p $W)
        if [[ $p == wcm-stream ]]; then st=$(cap_of wcm "stream_k${BEST_K}_W0.1"); (( st > 0 )) || st=22000; fi
        if [[ $p == wcm-batch && $W == 0.1 ]]; then local cb; cb=$(cap_of wcm batch_W0.1); (( cb > 0 )) && st=$cb; fi
        search_unit "$(prod_dut $p)" "$(prod_tag $p $W)" "$(prod_conf $p $W)" "$st" $(prod_env $p $W)
    done; done
    for rep in 2 3; do for p in $PRODUCTS; do
        local st; st=$(cap_of "$(prod_dut $p)" "$(prod_tag $p 0.1)"); (( st > 0 )) || st=$(start_rate $p 0.1)
        search_unit "$(prod_dut $p)" "$(rtag "$(prod_tag $p 0.1)" $rep)" "$(prod_conf $p 0.1)" "$st" $(prod_env $p 0.1)
    done; done
    s="PHASE2 CAP:"
    for p in $PRODUCTS; do s+=" $p="; for W in $W_LIST; do s+="$(cap_of $(prod_dut $p) "$(prod_tag $p $W)")/"; done
        s+=" (W0.1 r2/r3 $(cap_of $(prod_dut $p) "$(prod_tag $p 0.1)_r2")/$(cap_of $(prod_dut $p) "$(prod_tag $p 0.1)_r3"));"; done
    log "$s"
}

# ── SMOKE：每種單位一個縮短版真實量測 ───────────────────────────────────────
smoke() {
    BEST_K=64
    log "===== SMOKE (runs_smoke/) ====="
    UNIT_ENV=(LOAD_SECS=30 DRAIN_SECS=10)
    # 1. 一個容量 trial（search_align 的單一 trial 等價：run_point cap）
    check_stop; UNITS_RUN=$((UNITS_RUN+1))
    log "smoke cap trial wcm stream_k64_W0.1 @15000"
    env "${UNIT_ENV[@]}" CONF="$PS_DIR/conf/wcm_stream_k64_W0.1.yaml" TAG=smoke_stream_k64_W0.1 WCM_IMAGE="$IMAGE" \
        bash "$PS_DIR/run_point.sh" wcm 15000 cap < /dev/null >> "$LOG_DIR/smoke.out" 2>&1
    log "  -> $(head -1 "$RUNS_DIR/wcm/cap_rate_15000_smoke_stream_k64_W0.1/verdict.txt" 2>/dev/null)"
    # 2. 一個延遲點（60 s 負載，30–60 s 窗）— W=0.5、large max_transport_bytes
    UNIT_ENV=(LOAD_SECS=60 DRAIN_SECS=10)
    lat_unit wcm 5000 smoke_stream_k64_W0.5 conf/wcm_stream_k64_W0.5.yaml
    # 3. 一個 epoch-off 延遲點
    lat_unit wcm 15000 smoke_stream_k64_W0.1_noepoch conf/wcm_stream_k64_W0.1_noepoch.yaml
    # 4. 一個正確性 run（malformed 1%，30,000 筆）
    UNIT_ENV=(C3_COUNT_OVERRIDE=30000 DRAIN_SECS=10)
    corr_unit stream conf/wcm_stream_k64_W0.1.yaml 1 1
    # 5. 原生 fault：FAULT_NET（inert 與 sockets deny 載入拒絕）、FAULT_LOOP（watchdog 停用，縮短）
    UNIT_ENV=()
    fault_unit smoke net_inert FAULT_NET fault/fault_stream_k64_t2000.yaml SECS=20 FAULT_AT=8
    fault_unit smoke net_sockdeny FAULT_NET fault/fault_stream_k64_sockdeny.yaml MODE=refusal
    fault_unit smoke file_inert FAULT_FILE fault/fault_stream_k64_t2000.yaml SECS=20 FAULT_AT=8
    fault_unit smoke env_inert FAULT_ENV fault/fault_stream_k64_t2000.yaml SECS=20 FAULT_AT=8
    fault_unit smoke loop_noepoch FAULT_LOOP fault/fault_stream_k64_noepoch.yaml SECS=40 FAULT_AT=8 GEN_TIMEOUT=30
    fault_unit smoke loop_t1000 FAULT_LOOP fault/fault_stream_k64_t1000.yaml SECS=30 FAULT_AT=8
    fault_summarize smoke
    log "SMOKE DONE units_run=$UNITS_RUN"
}

# ── main ─────────────────────────────────────────────────────────────────────
preflight
if [[ "$SMOKE" == 1 ]]; then smoke; exit 0; fi
check_stop
BEST_K=""
if [[ -s "$RUNS_DIR/phase0_decision.txt" ]]; then
    BEST_K=$(sed -n 's/^BEST_K=//p' "$RUNS_DIR/phase0_decision.txt")
    log "Phase 0 已完成（$RUNS_DIR/phase0_decision.txt）：BEST_K=$BEST_K"
fi
if [[ -z "$BEST_K" ]]; then phase0; fi
check_stop
phase1
check_stop
phase2
check_stop
summary="DONE $(date -Is) BEST_K=$BEST_K units_run=$UNITS_RUN units_skipped=$UNITS_SKIP $(grep -E '^(ratio|stream_ge)' "$RUNS_DIR/phase0_decision.txt" | tr '\n' ' ')"
if [[ "$DRYRUN" != 1 ]]; then
    python3 "$PS_DIR/collect_suite.py" "$RUNS_DIR" > "$LOG_DIR/collect.out" 2>&1 || log "collect_suite.py failed（見 logs/collect.out；原始資料不受影響）"
fi
if [[ "$DRYRUN" == 1 ]]; then echo "$summary" > "$RUNS_DIR/DONE"; else echo "$summary" > "$PS_DIR/DONE"; fi
log "$summary"
