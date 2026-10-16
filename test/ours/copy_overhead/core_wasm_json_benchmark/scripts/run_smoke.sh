#!/usr/bin/env bash
set -euo pipefail

benchmark_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
timestamp=$(date +%Y%m%d_%H%M%S)
output_dir="$benchmark_dir/results/smoke_${timestamp}"
mkdir -p -- "$output_dir"

"$benchmark_dir/scripts/build.sh" >&2
for stage in parse filter format; do
    CPU=${CPU:-2} "$benchmark_dir/scripts/run_one.sh" "$stage" 64 64 3 1 \
        > "$output_dir/${stage}_rust_core_wasm_json_smoke.csv"
done

echo "$output_dir" >&2
