DOTNET      ?= dotnet
WASM_TOOLS  ?= wasm-tools
MSBUILD_PROPS ?= /p:IlcWasmGlobalBase=2097152

NATIVE_COMPONENT := bin/Release/net9.0/wasi-wasm/native/ReductionPlugin.wasm
BUILD_DIR   := .build
CORE_MODULE := $(BUILD_DIR)/core.wasm
EMBEDDED    := $(BUILD_DIR)/embedded.wasm
COMPONENT   := filter_csharp.wasm

ADAPTER ?= $(shell find ~/.cargo/registry -name "wasi_snapshot_preview1.reactor.wasm" 2>/dev/null | head -1)
WASI_SDK_PATH ?= /opt/wasi-sdk

.PHONY: all bindgen restore publish component clean

# ── 步驟 0: 重新產生 WIT 綁定 ─────────────────────────────────────────────
bindgen:
	mkdir -p internal
	wit-bindgen csharp -w reduction-plugin -r native-aot --generate-stub wit/ --out-dir internal/
	@if [ ! -f ReductionPluginWorldExportsImpl.cs ]; then \
	    mv internal/ReductionPluginWorldExportsImpl.cs .; \
	    echo "Stub moved to root: ReductionPluginWorldExportsImpl.cs"; \
	else \
	    rm -f internal/ReductionPluginWorldExportsImpl.cs; \
	    echo "Stub already exists in root, skipped overwrite."; \
	fi

# ── 步驟 1: 還原套件 ──────────────────────────────────────────────────────
restore:
	$(DOTNET) restore

# ── 步驟 2: 編譯 ──────────────────────────────────────────────────────────
publish: restore
	WASI_SDK_PATH=$(WASI_SDK_PATH) $(DOTNET) publish -c Release $(MSBUILD_PROPS)

# ── 步驟 3: 包裝成 WASM Component ────────────────────────────────────────
component: publish
	@test -n "$(ADAPTER)" || { \
	  echo "ERROR: wasi_snapshot_preview1.reactor.wasm not found."; \
	  echo "  Set ADAPTER=/path/to/wasi_snapshot_preview1.reactor.wasm"; \
	  exit 1; }
	mkdir -p $(BUILD_DIR)
	$(WASM_TOOLS) component unbundle --module-dir $(BUILD_DIR) $(NATIVE_COMPONENT) \
	    -o $(BUILD_DIR)/unbundled.wasm
	cp $(BUILD_DIR)/unbundled-module0.wasm $(CORE_MODULE)
	$(WASM_TOOLS) component embed --world reduction-plugin wit/ $(CORE_MODULE) -o $(EMBEDDED)
	$(WASM_TOOLS) component new $(EMBEDDED) \
	    --adapt "wasi_snapshot_preview1=$(ADAPTER)" \
	    -o $(COMPONENT)
	@echo ""
	@echo "✓  Component ready: $(COMPONENT)"
	@$(WASM_TOOLS) component wit $(COMPONENT) | grep -E "export"

all: component

clean:
	$(DOTNET) clean
	rm -rf $(BUILD_DIR) $(COMPONENT)
