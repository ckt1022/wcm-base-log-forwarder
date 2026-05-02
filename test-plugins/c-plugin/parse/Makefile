# ──────────────────────────────────────────────────────────────────
# C parse plugin – WASM Component Model build (with cJSON)
# ──────────────────────────────────────────────────────────────────

WASI_SDK_PATH ?= $(HOME)/wasi-sdk
CLANG    := $(WASI_SDK_PATH)/bin/clang
SYSROOT  := $(WASI_SDK_PATH)/share/wasi-sysroot
TARGET   := wasm32-wasip1

# Reactor adapter
ADAPTER  ?= $(shell find ~/.cargo/registry -name "wasi_snapshot_preview1.reactor.wasm" 2>/dev/null | head -1)

CFLAGS   := --target=$(TARGET) --sysroot=$(SYSROOT) -O2 -I. -Wall

CORE_WASM  := parse_core.wasm
FINAL_WASM := parser_c_json.wasm

# 定義所有的物件檔案
OBJS := main.o cJSON.o parser_plugin.o parser_plugin_component_type.o

.PHONY: all clean bindings check-tools

all: check-tools $(FINAL_WASM)

# ── Step 1a: 編譯主要邏輯與 cJSON ───────────────────────────────
main.o: main.c parser_plugin.h cJSON.h
	$(CLANG) $(CFLAGS) -c $< -o $@

cJSON.o: cJSON.c cJSON.h
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
		echo "ERROR: wasi_snapshot_preview1.reactor.wasm not found."; \
		exit 1; \
	}
	wasm-tools component new $(CORE_WASM) \
		--adapt wasi_snapshot_preview1="$(ADAPTER)" \
		-o $(FINAL_WASM)
	@echo "Successfully built: $(FINAL_WASM)"
	@wasm-tools validate $(FINAL_WASM) && echo "Validated OK"

# ── 重新產生 Bindings ──────────────────────────────────────────
bindings:
	wit-bindgen c --world parser-plugin --out-dir . wit/
	@echo "Regenerated bindings from WIT."

clean:
	rm -f $(CORE_WASM) $(FINAL_WASM) $(OBJS)

check-tools:
	@test -x "$(CLANG)" || { echo "ERROR: wasi-sdk not found at $(WASI_SDK_PATH)"; exit 1; }
	@wasm-tools --version > /dev/null 2>&1 || { echo "ERROR: wasm-tools not found"; exit 1; }