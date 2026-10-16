#!/usr/bin/env bash
set -euo pipefail

benchmark_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
batch_sizes=${3:-64,128,256,512,1024,2048,4096,8192}

exec "$benchmark_dir/run_threshold.sh" \
  "${1:-100}" "${2:-10}" "$batch_sizes" fresh
