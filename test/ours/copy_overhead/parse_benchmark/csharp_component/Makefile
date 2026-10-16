DOTNET ?= dotnet
WASM_TOOLS ?= wasm-tools
WASI_SDK_PATH ?= /opt/wasi-sdk
MSBUILD_PROPS ?= /p:IlcWasmGlobalBase=2097152

NATIVE_COMPONENT := bin/Release/net9.0/wasi-wasm/native/ParserBenchmark.wasm
TARGET := parser_csharp_benchmark.wasm

.PHONY: all publish component validate clean

all: component validate

publish: $(NATIVE_COMPONENT)

$(NATIVE_COMPONENT): ParserBenchmark.csproj Program.cs $(wildcard internal/*.cs) $(wildcard wit/*.wit)
	WASI_SDK_PATH=$(WASI_SDK_PATH) $(DOTNET) publish -c Release $(MSBUILD_PROPS)

component: $(TARGET)

$(TARGET): $(NATIVE_COMPONENT)
	cp $(NATIVE_COMPONENT) $(TARGET)

validate: $(TARGET)
	$(WASM_TOOLS) validate $(TARGET)

clean:
	$(DOTNET) clean
	rm -f $(TARGET)
