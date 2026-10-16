#!/usr/bin/env bash
set -euo pipefail

benchmark_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
target=wasm32-unknown-unknown

for stage in parse filter format; do
    cargo build \
        --manifest-path "$benchmark_dir/guest-${stage}/Cargo.toml" \
        --target "$target" \
        --release \
        --locked
    wasm-tools validate \
        "$benchmark_dir/guest-${stage}/target/$target/release/core_wasm_json_${stage}.wasm"
done

cargo build \
    --manifest-path "$benchmark_dir/host/Cargo.toml" \
    --release \
    --locked
