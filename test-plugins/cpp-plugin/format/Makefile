# ──────────────────────────────────────────────────────────────────
# C++ format plugin – WASM Component Model build
#
# Prerequisites:
#   wasi-sdk  ≥ 24  (set WASI_SDK_PATH, default ~/wasi-sdk)
#   wasm-tools      (cargo install wasm-tools)
#   wit-bindgen     (cargo install wit-bindgen-cli)
#
# Usage:
#   make                   # build format.wasm (the component)
#   make clean
#   make bindings          # regenerate C bindings from WIT only
# ──────────────────────────────────────────────────────────────────

WASI_SDK_PATH ?= $(HOME)/wasi-sdk
CLANGXX  := $(WASI_SDK_PATH)/bin/clang++
CLANG    := $(WASI_SDK_PATH)/bin/clang
SYSROOT  := $(WASI_SDK_PATH)/share/wasi-sysroot
TARGET   := wasm32-wasip1

# Reactor adapter: bundled in wasm-tools crate in cargo registry
ADAPTER  ?= $(shell find ~/.cargo/registry -name "wasi_snapshot_preview1.reactor.wasm" 2>/dev/null | head -1)

CFLAGS   := --target=$(TARGET) --sysroot=$(SYSROOT) -O2 -I.
CXXFLAGS := $(CFLAGS) -fno-exceptions -Wall

CORE_WASM  := format_core.wasm
FINAL_WASM := format.wasm

.PHONY: all clean bindings check-tools

all: check-tools $(FINAL_WASM)

# ── Step 1a: compile C++ implementation ───────────────────────────
format_impl.o: format_impl.cpp format_plugin.h
	$(CLANGXX) $(CXXFLAGS) -c $< -o $@

# ── Step 1b: compile generated C binding (must stay C, not C++) ───
format_plugin.o: format_plugin.c format_plugin.h
	$(CLANG) $(CFLAGS) -c $< -o $@

# ── Step 1c: link into a WASM core module ─────────────────────────
$(CORE_WASM): format_impl.o format_plugin.o format_plugin_component_type.o
	$(CLANG) $(CFLAGS) \
	    -Wl,--no-entry \
	    -Wl,--export-dynamic \
	    $^ -o $@

# ── Step 2: wrap core module into a WIT component ─────────────────
$(FINAL_WASM): $(CORE_WASM)
	@test -n "$(ADAPTER)" || { \
	    echo "ERROR: wasi_snapshot_preview1.reactor.wasm not found in cargo registry."; \
	    echo "  cargo install wasm-tools  # ensures the adapter is available"; \
	    exit 1; \
	}
	wasm-tools component new $(CORE_WASM) \
	    --adapt wasi_snapshot_preview1="$(ADAPTER)" \
	    -o $(FINAL_WASM)
	@echo "Built: $(FINAL_WASM)"
	@wasm-tools validate $(FINAL_WASM) && echo "Validated OK"

# ── Regenerate C bindings (only when WIT changes) ─────────────────
bindings:
	wit-bindgen c --world format-plugin --out-dir . wit/
	@echo "Regenerated: format_plugin.h  format_plugin.c  format_plugin_component_type.o"

clean:
	rm -f $(CORE_WASM) $(FINAL_WASM) format_impl.o format_plugin.o

check-tools:
	@test -x "$(CLANGXX)" || { \
	    echo ""; \
	    echo "ERROR: $(CLANGXX) not found."; \
	    echo "Download wasi-sdk to $(WASI_SDK_PATH):"; \
	    echo "  https://github.com/WebAssembly/wasi-sdk/releases"; \
	    echo "  tar -xf wasi-sdk-*.tar.gz -C \$${HOME} --one-top-level=wasi-sdk"; \
	    echo ""; \
	    exit 1; \
	}
	@wasm-tools --version > /dev/null 2>&1 || { \
	    echo "ERROR: wasm-tools not found.  cargo install wasm-tools"; exit 1; \
	}
