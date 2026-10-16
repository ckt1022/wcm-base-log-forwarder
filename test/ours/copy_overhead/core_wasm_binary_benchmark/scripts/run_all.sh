#!/usr/bin/env bash
set -euo pipefail

benchmark_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
rounds=${ROUNDS:-100}
warmup=${WARMUP:-10}
runs=${RUNS:-3}
interval=${INTERVAL:-20}
cpu=${CPU:-2}
read -r -a stages <<< "${STAGES:-parse filter format}"
timestamp=$(date +%Y%m%d_%H%M%S)
output_dir="$benchmark_dir/results/core_wasm_direct_memory_${timestamp}"

for value_name in rounds warmup runs interval cpu; do
    value=${!value_name}
    if [[ ! "$value" =~ ^[0-9]+$ ]]; then
        echo "ERROR: ${value_name^^} must be a non-negative integer: $value" >&2
        exit 2
    fi
done
if [[ "$rounds" -eq 0 || "$runs" -eq 0 ]]; then
    echo "ERROR: ROUNDS and RUNS must be greater than zero" >&2
    exit 2
fi
if [[ "${#stages[@]}" -eq 0 ]]; then
    echo "ERROR: STAGES must select at least one stage" >&2
    exit 2
fi
for stage in "${stages[@]}"; do
    case "$stage" in
        parse|filter|format) ;;
        *)
            echo "ERROR: invalid stage in STAGES: $stage" >&2
            exit 2
            ;;
    esac
done

mkdir -p -- "$output_dir"
"$benchmark_dir/scripts/build.sh" >&2

temporary=""
cleanup() {
    if [[ -n "$temporary" ]]; then
        rm -f -- "$temporary"
    fi
}
trap cleanup EXIT

total=$((${#stages[@]} * runs))
completed=0
echo "Output directory: $output_dir" >&2
echo "Experiments: $total; interval: ${interval}s; CPU: $cpu" >&2

for stage in "${stages[@]}"; do
    for run in $(seq 1 "$runs"); do
        output="$output_dir/${stage}_rust_core_wasm_direct_memory_run${run}.csv"
        temporary=$(mktemp "$output_dir/.${stage}-run${run}-XXXXXX.csv")
        echo "[$((completed + 1))/$total] stage=$stage language=rust run=$run/$runs" >&2
        CPU="$cpu" "$benchmark_dir/scripts/run_one.sh" \
            "$stage" 64 32768 "$rounds" "$warmup" > "$temporary"

        expected=$((1 + 10 * rounds))
        lines=$(wc -l < "$temporary")
        if [[ "$lines" -ne "$expected" ]]; then
            echo "ERROR: expected $expected lines, got $lines" >&2
            exit 1
        fi
        mv -- "$temporary" "$output"
        temporary=""
        completed=$((completed + 1))
        echo "  saved: $output" >&2

        if [[ "$completed" -lt "$total" && "$interval" -gt 0 ]]; then
            echo "  cooling down for ${interval}s..." >&2
            sleep "$interval"
        fi
    done
done

echo "All $total experiments completed: $output_dir" >&2
