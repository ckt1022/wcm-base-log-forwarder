DOTNET        ?= dotnet
WASM_TOOLS    ?= wasm-tools
MSBUILD_PROPS ?= /p:IlcWasmGlobalBase=2097152
WASI_SDK_PATH ?= $(HOME)/wasi-sdk

# NativeAOT 輸出的 wasi-wasm component（包含 core module 在內部）
NATIVE_COMPONENT := bin/Release/net9.0/wasi-wasm/native/ParserPlugin.wasm
# 中間產物目錄
BUILD_DIR   := .build
CORE_MODULE := $(BUILD_DIR)/core.wasm
EMBEDDED    := $(BUILD_DIR)/embedded.wasm
COMPONENT   := parser_csharp_loop.wasm

# WASI preview1→preview2 reactor adapter
# 優先用環境變數 ADAPTER，再從 cargo registry 自動搜尋
# 或使用同目錄下的 wasi_snapshot_preview1.reactor.wasm
ADAPTER ?= $(shell find ~/.cargo/registry -name "wasi_snapshot_preview1.reactor.wasm" 2>/dev/null | head -1)
ADAPTER := $(if $(ADAPTER),$(ADAPTER),$(dir $(abspath $(lastword $(MAKEFILE_LIST))))wasi_snapshot_preview1.reactor.wasm)

.PHONY: all bindgen restore publish component clean

all: component

# ── 步驟 0: 重新產生 WIT 綁定（WIT 變更後執行）──────────────────────────────
bindgen:
	mkdir -p internal
	wit-bindgen csharp -w parser-plugin -r native-aot --generate-stub wit/ --out-dir internal/
	@rm -f internal/ParserPluginWorldExportsImpl.cs
	@echo "Bindings regenerated. Root ParserPluginWorldExportsImpl.cs preserved."

# ── 步驟 1: 還原套件 ─────────────────────────────────────────────────────────
restore:
	$(DOTNET) restore

# ── 步驟 2: 編譯（NativeAOT LLVM → wasi-wasm component）─────────────────────
# 輸出: bin/Release/net9.0/wasi-wasm/native/ParserPlugin.wasm
# 注意: NativeAOT 直接輸出 component 格式（0d 00 01 00 magic），非 plain module
publish: restore
	WASI_SDK_PATH=$(WASI_SDK_PATH) $(DOTNET) publish -c Release $(MSBUILD_PROPS)

# ── 步驟 3: 包裝成含正確 WIT world 的 WASM Component ────────────────────────
# 流程:
#   NativeAOT component
#     → unbundle  取出 core module（plain module，包含 parse/report-usage exports）
#     → embed     注入我們的 WIT world 型別
#     → new       用 WASI preview1 adapter 組成最終 component
component: publish
	@test -f "$(ADAPTER)" || { \
	  echo "ERROR: wasi_snapshot_preview1.reactor.wasm not found."; \
	  echo "  Set ADAPTER=/path/to/wasi_snapshot_preview1.reactor.wasm"; \
	  exit 1; }
	mkdir -p $(BUILD_DIR)
	$(WASM_TOOLS) component unbundle --module-dir $(BUILD_DIR) $(NATIVE_COMPONENT) \
	    -o $(BUILD_DIR)/unbundled.wasm
	cp $(BUILD_DIR)/unbundled-module0.wasm $(CORE_MODULE)
	$(WASM_TOOLS) component embed --world parser-plugin wit/ $(CORE_MODULE) -o $(EMBEDDED)
	$(WASM_TOOLS) component new $(EMBEDDED) \
	    --adapt "wasi_snapshot_preview1=$(ADAPTER)" \
	    -o $(COMPONENT)
	@echo ""
	@echo "✓  Component ready: $(COMPONENT)"
	@$(WASM_TOOLS) component wit $(COMPONENT) | grep -E "export"

clean:
	$(DOTNET) clean
	rm -rf $(BUILD_DIR) $(COMPONENT)
