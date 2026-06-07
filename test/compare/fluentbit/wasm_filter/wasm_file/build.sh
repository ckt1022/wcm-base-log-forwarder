#!/usr/bin/env bash
# Build all crash_* WASM filters from Rust source.
#
# Prerequisites (one of):
#   A) Local Rust toolchain:
#        rustup target add wasm32-wasip1   # Rust ≥ 1.78
#        # or: rustup target add wasm32-wasi  (Rust < 1.78)
#   B) Docker (script falls back to a Rust Docker image if cargo is absent):
#        docker must be available and accessible
#
# Output: crash_loop.wasm  crash_io.wasm  crash_cpu.wasm  crash_mem.wasm
#         written to this directory (wasm_file/).
#
# Usage: ./build.sh [--docker]
#   --docker   force Docker-based build even when cargo is installed locally

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"

# ── Target selection ──────────────────────────────────────────────────────────
# Rust renamed wasm32-wasi → wasm32-wasip1 in 1.78.
# Prefer wasip1; fall back to wasi for older toolchains.
select_target() {
    if rustup target list --installed 2>/dev/null | grep -q "^wasm32-wasip1$"; then
        echo "wasm32-wasip1"
    elif rustup target list --installed 2>/dev/null | grep -q "^wasm32-wasi$"; then
        echo "wasm32-wasi"
    else
        # Try to add wasip1; if unavailable (old Rust), add wasi
        if rustup target add wasm32-wasip1 2>/dev/null; then
            echo "wasm32-wasip1"
        else
            rustup target add wasm32-wasi
            echo "wasm32-wasi"
        fi
    fi
}

# ── Docker-based build ────────────────────────────────────────────────────────
build_via_docker() {
    local target="wasm32-wasip1"
    local rust_image="rust:1.82-slim"
    echo "[build] Using Docker image: $rust_image"
    sudo docker run --rm \
        -v "$SCRIPT_DIR:/workspace" \
        -w /workspace \
        "$rust_image" \
        bash -c "
            set -euo pipefail
            rustup target add ${target} 2>/dev/null || rustup target add wasm32-wasi
            # detect which target actually got installed
            if rustup target list --installed | grep -q '^wasm32-wasip1$'; then
                T=wasm32-wasip1
            else
                T=wasm32-wasi
            fi
            cargo build --release --target \"\$T\"
            echo \"[build] Built with target: \$T\"
        "
    # copy .wasm files out of target/
    local built_target
    if [ -d "$SCRIPT_DIR/target/wasm32-wasip1/release" ]; then
        built_target="wasm32-wasip1"
    else
        built_target="wasm32-wasi"
    fi
    copy_wasm_files "$built_target"
}

# ── Local build ───────────────────────────────────────────────────────────────
build_locally() {
    local target
    target="$(select_target)"
    echo "[build] Using local cargo, target: $target"
    cd "$SCRIPT_DIR"
    cargo build --release --target "$target"
    copy_wasm_files "$target"
}

# ── Copy compiled .wasm files to wasm_file/ root ─────────────────────────────
copy_wasm_files() {
    local target="$1"
    local release_dir="$SCRIPT_DIR/target/${target}/release"
    echo ""
    echo "[build] Copying WASM files to $SCRIPT_DIR:"
    for wasm in "$release_dir"/crash_*.wasm; do
        [ -f "$wasm" ] || continue
        local name
        name="$(basename "$wasm")"
        cp "$wasm" "$SCRIPT_DIR/$name"
        echo "  $name  ($(du -h "$SCRIPT_DIR/$name" | cut -f1))"
    done
}

# ── Entry point ───────────────────────────────────────────────────────────────
FORCE_DOCKER=false
[[ "${1:-}" == "--docker" ]] && FORCE_DOCKER=true

if $FORCE_DOCKER || ! command -v cargo &>/dev/null; then
    echo "[build] cargo not found or --docker requested; building inside Docker"
    build_via_docker
else
    build_locally
fi

echo ""
echo "[build] Done. WASM files:"
ls -lh "$SCRIPT_DIR"/*.wasm 2>/dev/null || echo "  (none found — check build output above)"
