#!/usr/bin/env bash
set -euo pipefail

benchmark_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
copy_overhead_dir=$(cd -- "$benchmark_dir/.." && pwd)
stage=${1:-}
start=${2:-64}
end=${3:-32768}
rounds=${4:-20}
warmup=${5:-3}
sizes=${6:-}
cpu=${CPU:-2}

case "$stage" in
    parse)
        taskset -c "$cpu" "$copy_overhead_dir/parse_benchmark/run.sh" \
            "$start" "$end" "$rounds" "$warmup" "$sizes" fresh full
        ;;
    filter)
        taskset -c "$cpu" "$copy_overhead_dir/filter_benchmark/run.sh" \
            "$start" "$end" "$rounds" "$warmup" "$sizes"
        ;;
    format)
        taskset -c "$cpu" "$copy_overhead_dir/format_benchmark/run.sh" \
            "$start" "$end" "$rounds" "$warmup" "$sizes"
        ;;
    *)
        echo "usage: $0 {parse|filter|format} [start] [end] [rounds] [warmup] [size-list]" >&2
        exit 2
        ;;
esac
