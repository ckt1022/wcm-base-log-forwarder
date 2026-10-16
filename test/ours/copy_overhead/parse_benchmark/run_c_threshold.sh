#!/usr/bin/env bash
set -euo pipefail

benchmark_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
batch_sizes=${3:-256,288,320,384,448,512,640,768}

exec "$benchmark_dir/run_c.sh" \
  256 768 "${1:-100}" "${2:-10}" "$batch_sizes" fresh
