#!/usr/bin/env bash
set -euo pipefail

benchmark_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
timestamp=$(date +%Y%m%d_%H%M%S)
cpu=${CPU:-2}

for language in c csharp; do
  runner="$benchmark_dir/run_${language}.sh"
  for run in 1 2 3; do
    output="$benchmark_dir/results/filter_${language}_fresh_run${run}_${timestamp}.csv"
    echo "running ${language} round ${run}/3 -> ${output}" >&2
    taskset -c "$cpu" "$runner" 64 8192 100 10 > "$output"

    lines=$(wc -l < "$output")
    if [[ "$lines" -ne 901 ]]; then
      echo "ERROR: expected 901 lines, got ${lines}: ${output}" >&2
      exit 1
    fi
  done
done

echo "all C and C# runs completed (${timestamp})" >&2
