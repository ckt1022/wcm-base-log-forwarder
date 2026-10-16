#!/bin/bash
# WCM 正確性情境 C1–C4 的「原生（無容器、無 cgroup）」版本。
#
# 與 run_wcm_correctness.sh 相同的 generator → forwarder → validation sink 流程與判定
# （evaluate.py／summarize.py），差異只有 DUT 的執行方式：
#   - 不使用 docker：直接執行 release binary，taskset 釘在 DUT_CORE（預設 4）；
#   - 沒有 1 CPU／512 MiB cgroup 限制，mem-guard 會自動停用；
#   - CPU／RSS 以 sample_stats.sh（process 範圍）取樣，不可與容器 cgroup 數字混用；
#   - sink 釘在 core 8、generator 釘在 core 9,10（與容器版相同）。
# 因此本腳本的結果只能與「同樣以本腳本執行」的結果比較（例如 C3 修正前後）。
#
# 每一輪（一個 scenario × 一個 rep）在 /home/ckt1022/.wcm-bench.lock 內獨占執行。
#
# 環境變數：
#   VARIANT  isolation（預設，parse_isolation: true）| legacy（parse_isolation: false）
#   BIN      forwarder binary（預設 target/release/wcm-base-log-forwarder）
#   RUN_ID   結果目錄名稱；REPEATS（預設 3）；SCENARIOS（預設 C1 C2 C3 C4）
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../../.." && pwd)"
VARIANT="${VARIANT:-isolation}"
CONFIG="${CONFIG:-$SCRIPT_DIR/native/forwarder_b1_${VARIANT}.yaml}"
BIN="${BIN:-$REPO_ROOT/target/release/wcm-base-log-forwarder}"
REPEATS="${REPEATS:-3}"
SCENARIOS="${SCENARIOS:-C1 C2 C3 C4}"
RUN_ID="${RUN_ID:-native_${VARIANT}_$(date +%Y%m%d_%H%M%S)}"
RUN_ROOT="$SCRIPT_DIR/runs/$RUN_ID"
GEN_BIN="$SCRIPT_DIR/correctgen_bin"
SINK_BIN="$SCRIPT_DIR/validationsink_bin"
DRAIN_SECS="${DRAIN_SECS:-20}"
DUT_CORE="${DUT_CORE:-4}"
LOCK="${LOCK:-/home/ckt1022/.wcm-bench.lock}"

[[ -f "$CONFIG" ]] || { echo "missing config $CONFIG" >&2; exit 2; }
[[ -x "$BIN" ]] || { echo "missing binary $BIN" >&2; exit 2; }
[[ -x "$GEN_BIN" && -x "$SINK_BIN" ]] || { echo "missing correctgen_bin/validationsink_bin" >&2; exit 2; }

mkdir -p "$RUN_ROOT"
{
    echo "mode=native (no container, no cgroup limit)"
    echo "variant=$VARIANT config=$CONFIG"
    echo "binary=$BIN sha256=$(sha256sum "$BIN" | cut -d' ' -f1)"
    echo "pinning: dut=core$DUT_CORE sink=core8 generator=core9,10"
    echo "date=$(date -Is) kernel=$(uname -r)"
} > "$RUN_ROOT/meta.txt"

run_one() {
    local scenario="$1" rep="$2" rate count malformed out
    case "$scenario" in
        C1) rate=2500;  count=100000;  malformed=false ;;
        C2) rate=17500; count=1000000; malformed=false ;;
        C3) rate=2500;  count=100000;  malformed=true ;;
        C4) rate=2500;  count=100000;  malformed=false ;;
        *) echo "unknown scenario: $scenario" >&2; return 2 ;;
    esac
    out="$RUN_ROOT/${scenario}_rep${rep}"
    mkdir -p "$out"
    printf '%s\n' "$count" > "$out/input_count.txt"
    printf '%s\n' "$rate" > "$out/rate.txt"
    printf '%s\n' "$malformed" > "$out/malformed.txt"
    echo "[native-correctness] ===== $VARIANT $scenario rep=$rep rate=$rate count=$count malformed=$malformed ====="

    local sink_args=(-port 8080 -out-dir "$out" -input-count "$count")
    local gen_args=(-rate "$rate" -count "$count" -tcp-addr 127.0.0.1:5140 -summary "$out/generator_summary.json")
    if [[ "$malformed" == true ]]; then sink_args+=(-malformed); gen_args+=(-malformed); fi
    if [[ "$scenario" == C4 ]]; then sink_args+=(-edge-types); gen_args+=(-edge-types); fi

    taskset -c 8 "$SINK_BIN" "${sink_args[@]}" > "$out/sink.log" 2>&1 &
    local sink_pid=$!
    # error.txt 寫到本輪目錄，便於檢查被拒絕的行
    WCM_ERROR_FILE="$out/error.txt" taskset -c "$DUT_CORE" "$BIN" "$CONFIG" > "$out/dut.log" 2>&1 &
    local dut_pid=$!
    echo "native pid=$dut_pid binary=$BIN" > "$out/image.txt"

    local ready=false
    for _ in $(seq 1 60); do
        if timeout 1 bash -c 'echo -n > /dev/tcp/127.0.0.1/5140' 2>/dev/null; then ready=true; break; fi
        sleep 1
    done
    if [[ "$ready" != true ]]; then
        echo "[native-correctness] DUT readiness timeout" >&2
        kill "$dut_pid" "$sink_pid" 2>/dev/null || true
        return 1
    fi
    sleep 3
    "$REPO_ROOT/test/compare/sample_stats.sh" "$dut_pid" "$out/stats.csv" &
    local stats_pid=$!

    taskset -c 9,10 "$GEN_BIN" "${gen_args[@]}" | tee "$out/generator.log"
    echo "[native-correctness] generator done; drain ${DRAIN_SECS}s"
    sleep "$DRAIN_SECS"

    kill "$stats_pid" 2>/dev/null || true
    wait "$stats_pid" 2>/dev/null || true
    # 容器版的 OOM／存活欄位以 process 狀態代替
    echo false > "$out/oom.txt"
    if kill -0 "$dut_pid" 2>/dev/null; then
        echo '{"Running":true,"Restarting":false,"Native":true}' > "$out/pre_stop_state.json"
    else
        echo '{"Running":false,"Restarting":false,"Native":true}' > "$out/pre_stop_state.json"
    fi
    kill -TERM "$dut_pid" 2>/dev/null || true
    wait "$dut_pid" 2>/dev/null || true
    kill -TERM "$sink_pid" 2>/dev/null || true
    wait "$sink_pid" 2>/dev/null || true

    python3 "$SCRIPT_DIR/evaluate.py" "$out" || true
}

for scenario in $SCENARIOS; do
    for rep in $(seq 1 "$REPEATS"); do
        # 一輪一鎖：與其他量測交錯但不重疊
        flock "$LOCK" bash -c "$(declare -f run_one); \
            RUN_ROOT='$RUN_ROOT' VARIANT='$VARIANT' CONFIG='$CONFIG' BIN='$BIN' GEN_BIN='$GEN_BIN' \
            SINK_BIN='$SINK_BIN' DRAIN_SECS='$DRAIN_SECS' DUT_CORE='$DUT_CORE' REPO_ROOT='$REPO_ROOT' \
            SCRIPT_DIR='$SCRIPT_DIR' run_one '$scenario' '$rep'"
    done
done

python3 "$SCRIPT_DIR/summarize.py" "$RUN_ROOT" || true
echo "[native-correctness] completed run root: $RUN_ROOT"
