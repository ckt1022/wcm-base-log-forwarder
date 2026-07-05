# ──────────────────────────────────────────────────────────────────
# C transport-http (transport-plugin) – WASM Component Model build
#
# 與 transport/ 對應，但使用 wasi:http/outgoing-handler 取代 raw socket：
# init() 驗證/儲存設定、send() 依 max-batch-bytes 分批送出、
# report-usage() 回傳累積送出位元組數。
#
# 支援 http:// 與 https://（TLS 由 host / hyper 管理）。
#
# Prerequisites:
#   wasi-sdk  ≥ 24  (set WASI_SDK_PATH, default ~/wasi-sdk)
#   wasm-tools      (cargo install wasm-tools)
#   wit-bindgen     (cargo install wit-bindgen-cli)
#
# Usage:
#   make                   # build transport_http.wasm (the component)
#   make clean
#   make bindings          # regenerate C bindings from WIT only
# ──────────────────────────────────────────────────────────────────

WASI_SDK_PATH ?= $(HOME)/wasi-sdk
CLANG    := $(WASI_SDK_PATH)/bin/clang
SYSROOT  := $(WASI_SDK_PATH)/share/wasi-sysroot
TARGET   := wasm32-wasip1

ADAPTER  ?= $(shell find ~/.cargo/registry -name "wasi_snapshot_preview1.reactor.wasm" 2>/dev/null | head -1)

CFLAGS   := --target=$(TARGET) --sysroot=$(SYSROOT) -O2 -I. -Wall

CORE_WASM  := transport_http_core.wasm
FINAL_WASM := transport_http.wasm

OBJS := transport_http_impl.o transport_plugin.o transport_plugin_component_type.o

.PHONY: all clean bindings check-tools

all: check-tools $(FINAL_WASM)

# ── Step 1a: 編譯 HTTP transport 邏輯 ─────────────────────────
transport_http_impl.o: transport_http_impl.c transport_plugin.h
	$(CLANG) $(CFLAGS) -c $< -o $@

# ── Step 1b: 編譯 generated C binding ──────────────────────────
transport_plugin.o: transport_plugin.c transport_plugin.h
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
	wit-bindgen c --world transport-plugin --out-dir . wit/
	@echo "Regenerated: transport_plugin.h  transport_plugin.c  transport_plugin_component_type.o"

clean:
	rm -f $(CORE_WASM) $(FINAL_WASM) transport_http_impl.o transport_plugin.o

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
