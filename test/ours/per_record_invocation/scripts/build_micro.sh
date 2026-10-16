#!/bin/bash
# 在共用量測鎖內 build micro benchmark（release，-j 4）。
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../micro" && pwd)"
exec flock /home/ckt1022/.wcm-bench.lock cargo build --release -j 4 --manifest-path "$DIR/Cargo.toml"
