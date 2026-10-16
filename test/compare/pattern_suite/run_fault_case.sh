#!/bin/bash
# run_fault_case.sh — 原生（無容器、taskset 釘核）單一 fault case，整個 case 在 flock 內。
# 改寫自 test/ours/fault_isolation/scripts/run_fault.sh，量測流程相同（httpsink -parse-ts=false、
# fault_gen.py 定速 + 第 FAULT_AT 秒注入 marker、sample_stats.sh tree 取樣、drain 後停 DUT）。差異：
#   1. CONFIG／OUT 由 env 指定（pattern_suite/fault/*.yaml；plugin 由「設定檔目錄/plugin/」解析）。
#   2. 全程記錄 clock_drift.csv（D = wall − monotonic），供恢復時間的時鐘稽核。
#   3. generator 以 GEN_TIMEOUT 限時（watchdog 停用時 parse 永久停住、TCP 回壓會讓 generator 卡住）；
#      被終止時 fault_gen.py 仍寫 summary（terminated=true）。
#   4. 停 DUT：SIGTERM，10 s 內沒結束再 SIGKILL（記錄在 host_status.txt）。
#   5. MODE=refusal：只啟動 DUT、不送負載，等它自行結束（最多 REFUSAL_WAIT 秒），記錄 exit code 與 stderr
#      （用來記錄載入期能力拒絕訊息）。
# 用法：CASE=loop MARKER=FAULT_LOOP CONFIG=... OUT=... [EPS SECS FAULT_AT GEN_TIMEOUT DRAIN] bash run_fault_case.sh
set -uo pipefail
PS_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
LOCK="${LOCK:-/home/ckt1022/.wcm-bench.lock}"
if [[ -z "${_PS_LOCKED:-}" ]]; then
    exec flock "$LOCK" env _PS_LOCKED=1 bash "${BASH_SOURCE[0]}" "$@"
fi
REPO_ROOT="$(cd "$PS_DIR/../../.." && pwd)"
ALIGN_DIR="$REPO_ROOT/test/compare/batch_alignment"
BIN="${BIN:-$REPO_ROOT/target/release/wcm-base-log-forwarder}"
SINK_BIN="$REPO_ROOT/test/compare/capacity/httpsink_bin"
CONFIG="${CONFIG:?CONFIG required}"
OUT="${OUT:?OUT required}"
CASE="${CASE:-baseline}"
MARKER="${MARKER:-}"
MARKER_COUNT="${MARKER_COUNT:-1}"
EPS="${EPS:-5000}"
SECS="${SECS:-40}"
FAULT_AT="${FAULT_AT:-15}"
GEN_TIMEOUT="${GEN_TIMEOUT:-$(( SECS + 60 ))}"
DRAIN="${DRAIN:-8}"
MODE="${MODE:-run}"
REFUSAL_WAIT="${REFUSAL_WAIT:-30}"
DUT_CORE="${DUT_CORE:-4}"
SINK_CORE="${SINK_CORE:-8}"
GEN_CORE="${GEN_CORE:-9,10}"

mkdir -p "$OUT"
rm -f "$OUT"/*.csv "$OUT"/*.log "$OUT"/error.txt "$OUT"/host_status.txt "$OUT"/refusal.txt "$OUT"/generator_summary.json 2>/dev/null || true
{
    echo "case=$CASE marker=$MARKER marker_count=$MARKER_COUNT eps=$EPS secs=$SECS fault_at=$FAULT_AT gen_timeout=$GEN_TIMEOUT drain=$DRAIN mode=$MODE"
    echo "mode=native (no container/cgroup) pinning: dut=core$DUT_CORE sink=core$SINK_CORE gen=core$GEN_CORE"
    echo "config=$CONFIG"
    echo "binary_sha256=$(sha256sum "$BIN" | cut -d' ' -f1)"
    for p in "$(dirname "$CONFIG")"/plugin/*.wasm; do echo "plugin $(basename "$p") sha256=$(sha256sum "$(readlink -f "$p")" | cut -d' ' -f1)"; done
    echo "date=$(date -Is)"
    echo "----- config -----"; cat "$CONFIG"
} > "$OUT/meta.txt"

python3 "$ALIGN_DIR/clock_drift_logger.py" "$OUT/clock_drift.csv" 0.02 &
DRIFT_PID=$!
SINK_PID=""; DUT_PID=""; STATS_PID=""
cleanup() {
    [[ -n "$STATS_PID" ]] && kill "$STATS_PID" 2>/dev/null
    if [[ -n "$DUT_PID" ]] && kill -0 "$DUT_PID" 2>/dev/null; then
        kill -TERM "$DUT_PID" 2>/dev/null
        for _ in $(seq 20); do kill -0 "$DUT_PID" 2>/dev/null || break; sleep 0.5; done
        kill -9 "$DUT_PID" 2>/dev/null
    fi
    [[ -n "$SINK_PID" ]] && kill -TERM "$SINK_PID" 2>/dev/null && wait "$SINK_PID" 2>/dev/null
    kill "$DRIFT_PID" 2>/dev/null
    true
}
trap cleanup EXIT INT TERM

if [[ "$MODE" == refusal ]]; then
    t0=$(date +%s.%N)
    WCM_ERROR_FILE="$OUT/error.txt" timeout -k 5 "$REFUSAL_WAIT" taskset -c "$DUT_CORE" "$BIN" "$CONFIG" > "$OUT/dut.log" 2>&1
    rc=$?
    t1=$(date +%s.%N)
    {
        echo "exit_code=$rc elapsed_s=$(python3 -c "print(round($t1-$t0,3))")"
        if (( rc == 124 || rc == 137 )); then echo "outcome=NOT_REFUSED(still running after ${REFUSAL_WAIT}s)"; elif (( rc != 0 )); then echo "outcome=REFUSED_AT_LOAD"; else echo "outcome=EXITED_0"; fi
        grep -E '^\[capability\]|LOAD_REFUSED|未授權|refus|denied' "$OUT/dut.log" | head -20
    } > "$OUT/refusal.txt"
    echo "[fault] $CASE refusal → $(head -2 "$OUT/refusal.txt" | tr '\n' ' ')"
    exit 0
fi

taskset -c "$SINK_CORE" "$SINK_BIN" -port 8080 -out-dir "$OUT" -parse-ts=false > "$OUT/sink.log" 2>&1 &
SINK_PID=$!
WCM_ERROR_FILE="$OUT/error.txt" taskset -c "$DUT_CORE" "$BIN" "$CONFIG" > "$OUT/dut.log" 2>&1 &
DUT_PID=$!

ready=false
for _ in $(seq 1 60); do
    if timeout 1 bash -c 'echo -n > /dev/tcp/127.0.0.1/5140' 2>/dev/null; then ready=true; break; fi
    kill -0 "$DUT_PID" 2>/dev/null || break
    sleep 1
done
if [[ "$ready" != true ]]; then
    echo "DUT not ready (exited or 60 s timeout)" | tee "$OUT/host_status.txt" >&2
    exit 1
fi
sleep 2
"$REPO_ROOT/test/compare/sample_stats.sh" "$DUT_PID" "$OUT/stats.csv" 512m 1 tree &
STATS_PID=$!

gen_args=(--addr 127.0.0.1:5140 --eps "$EPS" --secs "$SECS" --fault-at "$FAULT_AT" --summary "$OUT/generator_summary.json")
[[ -n "$MARKER" ]] && gen_args+=(--marker "$MARKER" --marker-count "$MARKER_COUNT")
timeout -s TERM -k 10 "$GEN_TIMEOUT" taskset -c "$GEN_CORE" python3 -I "$PS_DIR/fault_gen.py" "${gen_args[@]}" > "$OUT/generator.log" 2>&1
echo "gen_rc=$?" >> "$OUT/meta.txt"

sleep "$DRAIN"
kill "$STATS_PID" 2>/dev/null; wait "$STATS_PID" 2>/dev/null; STATS_PID=""
if kill -0 "$DUT_PID" 2>/dev/null; then status=alive; else status="dead(exit)"; fi
kill -TERM "$DUT_PID" 2>/dev/null
for _ in $(seq 20); do kill -0 "$DUT_PID" 2>/dev/null || break; sleep 0.5; done
if kill -0 "$DUT_PID" 2>/dev/null; then kill -9 "$DUT_PID" 2>/dev/null; status+=" (needed SIGKILL)"; fi
wait "$DUT_PID" 2>/dev/null; DUT_PID=""
echo "$status" > "$OUT/host_status.txt"
kill -TERM "$SINK_PID" 2>/dev/null; wait "$SINK_PID" 2>/dev/null; SINK_PID=""
echo "[fault] $CASE done → $OUT (host: $status; $(grep -c . "$OUT/recv_per_sec.csv" 2>/dev/null) sink seconds)"
