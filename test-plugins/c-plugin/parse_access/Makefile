# ──────────────────────────────────────────────────────────────────
# C parse_access plugin – WASM Component Model build
#
# 目的：測試 WCM 能力模型是否能阻擋未授權的外部 IO 存取
#
# Prerequisites:
#   wasi-sdk  ≥ 24  (set WASI_SDK_PATH, default ~/wasi-sdk)
#   wasm-tools      (cargo install wasm-tools)
#
# Usage:
#   make           # → parser_c_access.wasm
#   make clean
#   make bindings  # 重新從 WIT 產生 C bindings
# ──────────────────────────────────────────────────────────────────

WASI_SDK_PATH ?= $(HOME)/wasi-sdk
CLANG    := $(WASI_SDK_PATH)/bin/clang
SYSROOT  := $(WASI_SDK_PATH)/share/wasi-sysroot
TARGET   := wasm32-wasip1

# Reactor adapter: bundled in wasm-tools crate in cargo registry
ADAPTER  ?= $(shell find ~/.cargo/registry -name "wasi_snapshot_preview1.reactor.wasm" 2>/dev/null | head -1)

CFLAGS   := --target=$(TARGET) --sysroot=$(SYSROOT) -O2 -I. -Wall

CORE_WASM  := parse_access_core.wasm
FINAL_WASM := parser_c_access.wasm

OBJS := main.o parser_plugin.o parser_plugin_component_type.o

.PHONY: all clean bindings check-tools

all: check-tools $(FINAL_WASM)

# ── Step 1a: 編譯主要邏輯 ────────────────────────────────────────
main.o: main.c parser_plugin.h
	$(CLANG) $(CFLAGS) -c $< -o $@

# ── Step 1b: 編譯 generated C binding ──────────────────────────
parser_plugin.o: parser_plugin.c parser_plugin.h
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
		echo "  cargo install wasm-tools"; \
		exit 1; \
	}
	wasm-tools component new $(CORE_WASM) \
		--adapt wasi_snapshot_preview1="$(ADAPTER)" \
		-o $(FINAL_WASM)
	@echo "Built: $(FINAL_WASM)"
	@wasm-tools validate $(FINAL_WASM) && echo "Validated OK"

# ── 重新產生 Bindings（WIT 更新時才需要）────────────────────────
bindings:
	wit-bindgen c --world parser-plugin --out-dir . wit/
	@echo "Regenerated: parser_plugin.h  parser_plugin.c  parser_plugin_component_type.o"

clean:
	rm -f $(CORE_WASM) $(FINAL_WASM) main.o parser_plugin.o

check-tools:
	@test -x "$(CLANG)" || { \
		echo ""; \
		echo "ERROR: $(CLANG) not found."; \
		echo "Download wasi-sdk to $(WASI_SDK_PATH):"; \
		echo "  https://github.com/WebAssembly/wasi-sdk/releases"; \
		echo ""; \
		exit 1; \
	}
	@wasm-tools --version > /dev/null 2>&1 || { \
		echo "ERROR: wasm-tools not found.  cargo install wasm-tools"; exit 1; \
	}
