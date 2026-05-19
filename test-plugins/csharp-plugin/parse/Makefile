DOTNET      ?= dotnet
WASM_TOOLS  ?= wasm-tools
MSBUILD_PROPS ?= /p:IlcWasmGlobalBase=2097152

# NativeAOT 輸出的 wasi-wasm component（包含 core module 在內部）
NATIVE_COMPONENT := bin/Release/net9.0/wasi-wasm/native/ParserPlugin.wasm
# 中間產物目錄
BUILD_DIR   := .build
CORE_MODULE := $(BUILD_DIR)/core.wasm
EMBEDDED    := $(BUILD_DIR)/embedded.wasm
COMPONENT   := parser_csharp.wasm
NOOP_BUILD_DIR := .build-noop
NOOP_COMPONENT := parser_csharp_noop.wasm

# WASI preview1→preview2 reactor adapter
# 優先用環境變數 ADAPTER，再從 cargo registry 自動搜尋
ADAPTER ?= $(shell find ~/.cargo/registry -name "wasi_snapshot_preview1.reactor.wasm" 2>/dev/null | head -1)

.PHONY: all bindgen restore publish component noop clean

# ── 步驟 0: 重新產生 WIT 綁定（WIT 變更後執行）──────────────────────────────
bindgen:
	mkdir -p internal
	wit-bindgen csharp -w parser-plugin -r native-aot --generate-stub wit/ --out-dir internal/
	@if [ ! -f ParserPluginWorldExportsImpl.cs ]; then \
	    mv internal/ParserPluginWorldExportsImpl.cs .; \
	    echo "Stub moved to root: ParserPluginWorldExportsImpl.cs"; \
	else \
	    rm -f internal/ParserPluginWorldExportsImpl.cs; \
	    echo "Stub already exists in root, skipped overwrite."; \
	fi

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
#     → unbundle  取出 core module（plain module，包含 parse/report-usage/reset exports）
#     → embed     注入我們的 WIT world 型別
#     → new       用 WASI preview1 adapter 組成最終 component
component: publish
	@test -n "$(ADAPTER)" || { \
	  echo "ERROR: wasi_snapshot_preview1.reactor.wasm not found."; \
	  echo "  Set ADAPTER=/path/to/wasi_snapshot_preview1.reactor.wasm"; \
	  exit 1; }
	mkdir -p $(BUILD_DIR)
	# 取出 core module（module0 = 主程式，包含我們的 WIT exports）
	$(WASM_TOOLS) component unbundle --module-dir $(BUILD_DIR) $(NATIVE_COMPONENT) \
	    -o $(BUILD_DIR)/unbundled.wasm
	cp $(BUILD_DIR)/unbundled-module0.wasm $(CORE_MODULE)
	# 嵌入 WIT component type（告訴 wasm-tools 我們的 world 有哪些 exports）
	$(WASM_TOOLS) component embed --world parser-plugin wit/ $(CORE_MODULE) -o $(EMBEDDED)
	# 組成最終 component（加入 WASI preview1 adapter）
	$(WASM_TOOLS) component new $(EMBEDDED) \
	    --adapt "wasi_snapshot_preview1=$(ADAPTER)" \
	    -o $(COMPONENT)
	@echo ""
	@echo "✓  Component ready: $(COMPONENT)"
	@$(WASM_TOOLS) component wit $(COMPONENT) | grep -E "export"

# ── C# no-op parser：同 WIT world，但 parse 只接收 input 並回傳空 list ─────
# 用於 differential timing 的 C#-vs-C# copy-in baseline。
noop: restore
	WASI_SDK_PATH=$(WASI_SDK_PATH) $(DOTNET) publish -c Release $(MSBUILD_PROPS) /p:DefineConstants=NOOP_PARSER
	@test -n "$(ADAPTER)" || { \
	  echo "ERROR: wasi_snapshot_preview1.reactor.wasm not found."; \
	  echo "  Set ADAPTER=/path/to/wasi_snapshot_preview1.reactor.wasm"; \
	  exit 1; }
	mkdir -p $(NOOP_BUILD_DIR)
	$(WASM_TOOLS) component unbundle --module-dir $(NOOP_BUILD_DIR) $(NATIVE_COMPONENT) \
	    -o $(NOOP_BUILD_DIR)/unbundled.wasm
	cp $(NOOP_BUILD_DIR)/unbundled-module0.wasm $(NOOP_BUILD_DIR)/core.wasm
	$(WASM_TOOLS) component embed --world parser-plugin wit/ $(NOOP_BUILD_DIR)/core.wasm \
	    -o $(NOOP_BUILD_DIR)/embedded.wasm
	$(WASM_TOOLS) component new $(NOOP_BUILD_DIR)/embedded.wasm \
	    --adapt "wasi_snapshot_preview1=$(ADAPTER)" \
	    -o $(NOOP_COMPONENT)
	@echo ""
	@echo "✓  No-op component ready: $(NOOP_COMPONENT)"
	@$(WASM_TOOLS) component wit $(NOOP_COMPONENT) | grep -E "export"

all: component

clean:
	$(DOTNET) clean
	rm -rf $(BUILD_DIR) $(NOOP_BUILD_DIR) $(COMPONENT) $(NOOP_COMPONENT)
