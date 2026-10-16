#!/usr/bin/env bash
set -euo pipefail

benchmark_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
copy_overhead_dir=$(cd -- "$benchmark_dir/.." && pwd)
parse_dir="$copy_overhead_dir/parse_benchmark"

rounds=${ROUNDS:-100}
warmup=${WARMUP:-10}
runs=${RUNS:-3}
interval=${INTERVAL:-20}
cpu=${CPU:-2}
read -r -a stages <<< "${STAGES:-parse filter format}"
read -r -a languages <<< "${LANGUAGES:-rust c csharp}"
batches=(64 128 256 512 1024 2048 4096 8192 16384 32768)
timestamp=$(date +%Y%m%d_%H%M%S)
output_dir="$benchmark_dir/results/all_stages_${timestamp}"

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
for stage in "${stages[@]}"; do
    case "$stage" in
        parse|filter|format) ;;
        *) echo "ERROR: invalid stage in STAGES: $stage" >&2; exit 2 ;;
    esac
done
for language in "${languages[@]}"; do
    case "$language" in
        rust|c|csharp) ;;
        *) echo "ERROR: invalid language in LANGUAGES: $language" >&2; exit 2 ;;
    esac
done

mkdir -p -- "$output_dir"

temporary=""
batch_temporary=""
cleanup() {
    if [[ -n "$temporary" ]]; then
        rm -f -- "$temporary"
    fi
    if [[ -n "$batch_temporary" ]]; then
        rm -f -- "$batch_temporary"
    fi
}
trap cleanup EXIT

parse_runner() {
    case "$1" in
        rust) echo "$parse_dir/run.sh" ;;
        c) echo "$parse_dir/run_c.sh" ;;
        csharp) echo "$parse_dir/run_csharp.sh" ;;
        *) echo "ERROR: unsupported parse language: $1" >&2; return 2 ;;
    esac
}

run_parse_sweep() {
    local language=$1
    local output=$2
    local runner
    local expected
    local lines

    runner=$(parse_runner "$language")
    temporary=$(mktemp "$output_dir/.parse-${language}-XXXXXX.csv")

    # The explicit "fresh" argument matches the production parse pool lifecycle.
    taskset -c "$cpu" "$runner" 64 32768 "$rounds" "$warmup" "" fresh full \
        > "$temporary"

    # Parse includes the zero-entry control in addition to every non-zero batch.
    expected=$((1 + (${#batches[@]} + 1) * rounds))
    lines=$(wc -l < "$temporary")
    if [[ "$lines" -ne "$expected" ]]; then
        echo "ERROR: parse expected $expected lines, got $lines" >&2
        return 1
    fi

    mv -- "$temporary" "$output"
    temporary=""
}

run_pipeline_sweep() {
    local stage=$1
    local language=$2
    local output=$3
    local runner
    local first_batch=1
    local batch
    local expected
    local lines

    case "$stage" in
        filter) runner="$benchmark_dir/run_one.sh" ;;
        format) runner="$benchmark_dir/run_format_one.sh" ;;
        *) echo "ERROR: unsupported pipeline stage: $stage" >&2; return 2 ;;
    esac

    temporary=$(mktemp "$output_dir/.${stage}-${language}-all-XXXXXX.csv")
    for batch in "${batches[@]}"; do
        echo "  batch=$batch" >&2
        batch_temporary=$(mktemp "$output_dir/.${stage}-${language}-${batch}-XXXXXX.csv")
        CPU="$cpu" "$runner" "$language" "$batch" "$rounds" "$warmup" \
            > "$batch_temporary"

        if [[ "$first_batch" -eq 1 ]]; then
            cp -- "$batch_temporary" "$temporary"
            first_batch=0
        else
            tail -n +2 "$batch_temporary" >> "$temporary"
        fi
        rm -f -- "$batch_temporary"
        batch_temporary=""
    done

    expected=$((1 + ${#batches[@]} * rounds))
    lines=$(wc -l < "$temporary")
    if [[ "$lines" -ne "$expected" ]]; then
        echo "ERROR: $stage expected $expected lines, got $lines" >&2
        return 1
    fi

    mv -- "$temporary" "$output"
    temporary=""
}

total=$((${#stages[@]} * ${#languages[@]} * runs))
completed=0

echo "Output directory: $output_dir" >&2
echo "Experiments: $total; interval: ${interval}s; CPU: $cpu" >&2

for stage in "${stages[@]}"; do
    for language in "${languages[@]}"; do
        for run in $(seq 1 "$runs"); do
            output="$output_dir/${stage}_${language}_run${run}.csv"
            echo "[$((completed + 1))/$total] stage=$stage language=$language run=$run/$runs" >&2

            if [[ "$stage" == parse ]]; then
                run_parse_sweep "$language" "$output"
            else
                run_pipeline_sweep "$stage" "$language" "$output"
            fi

            completed=$((completed + 1))
            echo "  saved: $output" >&2

            if [[ "$completed" -lt "$total" && "$interval" -gt 0 ]]; then
                echo "  cooling down for ${interval}s..." >&2
                sleep "$interval"
            fi
        done
    done
done

echo "All $total experiments completed: $output_dir" >&2
