WASI_SDK    ?= $(HOME)/wasi-sdk
CLANG       := $(WASI_SDK)/bin/clang
SYSROOT     := $(WASI_SDK)/share/wasi-sysroot
TARGET      := wasm32-wasip1
ADAPTER     := $(shell find $(HOME)/.cargo/registry/src -name "wasi_snapshot_preview1.reactor.wasm" 2>/dev/null | head -1)

CFLAGS := --target=$(TARGET) --sysroot=$(SYSROOT) -O2 -I. -Wall

CORE   := parse_bad_core.wasm
OUTPUT := parser_c_bad.wasm

.PHONY: all bindings clean

all: $(OUTPUT)

bindings:
	wit-bindgen c --world parser-plugin --out-dir . wit/
	@echo "Regenerated: parser_plugin.h  parser_plugin.c  parser_plugin_component_type.o"

main.o: main.c parser_plugin.h
	$(CLANG) $(CFLAGS) -c main.c -o main.o

parser_plugin.o: parser_plugin.c
	$(CLANG) $(CFLAGS) -c parser_plugin.c -o parser_plugin.o

$(CORE): main.o parser_plugin.o parser_plugin_component_type.o
	$(CLANG) $(CFLAGS) \
		-Wl,--no-entry \
		-Wl,--export-dynamic \
		main.o parser_plugin.o parser_plugin_component_type.o -o $(CORE)

$(OUTPUT): $(CORE)
	wasm-tools component new $(CORE) \
		--adapt wasi_snapshot_preview1="$(ADAPTER)" \
		-o $(OUTPUT)
	@echo "Built: $(OUTPUT)"
	@wasm-tools validate $(OUTPUT) && echo "Validated OK"

clean:
	rm -f $(CORE) $(OUTPUT) main.o parser_plugin.o
