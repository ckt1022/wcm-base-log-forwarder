#!/usr/bin/env bash
set -euo pipefail

benchmark_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
stage=${1:-format}
rounds=${ROUNDS:-100}
warmup=${WARMUP:-10}
timestamp=$(date +%Y%m%d_%H%M%S)

case "$stage" in
    filter) runner="$benchmark_dir/run_one.sh" ;;
    format) runner="$benchmark_dir/run_format_one.sh" ;;
    *) echo "stage must be filter or format" >&2; exit 2 ;;
esac

for language in rust c csharp; do
    for run in 1 2 3; do
        output="$benchmark_dir/results/src_${stage}_${language}_fresh_run${run}_${timestamp}.csv"
        first_batch=1
        for batch in 64 128 256 512 1024 2048 4096 8192 16384 32768; do
            temporary=$(mktemp "$benchmark_dir/results/.${stage}-${language}-${batch}-XXXXXX.csv")
            echo "${stage} ${language} run ${run}/3 batch=${batch}" >&2
            "$runner" "$language" "$batch" "$rounds" "$warmup" > "$temporary"
            if [[ "$first_batch" -eq 1 ]]; then
                cp "$temporary" "$output"
                first_batch=0
            else
                tail -n +2 "$temporary" >> "$output"
            fi
            rm -f -- "$temporary"
        done

        expected=$((1 + 10 * rounds))
        lines=$(wc -l < "$output")
        if [[ "$lines" -ne "$expected" ]]; then
            echo "ERROR: expected ${expected} lines, got ${lines}: ${output}" >&2
            exit 1
        fi
    done
done

echo "all copied-src ${stage} runs completed (${timestamp})" >&2
