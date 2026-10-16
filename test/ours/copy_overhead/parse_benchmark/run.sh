#!/usr/bin/env bash
set -euo pipefail

benchmark_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

cargo build \
  --manifest-path "$benchmark_dir/component/Cargo.toml" \
  --target wasm32-wasip2 \
  --release \
  --locked

cargo run \
  --manifest-path "$benchmark_dir/host/Cargo.toml" \
  --release \
  --locked \
  -- "${1:-64}" "${2:-32768}" "${3:-20}" "${4:-3}" "${5:-}" "${6:-reuse}" "${7:-full}"
