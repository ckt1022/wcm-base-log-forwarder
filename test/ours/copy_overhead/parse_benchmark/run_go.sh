#!/usr/bin/env bash
set -euo pipefail

benchmark_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)

make -C "$benchmark_dir/go_component" >&2

cargo run \
  --manifest-path "$benchmark_dir/host/Cargo.toml" \
  --release \
  --locked \
  -- "${1:-64}" "${2:-8192}" "${3:-20}" "${4:-3}" "${5:-}" "${6:-fresh}" full \
  "$benchmark_dir/go_component/parser_go_benchmark.wasm" go
