WASI_SDK_PATH ?= /opt/wasi-sdk
CXX := $(WASI_SDK_PATH)/bin/wasm32-wasip2-clang++
CC := $(WASI_SDK_PATH)/bin/wasm32-wasip2-clang

CFLAGS := -O2 -DNDEBUG -Wall -Wextra -Igenerated
CXXFLAGS := $(CFLAGS) -fno-exceptions
LDFLAGS := -mexec-model=reactor -Wl,--export-dynamic

TARGET := format_c_benchmark.wasm
OBJECTS := main.o format_plugin.o generated/format_plugin_component_type.o

.PHONY: all bindings validate

all: $(TARGET) validate

bindings:
	mkdir -p generated
	wit-bindgen c --world format-plugin --out-dir generated ../wit

main.o: main.cpp generated/format_plugin.h
	$(CXX) $(CXXFLAGS) -c $< -o $@

format_plugin.o: generated/format_plugin.c generated/format_plugin.h
	$(CC) $(CFLAGS) -c generated/format_plugin.c -o $@

$(TARGET): $(OBJECTS)
	$(CXX) $(CXXFLAGS) $(LDFLAGS) $(OBJECTS) -o $@

validate: $(TARGET)
	wasm-tools validate $(TARGET)
