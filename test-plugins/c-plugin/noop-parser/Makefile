# ──────────────────────────────────────────────────────────────────
# C no-op parse plugin – input-only differential benchmark component
# ──────────────────────────────────────────────────────────────────

WASI_SDK_PATH ?= $(HOME)/wasi-sdk
CLANG    := $(WASI_SDK_PATH)/bin/clang
SYSROOT  := $(WASI_SDK_PATH)/share/wasi-sysroot
TARGET   := wasm32-wasip1

ADAPTER  ?= $(shell find ~/.cargo/registry -name "wasi_snapshot_preview1.reactor.wasm" 2>/dev/null | head -1)

CFLAGS   := --target=$(TARGET) --sysroot=$(SYSROOT) -O2 -I../parse -Wall

CORE_WASM  := noop_core.wasm
FINAL_WASM := parser_c_noop.wasm

OBJS := main.o parser_plugin.o parser_plugin_component_type.o

.PHONY: all clean check-tools

all: check-tools $(FINAL_WASM)

main.o: main.c ../parse/parser_plugin.h
	$(CLANG) $(CFLAGS) -c $< -o $@

parser_plugin.o: ../parse/parser_plugin.c ../parse/parser_plugin.h
	$(CLANG) $(CFLAGS) -c $< -o $@

parser_plugin_component_type.o: ../parse/parser_plugin_component_type.o
	cp $< $@

$(CORE_WASM): $(OBJS)
	$(CLANG) $(CFLAGS) \
		-Wl,--no-entry \
		-Wl,--export-dynamic \
		$(OBJS) -o $@

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

clean:
	rm -f $(CORE_WASM) $(FINAL_WASM) $(OBJS)

check-tools:
	@test -x "$(CLANG)" || { echo "ERROR: wasi-sdk not found at $(WASI_SDK_PATH)"; exit 1; }
	@wasm-tools --version > /dev/null 2>&1 || { echo "ERROR: wasm-tools not found"; exit 1; }
