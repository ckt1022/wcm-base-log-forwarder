#!/bin/bash
# run_gaps.sh — 補 pattern_suite/ANALYSIS.md §8「仍然薄弱」的證據缺口（見 README.md）。
#
#   G1 batch vs stream(k=256) W=0.1 容量，同一 session 交錯 3 對 + 5,000 EPS 延遲交錯 3 對     （Streaming Micro-Batch）
#   G2 FAULT_LOOP watchdog 消融：epoch off × 3 與 on(T=2 s) × 3 交錯                            （Watchdog and Respawn）
#   G3 epoch on/off 開銷 @ 5k／10k／20k EPS × 3 reps，交錯                                       （Watchdog and Respawn）
#   G4 k ∈ {4, 8, 512, 1024} 與 k=256 wait ∈ {10, 20} ms 的容量 × 2、5,000 EPS 延遲 × 1          （Amortized Boundary Crossing）
#   G5 malformed 1%／5% 負載下的容量（× 1）與 5,000 EPS 延遲（× 2），batch vs stream             （Poison Record Bisection）
#   G6 filter／format／transport 的能力探測（chaos 變體）+ --check-plugin 矩陣 × 3              （Capability-Scoped Stage）
#   G7 perf profiling：perf 無法在此 WSL2 kernel 使用 → 跳過（只寫一行 SUMMARY）
#
# 每個量測單位（一次完整 run）各自在 flock $LOCK 內（run_point.sh、run_fault_case.sh、checkplugin.sh 內部持鎖）。
# 延遲 run 與容量 trial 由 capacity_align.sh 記錄 clock_drift.csv；fault run 由 pattern_suite/run_fault_case.sh 記錄。
# 延遲 run 另以 pattern_suite/fix_latency.py（向前／向後階梯校正）寫 fix_latency.json 到 run 目錄，log 的 p99 用校正值。
# 可續傳：verdict.txt／CAPACITY 行／host_status.txt／refusal.txt／#DONE 已存在的單位會跳過。
# 單位之間檢查 STOP（touch test/compare/pattern_gaps/STOP）；結束時寫 DONE。
#   SMOKE=1  每種單位一個縮短版真實量測（runs_smoke/），然後結束。
set -uo pipefail
G_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$G_DIR/../../.." && pwd)"
PS_DIR="$REPO_ROOT/test/compare/pattern_suite"
FI_DIR="$REPO_ROOT/test/ours/fault_isolation"
LOCK="${LOCK:-/home/ckt1022/.wcm-bench.lock}"
IMAGE="${SUITE_IMAGE:-wcm-base-log-forwarder:suite}"
BIN="$REPO_ROOT/target/release/wcm-base-log-forwarder"
GEN_MAL="$G_DIR/bin/gen_malformed_bin"
DOCKER="${DOCKER:-docker}"
SMOKE="${SMOKE:-0}"
if [[ "$SMOKE" == 1 ]]; then RUNS_DIR="$G_DIR/runs_smoke"; LOG_DIR="$G_DIR/runs_smoke/logs"
else RUNS_DIR="$G_DIR/runs"; LOG_DIR="$G_DIR/logs"; fi
export RUNS_DIR
mkdir -p "$RUNS_DIR" "$LOG_DIR"
UNIT_ENV=()
UNITS_RUN=0; UNITS_SKIP=0

log() { echo "[$(date -Is)] $*"; }
check_stop() {
    if [[ -f "$G_DIR/STOP" ]]; then
        log "STOP 檔存在，於單位之間結束（rm STOP 後重跑即可續傳）"
        exit 0
    fi
}
num_or_zero() { [[ "$1" =~ ^[0-9]+$ ]] && echo "$1" || echo 0; }

# 校正後延遲（fix_latency.py）→ <run>/fix_latency.json；印 p99
fixlat() {  # <run_dir>
    local d=$1
    [[ -s "$d/latency_worker_1.csv" && -s "$d/clock_drift.csv" ]] || { echo NA; return; }
    if [[ ! -s "$d/fix_latency.json" ]]; then
        python3 "$PS_DIR/fix_latency.py" "$d" > "$d/fix_latency.json" 2> "$d/fix_latency.err" || { echo ERR; return; }
    fi
    python3 -c 'import json,sys; j=json.load(open(sys.argv[1])); print(j.get("p99"), "steps=%s" % j.get("n_steps"), ("issues=" + j["issues"]) if j.get("issues") else "")' "$d/fix_latency.json" 2>/dev/null || echo ERR
}

# ── 單位：延遲（run_point.sh lat）────────────────────────────────────────────
# lat_unit <rate> <tag> <conf> [VAR=VAL ...]（dut 一律 wcm）
lat_unit() {
    local rate=$1 tag=$2 conf=$3; shift 3
    [[ "$conf" = /* ]] || conf="$G_DIR/$conf"
    local out="$RUNS_DIR/wcm/lat_rate_${rate}_$tag"
    check_stop
    if [[ -s "$out/verdict.txt" ]]; then UNITS_SKIP=$((UNITS_SKIP+1)); return 0; fi
    UNITS_RUN=$((UNITS_RUN+1))
    log "lat wcm $rate $tag conf=$(basename "$conf") $*"
    env "${UNIT_ENV[@]}" CONF="$conf" TAG="$tag" WCM_IMAGE="$IMAGE" "$@" \
        bash "$G_DIR/run_point.sh" wcm "$rate" lat < /dev/null >> "$LOG_DIR/lat.out" 2>&1
    log "  -> $(head -1 "$out/verdict.txt" 2>/dev/null | cut -c1-60) | p99(fix)=$(fixlat "$out") ms | $(grep -oE 'cpu_mean=[0-9.]+%' "$out/resource_stats_30_150.txt" 2>/dev/null)"
}
lat_p99() { local d="$RUNS_DIR/wcm/lat_rate_${1}_$2"; [[ -s "$d/fix_latency.json" ]] && python3 -c 'import json,sys; print(json.load(open(sys.argv[1])).get("p99"))' "$d/fix_latency.json" 2>/dev/null || echo NA; }
lat_cpu() { grep -oE 'cpu_mean=[0-9.]+' "$RUNS_DIR/wcm/lat_rate_${1}_$2/resource_stats_30_150.txt" 2>/dev/null | cut -d= -f2; }
lat_v() { head -1 "$RUNS_DIR/wcm/lat_rate_${1}_$2/verdict.txt" 2>/dev/null | awk '{print $1}'; }

# ── 單位：容量搜尋（search_align.sh；每個 trial 一個 run_point.sh）───────────
# search_unit <tag> <conf> <start> [VAR=VAL ...]
search_unit() {
    local tag=$1 conf=$2 start=$3; shift 3
    [[ "$conf" = /* ]] || conf="$G_DIR/$conf"
    local res="$RUNS_DIR/wcm/search_$tag.txt"
    check_stop
    if grep -q '^CAPACITY' "$res" 2>/dev/null; then UNITS_SKIP=$((UNITS_SKIP+1)); return 0; fi
    UNITS_RUN=$((UNITS_RUN+1))
    log "search wcm $tag start=$start conf=$(basename "$conf") $*"
    env "${UNIT_ENV[@]}" WCM_IMAGE="$IMAGE" bash "$G_DIR/search_align.sh" wcm "$tag" "$conf" "$start" "$@" \
        < /dev/null >> "$LOG_DIR/search.out" 2>&1
    local rc=$?
    (( rc == 3 )) && check_stop
    log "  -> $(grep '^CAPACITY' "$res" 2>/dev/null | tail -1)"
}
cap_of() {
    local line; line=$(grep '^CAPACITY' "$RUNS_DIR/wcm/search_$1.txt" 2>/dev/null | tail -1)
    local eps; eps=$(sed -n 's/.* eps=\([^ ]*\).*/\1/p' <<< "$line")
    if [[ "$eps" =~ ^[0-9]+$ ]]; then echo "$eps"; return; fi
    num_or_zero "$(sed -n 's/.* last_pass=\([0-9]*\).*/\1/p' <<< "$line")"
}

# ── 單位：原生 fault case（pattern_suite/run_fault_case.sh）──────────────────
# fault_unit <group> <case> <marker> <config> [VAR=VAL ...]
fault_unit() {
    local group=$1 case=$2 marker=$3 conf=$4; shift 4
    [[ "$conf" = /* ]] || conf="$G_DIR/$conf"
    local out="$RUNS_DIR/fault/$group/$case"
    check_stop
    if [[ -s "$out/host_status.txt" || -s "$out/refusal.txt" ]]; then UNITS_SKIP=$((UNITS_SKIP+1)); return 0; fi
    UNITS_RUN=$((UNITS_RUN+1))
    log "fault $group/$case marker=${marker:-none} conf=$(basename "$conf") $*"
    env "${UNIT_ENV[@]}" CASE="$case" MARKER="$marker" CONFIG="$conf" OUT="$out" BIN="$BIN" "$@" \
        bash "$PS_DIR/run_fault_case.sh" < /dev/null >> "$LOG_DIR/fault.out" 2>&1
    log "  -> host=$(cat "$out/host_status.txt" 2>/dev/null) $(head -2 "$out/refusal.txt" 2>/dev/null | tr '\n' ' ') isolate=$(grep -m1 -o '\[parse-isolate\].*' "$out/dut.log" 2>/dev/null | cut -c1-80) probes=[$(grep -o 'PROBE [a-z_]* .*outcome=[A-Za-z()]*' "$out/dut.log" 2>/dev/null | sed 's/ detail=.*//' | sort | uniq -c | tr -s ' ' | tr '\n' ';' | cut -c1-400)] security=$(grep -c 'SECURITY' "$out/dut.log" 2>/dev/null)"
}
fault_summarize() {  # <group>
    local d="$RUNS_DIR/fault/$1"
    [[ -d "$d" ]] || return 0
    local py=python3; [[ -x "$REPO_ROOT/test/tools/venv/bin/python" ]] && py="$REPO_ROOT/test/tools/venv/bin/python"
    $py "$FI_DIR/scripts/summarize_fault.py" "$d" > "$d/summarize.out" 2>&1 || log "  (summarize_fault.py failed for $1; see $d/summarize.out)"
}
zero_tail() {  # <run_dir>：sink 最後連續 0 吞吐秒數（watchdog 停用時應 > 0）
    python3 - "$1/recv_per_sec.csv" <<'EOF' 2>/dev/null || echo NA
import sys
rows=[l.strip().split(',') for l in open(sys.argv[1]).readlines()[1:] if l.strip()]
n=0
for r in reversed(rows):
    if int(r[1])==0: n+=1
    else: break
print(n, "of", len(rows), "s; recv_total=%d" % sum(int(r[1]) for r in rows))
EOF
}

# ── 單位：--check-plugin 矩陣 ─────────────────────────────────────────────────
checkplugin_unit() {  # <rep>
    local f="$RUNS_DIR/g6_checkplugin/checkplugin_r$1.csv"
    check_stop
    if grep -q '^#DONE' "$f" 2>/dev/null; then UNITS_SKIP=$((UNITS_SKIP+1)); return 0; fi
    UNITS_RUN=$((UNITS_RUN+1)); log "checkplugin rep$1"
    bash "$G_DIR/checkplugin.sh" "$f" < /dev/null >> "$LOG_DIR/checkplugin.out" 2>&1
    log "  -> $(grep -c ',LOAD_OK,' "$f" 2>/dev/null) OK / $(grep -c ',LOAD_REFUSED,' "$f" 2>/dev/null) REFUSED"
}

# ── preflight ────────────────────────────────────────────────────────────────
preflight() {
    log "run_gaps.sh start pid=$$ image=$IMAGE runs=$RUNS_DIR smoke=$SMOKE"
    $DOCKER info >/dev/null 2>&1 || { log "ERROR: docker daemon 無法連線，中止"; exit 1; }
    $DOCKER image inspect "$IMAGE" >/dev/null 2>&1 || { log "ERROR: image $IMAGE 不存在，中止"; exit 1; }
    [[ -x "$BIN" ]] || { log "ERROR: $BIN 不存在，中止"; exit 1; }
    [[ -x "$GEN_MAL" ]] || { log "ERROR: $GEN_MAL 不存在（見 README 重現方式），中止"; exit 1; }
    [[ -s "$G_DIR/conf/wcm_stream_k1024_W0.1.yaml" && -s "$G_DIR/fault/cap_transport_endpoint.yaml" ]] || bash "$G_DIR/gen_confs.sh"
    echo "$(date -Is) $IMAGE $($DOCKER image inspect --format '{{.Id}}' "$IMAGE" 2>/dev/null)" >> "$LOG_DIR/images.txt"
    {
        echo "$(date -Is) binary_sha256=$(sha256sum "$BIN" | cut -d' ' -f1)"
        echo "  gen_malformed_bin_sha256=$(sha256sum "$GEN_MAL" | cut -d' ' -f1)  gen_bin_sha256=$(sha256sum "$REPO_ROOT/test/compare/capacity/gen_bin" | cut -d' ' -f1)"
        for p in "$G_DIR"/fault/plugin/*.wasm; do echo "  $(sha256sum "$(readlink -f "$p")" | cut -d' ' -f1)  ${p#$REPO_ROOT/}"; done
        echo "  git_head=$(git -C "$REPO_ROOT" rev-parse HEAD 2>/dev/null) (uncommitted working tree)"
        echo "  clocksource=$(cat /sys/devices/system/clocksource/clocksource0/current_clocksource 2>/dev/null) nproc=$(nproc) mem=$(free -m | awk '/Mem:/{print $2"MiB total, "$7"MiB avail"}')"
    } >> "$LOG_DIR/environment.txt"
}

B_CONF="$PS_DIR/conf/wcm_batch_W0.1.yaml"
S_CONF="$PS_DIR/conf/wcm_stream_k256_W0.1.yaml"
S_NOEPOCH="$PS_DIR/conf/wcm_stream_k256_W0.1_noepoch.yaml"

# ── G1 ───────────────────────────────────────────────────────────────────────
g1() {
    log "===== G1: batch vs stream(k=256) W=0.1, interleaved ====="
    local rep
    # 兩者起點都用 22,000（同一個搜尋網格，避免起點造成的解析度差異）
    for rep in 1 2 3; do
        search_unit "g1_batch_W0.1_r$rep" "$B_CONF" 22000
        search_unit "g1_stream_k256_W0.1_r$rep" "$S_CONF" 22000
    done
    for rep in 1 2 3; do
        lat_unit 5000 "g1_batch_W0.1_r$rep" "$B_CONF"
        lat_unit 5000 "g1_stream_k256_W0.1_r$rep" "$S_CONF"
    done
    local s="G1 SUMMARY: cap batch/stream:" rep2
    for rep2 in 1 2 3; do s+=" r$rep2=$(cap_of "g1_batch_W0.1_r$rep2")/$(cap_of "g1_stream_k256_W0.1_r$rep2")"; done
    s+=" | p99@5k(fix) batch/stream:"
    for rep2 in 1 2 3; do s+=" r$rep2=$(lat_p99 5000 "g1_batch_W0.1_r$rep2")/$(lat_p99 5000 "g1_stream_k256_W0.1_r$rep2")"; done
    s+=" | cpu% batch/stream:"
    for rep2 in 1 2 3; do s+=" $(lat_cpu 5000 "g1_batch_W0.1_r$rep2")/$(lat_cpu 5000 "g1_stream_k256_W0.1_r$rep2")"; done
    log "$s"
}

# ── G2 ───────────────────────────────────────────────────────────────────────
g2() {
    log "===== G2: watchdog ablation (FAULT_LOOP), off x3 / on(T=2s) x3 interleaved ====="
    local rep
    for rep in 1 2 3; do
        # 與 pattern_suite P4 相同：注入（t=15 s）後 60 s 由 GEN_TIMEOUT=75 結束 generator
        fault_unit g2_watchdog "loop_noepoch_r$rep" FAULT_LOOP "$PS_DIR/fault/fault_stream_k256_noepoch.yaml" SECS=75 GEN_TIMEOUT=75
        # 對照：同樣 75 s 負載、watchdog 開（T=2 s）；GEN_TIMEOUT 放寬讓 generator 正常結束
        fault_unit g2_watchdog "loop_t2000_r$rep" FAULT_LOOP "$PS_DIR/fault/fault_stream_k256_t2000.yaml" SECS=75 GEN_TIMEOUT=135
    done
    fault_summarize g2_watchdog
    local d="$RUNS_DIR/fault/g2_watchdog" s="G2 SUMMARY:"
    for rep in 1 2 3; do
        s+=" off_r$rep(host=$(cat "$d/loop_noepoch_r$rep/host_status.txt" 2>/dev/null) zero_tail=$(zero_tail "$d/loop_noepoch_r$rep"))"
        s+=" on_r$rep(host=$(cat "$d/loop_t2000_r$rep/host_status.txt" 2>/dev/null) zero_tail=$(zero_tail "$d/loop_t2000_r$rep"))"
    done
    log "$s"
}

# ── G3 ───────────────────────────────────────────────────────────────────────
g3() {
    log "===== G3: epoch on/off overhead at 5k/10k/20k EPS (stream k=256), 3 reps interleaved ====="
    local rep rate
    for rep in 1 2 3; do for rate in 5000 10000 20000; do
        if (( rep % 2 == 1 )); then
            lat_unit "$rate" "g3_stream_k256_W0.1_epochon_r$rep" "$S_CONF"
            lat_unit "$rate" "g3_stream_k256_W0.1_epochoff_r$rep" "$S_NOEPOCH"
        else
            lat_unit "$rate" "g3_stream_k256_W0.1_epochoff_r$rep" "$S_NOEPOCH"
            lat_unit "$rate" "g3_stream_k256_W0.1_epochon_r$rep" "$S_CONF"
        fi
    done; done
    local s="G3 SUMMARY (cpu% on/off, p99(fix) on/off):"
    for rate in 5000 10000 20000; do
        s+=" @$rate:"
        for rep in 1 2 3; do s+=" $(lat_cpu $rate "g3_stream_k256_W0.1_epochon_r$rep")/$(lat_cpu $rate "g3_stream_k256_W0.1_epochoff_r$rep")"; done
        s+=" p99"
        for rep in 1 2 3; do s+=" $(lat_p99 $rate "g3_stream_k256_W0.1_epochon_r$rep")/$(lat_p99 $rate "g3_stream_k256_W0.1_epochoff_r$rep")"; done
        s+=" verdicts"
        for rep in 1 2 3; do s+=" $(lat_v $rate "g3_stream_k256_W0.1_epochon_r$rep")/$(lat_v $rate "g3_stream_k256_W0.1_epochoff_r$rep")"; done
        s+=";"
    done
    log "$s"
}

# ── G4 ───────────────────────────────────────────────────────────────────────
G4_VARIANTS="k4 k8 k512 k1024 wait10 wait20"
g4_conf() { case "$1" in wait*) echo "conf/wcm_stream_k256_W0.1_$1.yaml";; *) echo "conf/wcm_stream_$1_W0.1.yaml";; esac; }
g4_tag() { case "$1" in wait*) echo "g4_stream_k256_W0.1_$1";; *) echo "g4_stream_$1_W0.1";; esac; }
g4_start() { case "$1" in k4) echo 9000;; k8) echo 12000;; *) echo 22000;; esac; }
g4() {
    log "===== G4: k sweep extension (k=4,8,512,1024) + k=256 wait 10/20 ms ====="
    local rep v
    for rep in 1 2; do for v in $G4_VARIANTS; do
        local st; st=$(g4_start "$v")
        if (( rep == 2 )); then local c; c=$(cap_of "$(g4_tag "$v")_r1"); (( c > 0 )) && st=$c; fi
        search_unit "$(g4_tag "$v")_r$rep" "$(g4_conf "$v")" "$st"
    done; done
    for v in k512 k1024 wait10 wait20; do lat_unit 5000 "$(g4_tag "$v")_r1" "$(g4_conf "$v")"; done
    local s="G4 SUMMARY: cap r1/r2:"
    for v in $G4_VARIANTS; do s+=" $v=$(cap_of "$(g4_tag "$v")_r1")/$(cap_of "$(g4_tag "$v")_r2")"; done
    s+=" | p99@5k(fix)/cpu:"
    for v in k512 k1024 wait10 wait20; do s+=" $v=$(lat_p99 5000 "$(g4_tag "$v")_r1")/$(lat_cpu 5000 "$(g4_tag "$v")_r1")"; done
    log "$s"
}

# ── G5 ───────────────────────────────────────────────────────────────────────
g5() {
    log "===== G5: bisection under load (malformed 1%/5%), batch vs stream ====="
    local mr mode conf rep pct
    for mr in 0.01 0.05; do
        pct=$(awk -v m="$mr" 'BEGIN{printf "%d", m*100}')
        for mode in batch stream; do
            if [[ $mode == batch ]]; then conf=$B_CONF; else conf=$S_CONF; fi
            local st=18000; [[ $mr == 0.05 ]] && st=12000
            search_unit "g5_${mode}_mr${pct}" "$conf" "$st" GEN_BIN="$GEN_MAL" GEN_EXTRA_ARGS="-malformed-rate $mr"
        done
    done
    for rep in 1 2; do for mr in 0.01 0.05; do
        pct=$(awk -v m="$mr" 'BEGIN{printf "%d", m*100}')
        for mode in batch stream; do
            if [[ $mode == batch ]]; then conf=$B_CONF; else conf=$S_CONF; fi
            lat_unit 5000 "g5_${mode}_mr${pct}_r$rep" "$conf" GEN_BIN="$GEN_MAL" GEN_EXTRA_ARGS="-malformed-rate $mr"
        done
    done; done
    local s="G5 SUMMARY: cap batch/stream: 1%=$(cap_of g5_batch_mr1)/$(cap_of g5_stream_mr1) 5%=$(cap_of g5_batch_mr5)/$(cap_of g5_stream_mr5) (0% = G1)"
    s+=" | p99@5k(fix) batch/stream r1,r2:"
    for pct in 1 5; do s+=" $pct%=$(lat_p99 5000 "g5_batch_mr${pct}_r1"),$(lat_p99 5000 "g5_batch_mr${pct}_r2")/$(lat_p99 5000 "g5_stream_mr${pct}_r1"),$(lat_p99 5000 "g5_stream_mr${pct}_r2")"; done
    s+=" | cpu:"
    for pct in 1 5; do s+=" $pct%=$(lat_cpu 5000 "g5_batch_mr${pct}_r1")/$(lat_cpu 5000 "g5_stream_mr${pct}_r1")"; done
    log "$s"
}

# ── G6 ───────────────────────────────────────────────────────────────────────
g6() {
    log "===== G6: capability probes for filter/format/transport (chaos variants) ====="
    local rep stage probe
    for rep in 1 2 3; do
        for stage in filter format transport; do
            fault_unit g6_cap "${stage}_deny_r$rep" FAULT_NET "fault/cap_${stage}_deny.yaml" MODE=refusal
            for probe in NET FILE ENV; do
                fault_unit g6_cap "${stage}_$(tr 'A-Z' 'a-z' <<< "$probe")_inert_r$rep" "FAULT_$probe" "fault/cap_${stage}_inert.yaml" SECS=30
            done
        done
        # transport：raw sockets 以 endpoint 授權 → listed 127.0.0.1:8080 應連上、8081 應被拒；wasi:http 探測同 inert
        fault_unit g6_cap "transport_net_endpoint_r$rep" FAULT_NET "fault/cap_transport_endpoint.yaml" SECS=30
        checkplugin_unit "$rep"
    done
    fault_summarize g6_cap
    local d="$RUNS_DIR/fault/g6_cap" s="G6 SUMMARY (r1):"
    for stage in filter format transport; do
        s+=" $stage: deny=$(grep -m1 outcome "$d/${stage}_deny_r1/refusal.txt" 2>/dev/null | cut -c1-40)"
        for probe in net file env; do
            s+=" $probe=[$(grep -o 'PROBE [a-z_]* .*outcome=[A-Za-z()]*' "$d/${stage}_${probe}_inert_r1/dut.log" 2>/dev/null | sed -e 's/ detail=.*//' -e 's/PROBE //' | sort -u | tr '\n' ';' | cut -c1-300)]"
        done
        s+=";"
    done
    s+=" transport_endpoint=[$(grep -o 'PROBE [a-z_]* .*outcome=[A-Za-z()]*' "$d/transport_net_endpoint_r1/dut.log" 2>/dev/null | sed -e 's/ detail=.*//' -e 's/PROBE //' | sort -u | tr '\n' ';')]"
    s+=" | checkplugin r1: $(grep -c ',LOAD_OK,' "$RUNS_DIR/g6_checkplugin/checkplugin_r1.csv" 2>/dev/null) OK / $(grep -c ',LOAD_REFUSED,' "$RUNS_DIR/g6_checkplugin/checkplugin_r1.csv" 2>/dev/null) REFUSED"
    log "$s"
}

g7() {
    log "G7 SUMMARY: SKIPPED — /usr/bin/perf 是 linux-tools wrapper，'perf --version' 回報 'perf not found for kernel 6.6.87.2-microsoft'（沒有對應此 WSL2 kernel 的 perf binary；不使用 sudo 安裝）。"
}

# ── SMOKE ────────────────────────────────────────────────────────────────────
smoke() {
    log "===== SMOKE (runs_smoke/) ====="
    UNIT_ENV=(LOAD_SECS=30 DRAIN_SECS=10)
    local t
    for t in "g1-type:15000:$G_DIR/conf/wcm_stream_k1024_W0.1.yaml:smoke_stream_k1024::" \
             "g5-type:10000:$B_CONF:smoke_batch_mr5:$GEN_MAL:-malformed-rate 0.05"; do
        IFS=: read -r name rate conf tag gbin gargs <<< "$t"
        check_stop; UNITS_RUN=$((UNITS_RUN+1))
        log "smoke cap trial ($name) wcm $tag @$rate"
        local gen_env=()
        [[ -n "$gbin" ]] && gen_env=(GEN_BIN="$gbin" GEN_EXTRA_ARGS="$gargs")
        env "${UNIT_ENV[@]}" CONF="$conf" TAG="$tag" WCM_IMAGE="$IMAGE" "${gen_env[@]}" \
            bash "$G_DIR/run_point.sh" wcm "$rate" cap < /dev/null >> "$LOG_DIR/smoke.out" 2>&1
        log "  -> $(head -1 "$RUNS_DIR/wcm/cap_rate_${rate}_$tag/verdict.txt" 2>/dev/null)"
    done
    UNIT_ENV=(LOAD_SECS=60 DRAIN_SECS=10)
    lat_unit 5000 smoke_stream_k256_wait20 conf/wcm_stream_k256_W0.1_wait20.yaml
    lat_unit 5000 smoke_stream_mr1 "$S_CONF" GEN_BIN="$GEN_MAL" GEN_EXTRA_ARGS="-malformed-rate 0.01"
    UNIT_ENV=()
    fault_unit smoke loop_noepoch FAULT_LOOP "$PS_DIR/fault/fault_stream_k256_noepoch.yaml" SECS=40 FAULT_AT=8 GEN_TIMEOUT=30
    fault_unit smoke transport_deny FAULT_NET fault/cap_transport_deny.yaml MODE=refusal
    fault_unit smoke filter_net_inert FAULT_NET fault/cap_filter_inert.yaml SECS=20 FAULT_AT=8
    fault_unit smoke format_file_inert FAULT_FILE fault/cap_format_inert.yaml SECS=20 FAULT_AT=8
    fault_unit smoke transport_net_inert FAULT_NET fault/cap_transport_inert.yaml SECS=20 FAULT_AT=8
    fault_unit smoke transport_env_inert FAULT_ENV fault/cap_transport_inert.yaml SECS=20 FAULT_AT=8
    fault_unit smoke transport_net_endpoint FAULT_NET fault/cap_transport_endpoint.yaml SECS=20 FAULT_AT=8
    fault_summarize smoke
    checkplugin_unit 1
    log "SMOKE: noepoch zero_tail=$(zero_tail "$RUNS_DIR/fault/smoke/loop_noepoch")"
    log "SMOKE DONE units_run=$UNITS_RUN"
}

# ── main ─────────────────────────────────────────────────────────────────────
preflight
if [[ "$SMOKE" == 1 ]]; then smoke; exit 0; fi
for grp in g1 g2 g3 g4 g5 g6 g7; do check_stop; $grp; done
check_stop
summary="DONE $(date -Is) units_run=$UNITS_RUN units_skipped=$UNITS_SKIP"
echo "$summary" > "$G_DIR/DONE"
log "$summary"
