#!/usr/bin/env bash
set -euo pipefail

benchmark_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
root_dir=$(cd -- "$benchmark_dir/../../../.." && pwd)
language=${1:-rust}
batch=${2:-64}
rounds=${3:-5}
warmup=${4:-2}
cpu=${CPU:-2}

case "$language" in
    rust)
        cargo build --manifest-path "$root_dir/test/ours/copy_overhead/format_benchmark/component/Cargo.toml" --target wasm32-wasip2 --release --locked >&2
        component="$root_dir/test/ours/copy_overhead/format_benchmark/component/target/wasm32-wasip2/release/format_copy_benchmark_component.wasm"
        ;;
    c)
        make -C "$root_dir/test/ours/copy_overhead/format_benchmark/c_component" >&2
        component="$root_dir/test/ours/copy_overhead/format_benchmark/c_component/format_c_benchmark.wasm"
        ;;
    csharp)
        make -C "$root_dir/test/ours/copy_overhead/format_benchmark/csharp_component" >&2
        component="$root_dir/test/ours/copy_overhead/format_benchmark/csharp_component/format_csharp_benchmark.wasm"
        ;;
    *)
        echo "language must be rust, c, or csharp" >&2
        exit 2
        ;;
esac

cargo build --manifest-path "$benchmark_dir/Cargo.toml" --release --locked >&2

config=$(mktemp "$benchmark_dir/format-${language}-XXXXXX.yaml")
trap 'rm -f -- "$config"' EXIT
sed "s|__FORMAT_COMPONENT__|$component|" "$benchmark_dir/format_benchmark.yaml" > "$config"

calls=$((rounds + warmup))
node "$benchmark_dir/generate_input.js" "$batch" "$calls" |
    WCM_BENCH_STDIN=1 \
    WCM_BENCH_BATCH_RECORDS="$batch" \
    WCM_BENCH_WARMUP="$warmup" \
    taskset -c "$cpu" "$benchmark_dir/target/release/wcm-base-log-forwarder" "$config" |
    awk '/^FORMAT_BENCH,/ {sub(/^FORMAT_BENCH,/, ""); print}'
