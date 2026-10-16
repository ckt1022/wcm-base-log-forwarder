WASI_SDK_PATH ?= /opt/wasi-sdk
CC := $(WASI_SDK_PATH)/bin/wasm32-wasip2-clang

CFLAGS := -O2 -DNDEBUG -Wall -Wextra -Igenerated
LDFLAGS := -mexec-model=reactor -Wl,--export-dynamic

TARGET := filter_c_benchmark.wasm
OBJECTS := main.o reduction_plugin.o generated/reduction_plugin_component_type.o

.PHONY: all bindings validate

all: $(TARGET) validate

bindings:
	mkdir -p generated
	wit-bindgen c --world reduction-plugin --out-dir generated ../wit

main.o: main.c generated/reduction_plugin.h
	$(CC) $(CFLAGS) -c $< -o $@

reduction_plugin.o: generated/reduction_plugin.c generated/reduction_plugin.h
	$(CC) $(CFLAGS) -c generated/reduction_plugin.c -o $@

$(TARGET): $(OBJECTS)
	$(CC) $(CFLAGS) $(LDFLAGS) $(OBJECTS) -o $@

validate: $(TARGET)
	wasm-tools validate $(TARGET)
