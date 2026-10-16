DOTNET ?= dotnet
WASM_TOOLS ?= wasm-tools
WASI_SDK_PATH ?= /opt/wasi-sdk
MSBUILD_PROPS ?= /p:IlcWasmGlobalBase=2097152

NATIVE_COMPONENT := bin/Release/net9.0/wasi-wasm/native/FormatBenchmark.wasm
TARGET := format_csharp_benchmark.wasm

.PHONY: all validate

all: $(TARGET) validate

$(NATIVE_COMPONENT): FormatBenchmark.csproj Program.cs $(wildcard internal/*.cs) $(wildcard ../wit/*.wit)
	WASI_SDK_PATH=$(WASI_SDK_PATH) $(DOTNET) publish -c Release $(MSBUILD_PROPS)

$(TARGET): $(NATIVE_COMPONENT)
	cp $(NATIVE_COMPONENT) $(TARGET)

validate: $(TARGET)
	$(WASM_TOOLS) validate $(TARGET)
