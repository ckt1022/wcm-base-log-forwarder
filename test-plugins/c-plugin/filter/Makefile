# ──────────────────────────────────────────────────────────────────
# C filter (reduction-plugin) – WASM Component Model build
#
# Prerequisites:
#   wasi-sdk  ≥ 24  (set WASI_SDK_PATH, default ~/wasi-sdk)
#   wasm-tools      (cargo install wasm-tools)
#   wit-bindgen     (cargo install wit-bindgen-cli)
#
# Usage:
#   make                   # build latency_c_filter.wasm (the component)
#   make clean
#   make bindings          # regenerate C bindings from WIT only
# ──────────────────────────────────────────────────────────────────

WASI_SDK_PATH ?= $(HOME)/wasi-sdk
CLANG    := $(WASI_SDK_PATH)/bin/clang
SYSROOT  := $(WASI_SDK_PATH)/share/wasi-sysroot
TARGET   := wasm32-wasip1

# Reactor adapter: bundled in wasm-tools crate in cargo registry
ADAPTER  ?= $(shell find ~/.cargo/registry -name "wasi_snapshot_preview1.reactor.wasm" 2>/dev/null | head -1)

CFLAGS   := --target=$(TARGET) --sysroot=$(SYSROOT) -O2 -I. -Wall

CORE_WASM  := filter_core.wasm
FINAL_WASM := latency_c_filter.wasm

# 定義所有的物件檔案
OBJS := filter_impl.o reduction_plugin.o reduction_plugin_component_type.o

.PHONY: all clean bindings check-tools

all: check-tools $(FINAL_WASM)

# ── Step 1a: 編譯過濾邏輯 ───────────────────────────────────────
filter_impl.o: filter_impl.c reduction_plugin.h
	$(CLANG) $(CFLAGS) -c $< -o $@

# ── Step 1b: 編譯 generated C binding（必須維持 C，非 C++）────
reduction_plugin.o: reduction_plugin.c reduction_plugin.h
	$(CLANG) $(CFLAGS) -c $< -o $@

# ── Step 1c: 連結成 WASM core module ────────────────────────────
$(CORE_WASM): $(OBJS)
	$(CLANG) $(CFLAGS) \
	    -Wl,--no-entry \
	    -Wl,--export-dynamic \
	    $(OBJS) -o $@

# ── Step 2: 封裝成 WIT component ────────────────────────────────
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
	wit-bindgen c --world reduction-plugin --out-dir . wit/
	@echo "Regenerated: reduction_plugin.h  reduction_plugin.c  reduction_plugin_component_type.o"

clean:
	rm -f $(CORE_WASM) $(FINAL_WASM) filter_impl.o reduction_plugin.o

check-tools:
	@test -x "$(CLANG)" || { \
	    echo ""; \
	    echo "ERROR: $(CLANG) not found."; \
	    echo "Download wasi-sdk to $(WASI_SDK_PATH):"; \
	    echo "  https://github.com/WebAssembly/wasi-sdk/releases"; \
	    echo "  tar -xf wasi-sdk-*.tar.gz -C \$${HOME} --one-top-level=wasi-sdk"; \
	    echo ""; \
	    exit 1; \
	}
	@wasm-tools --version > /dev/null 2>&1 || { \
	    echo "ERROR: wasm-tools not found.  cargo install wasm-tools"; exit 1; \
	}
