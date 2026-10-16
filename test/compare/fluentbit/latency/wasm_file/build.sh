#!/usr/bin/env bash
# Build latency_filter.wasm（fluent-bit WASM filter，延遲對比實驗）
#
# Prerequisites: rustup target add wasm32-wasip1
# Output: latency_filter.wasm（本目錄）

set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$SCRIPT_DIR"

# Rust 1.78 起 wasm32-wasi 更名為 wasm32-wasip1（同 wasm_filter/wasm_file/build.sh）
if rustup target list --installed 2>/dev/null | grep -q "^wasm32-wasip1$"; then
    TARGET="wasm32-wasip1"
elif rustup target list --installed 2>/dev/null | grep -q "^wasm32-wasi$"; then
    TARGET="wasm32-wasi"
else
    rustup target add wasm32-wasip1
    TARGET="wasm32-wasip1"
fi

echo "[build] target: $TARGET"
cargo build --release --target "$TARGET"

cp "target/$TARGET/release/latency_filter.wasm" "$SCRIPT_DIR/latency_filter.wasm"
echo "[build] Done:"
ls -lh "$SCRIPT_DIR/latency_filter.wasm"
