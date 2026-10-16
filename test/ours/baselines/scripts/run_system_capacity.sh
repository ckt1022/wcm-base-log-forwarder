#!/bin/bash
# 系統層 no-WASM baseline：同一個 forwarder binary、同一套 generator/sink/判定，
# 只切換 plugins（全 WCM Rust plugin vs 全 host 內建原生），比較
#   (1) 5,000 EPS 的穩態延遲（lat 模式）
#   (2) 最大可持續 EPS（cap 模式 + 倍增/二分 probe）
# 1024 B / 36-field JSON，filter 保留 warn/error。
#
# 原生（無容器、無 cgroup）執行，taskset 釘核心；判定沿用 capacity/eval_trial.py
# 與 calc_latency_stream.py。因此只能與「本腳本」的另一個變體比較，
# 不可與先前 1 CPU / 512 MiB 容器數字直接並列。
#
# 每一個試驗單元（一個 variant、一個 rate、一個 mode）在 /home/ckt1022/.wcm-bench.lock
# 內獨占執行，涵蓋 DUT 啟動到關閉，使用固定 port 5140/8080。
#
# 用法：
#   MODE=lat RATE=5000 LOAD_SECS=150 bash run_system_capacity.sh        # 兩 variant 都跑
#   MODE=cap RATES="10000 15000 20000 25000 30000" bash run_system_capacity.sh
set -uo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BASE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
REPO_ROOT="$(cd "$BASE_DIR/../../.." && pwd)"
CAP="$REPO_ROOT/test/compare/capacity"
BIN="${BIN:-$REPO_ROOT/target/release/wcm-base-log-forwarder}"
GEN_BIN="$CAP/gen_bin"
SINK_BIN="$CAP/httpsink_bin"
MODE="${MODE:-lat}"
RATES="${RATES:-5000}"
LOAD_SECS="${LOAD_SECS:-$([[ "$MODE" == lat ]] && echo 150 || echo 60)}"
DRAIN_SECS="${DRAIN_SECS:-20}"
VARIANTS="${VARIANTS:-wcm native}"
RUN_ID="${RUN_ID:-$(date +%Y%m%d_%H%M%S)}"
DUT_CORE="${DUT_CORE:-4}"; SINK_CORE="${SINK_CORE:-8}"; GEN_CORES="${GEN_CORES:-9,10}"
LOCK="${LOCK:-/home/ckt1022/.wcm-bench.lock}"

[[ -x "$GEN_BIN" ]] || ( cd "$REPO_ROOT/tools/gen" && go build -o "$GEN_BIN" main.go )
[[ -x "$SINK_BIN" ]] || ( cd "$CAP/httpsink" && go build -o "$SINK_BIN" main.go )

trial() {  # $1=variant $2=rate
    local variant="$1" rate="$2" out cfg parse_ts prealloc
    cfg="$BASE_DIR/system/forwarder_${variant}.yaml"
    out="$BASE_DIR/results/system_${RUN_ID}/${variant}_${MODE}_${rate}"
    mkdir -p "$out"
    if [[ "$MODE" == lat ]]; then parse_ts=true; prealloc=$(( rate * 4 / 10 * LOAD_SECS * 11 / 10 )); else parse_ts=false; prealloc=0; fi
    local sink_epoch; sink_epoch=$(date +%s.%N); echo "$sink_epoch" > "$out/sink_epoch.txt"
    taskset -c "$SINK_CORE" "$SINK_BIN" -port 8080 -out-dir "$out" -parse-ts="$parse_ts" -prealloc "$prealloc" > "$out/sink.log" 2>&1 &
    local sink_pid=$!
    WCM_ERROR_FILE="$out/error.txt" taskset -c "$DUT_CORE" "$BIN" "$cfg" > "$out/dut.log" 2>&1 &
    local dut_pid=$!
    local ready=false
    for _ in $(seq 1 60); do timeout 1 bash -c 'echo -n > /dev/tcp/127.0.0.1/5140' 2>/dev/null && { ready=true; break; }; sleep 1; done
    [[ "$ready" == true ]] || { echo "DUT not ready" >&2; kill "$dut_pid" "$sink_pid" 2>/dev/null; return 1; }
    sleep 3
    local dut_pid_stats=""
    "$REPO_ROOT/test/compare/sample_stats.sh" "$dut_pid" "$out/stats.csv" >/dev/null 2>&1 &
    dut_pid_stats=$!
    local gs ge; gs=$(date +%s.%N)
    timeout -k 5 $(( LOAD_SECS + 90 )) taskset -c "$GEN_CORES" "$GEN_BIN" \
        -rate "$rate" -duration "$LOAD_SECS" -traffic flat -mode json-fields1k \
        -ts-mode sched -output tcp -tcp-addr 127.0.0.1:5140 \
        -send-unit batch -batch-lines 500 -flush-ms 20 -conns 1 -log-file "$out/gen.log" || true
    ge=$(date +%s.%N); echo "$gs $ge" > "$out/gen_epoch.txt"
    sleep "$DRAIN_SECS"
    kill "$dut_pid_stats" 2>/dev/null || true; wait "$dut_pid_stats" 2>/dev/null || true
    echo false > "$out/oom.txt"
    kill -TERM "$dut_pid" 2>/dev/null || true; wait "$dut_pid" 2>/dev/null || true
    kill -TERM "$sink_pid" 2>/dev/null || true; wait "$sink_pid" 2>/dev/null || true
    python3 "$CAP/eval_trial.py" "$out" "$rate" | tee "$out/verdict.txt" || true
    if [[ "$MODE" == lat && -s "$out/latency_worker_1.csv" ]]; then
        python3 "$CAP/calc_latency_stream.py" "$out/latency_worker_1.csv" 30 $(( LOAD_SECS > 150 ? 150 : LOAD_SECS - 10 )) | tee "$out/latency_stats.txt"
    fi
    if [[ -s "$out/stats.csv" ]]; then
        awk -F, 'NR>1 && $1>=30 {n++; c+=$2; m+=$5; if($2>cm)cm=$2; if($5>mm)mm=$5} END{if(n)printf "[stats] samples=%d cpu_mean=%.2f%% cpu_max=%.2f%% rss_mean=%.2fMiB rss_max=%.2fMiB\n",n,c/n,cm,m/n,mm}' "$out/stats.csv" | tee "$out/resource_stats.txt"
    fi
}

for variant in $VARIANTS; do
    for rate in $RATES; do
        echo "[system] ===== variant=$variant mode=$MODE rate=$rate load=${LOAD_SECS}s ====="
        flock "$LOCK" bash -c "$(declare -f trial); \
            MODE='$MODE' LOAD_SECS='$LOAD_SECS' DRAIN_SECS='$DRAIN_SECS' RUN_ID='$RUN_ID' \
            BASE_DIR='$BASE_DIR' REPO_ROOT='$REPO_ROOT' CAP='$CAP' BIN='$BIN' GEN_BIN='$GEN_BIN' SINK_BIN='$SINK_BIN' \
            DUT_CORE='$DUT_CORE' SINK_CORE='$SINK_CORE' GEN_CORES='$GEN_CORES' trial '$variant' '$rate'"
    done
done
echo "[system] done: $BASE_DIR/results/system_${RUN_ID}"
