#!/usr/bin/env bash
# No-WASM 微基準正式執行：native 與 subprocess 兩種方法 × parse/filter/format × 3 runs，
# 每個 batch 100 筆量測 + 10 筆 warm-up（與 copy_overhead 的 WCM／Core JSON／Core Direct 相同）。
# 另外量測 child process 冷啟動（spawn）成本。
#
# 每一個 (method, stage, run) 單元都在 /home/ckt1022/.wcm-bench.lock 內獨占執行，
# 並以 taskset 釘在固定核心（預設 CPU=2，與既有 copy_overhead 實驗相同；
# subprocess 的 child 繼承同一個 affinity，host 與 child 共用同一顆核心）。
#
# 用法：CPU=2 RUNS=3 ROUNDS=100 WARMUP=10 INTERVAL=10 ./scripts/run_micro.sh
set -euo pipefail
dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
cpu=${CPU:-2}
runs=${RUNS:-3}
rounds=${ROUNDS:-100}
warmup=${WARMUP:-10}
interval=${INTERVAL:-10}
batches=${BATCHES:-64,128,256,512,1024,2048,4096,8192}
lock=${LOCK:-/home/ckt1022/.wcm-bench.lock}
out="$dir/results/micro_$(date +%Y%m%d_%H%M%S)"
mkdir -p "$out"

flock "$lock" -c "cd '$dir' && cargo build --release -j 4"
{
    echo "cpu=$cpu runs=$runs rounds=$rounds warmup=$warmup batches=$batches"
    echo "host=$(hostname) kernel=$(uname -r) date=$(date -Is)"
    echo "rustc=$(rustc --version)"
    echo "git=$(git -C "$dir" rev-parse HEAD) (working tree has uncommitted changes)"
} > "$out/meta.txt"

for run in $(seq 1 "$runs"); do
    for method in native subprocess; do
        for stage in parse filter format; do
            f="$out/${method}_${stage}_run${run}.csv"
            echo "[micro] run=$run method=$method stage=$stage → $f"
            flock "$lock" taskset -c "$cpu" "$dir/target/release/micro" \
                --method "$method" --stage "$stage" --run "$run" \
                --rounds "$rounds" --warmup "$warmup" --batches "$batches" > "$f"
        done
    done
    f="$out/spawn_run${run}.csv"
    flock "$lock" taskset -c "$cpu" "$dir/target/release/micro" --method spawn --run "$run" \
        --rounds 20 --warmup 2 > "$f"
    (( run < runs )) && sleep "$interval"
done
echo "[micro] done: $out"
