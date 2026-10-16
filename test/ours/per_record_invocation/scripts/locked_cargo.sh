#!/bin/bash
# 在共用量測鎖內執行 host 的 cargo build/test（release profile，-j 4）。
# 用法: locked_cargo.sh build|test
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
cd "$ROOT"
exec flock /home/ckt1022/.wcm-bench.lock cargo "$1" --release -j 4
