#!/usr/bin/env bash
set -euo pipefail

benchmark_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
stage=${1:-parse}
start=${2:-64}
end=${3:-32768}
rounds=${4:-20}
warmup=${5:-3}
cpu=${CPU:-2}

case "$stage" in
    parse|filter|format) ;;
    *) echo "stage must be parse, filter, or format" >&2; exit 2 ;;
esac

module="$benchmark_dir/guest-${stage}/target/wasm32-unknown-unknown/release/core_wasm_binary_${stage}.wasm"
host="$benchmark_dir/host/target/release/core-wasm-binary-host"

if [[ ! -x "$host" || ! -f "$module" ]]; then
    "$benchmark_dir/scripts/build.sh" >&2
fi

taskset -c "$cpu" "$host" \
    "$stage" "$start" "$end" "$rounds" "$warmup" "$module"
