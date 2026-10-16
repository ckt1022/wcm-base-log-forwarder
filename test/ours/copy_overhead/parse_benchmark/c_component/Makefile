WASI_SDK_PATH ?= /opt/wasi-sdk
CC := $(WASI_SDK_PATH)/bin/wasm32-wasip2-clang
CJSON_DIR := $(abspath ../../../../../test-plugins/c-plugin/parse)

CFLAGS := -O2 -DNDEBUG -Wall -Wextra -Igenerated -I$(CJSON_DIR)
LDFLAGS := -mexec-model=reactor -Wl,--export-dynamic

TARGET := parser_c_benchmark.wasm
OBJECTS := main.o cJSON.o parser_plugin.o generated/parser_plugin_component_type.o

.PHONY: all bindings validate clean

all: $(TARGET) validate

bindings:
	mkdir -p generated
	wit-bindgen c --world parser-plugin --out-dir generated wit

main.o: main.c generated/parser_plugin.h
	$(CC) $(CFLAGS) -c $< -o $@

cJSON.o: $(CJSON_DIR)/cJSON.c $(CJSON_DIR)/cJSON.h
	$(CC) $(CFLAGS) -c $(CJSON_DIR)/cJSON.c -o $@

parser_plugin.o: generated/parser_plugin.c generated/parser_plugin.h
	$(CC) $(CFLAGS) -c generated/parser_plugin.c -o $@

$(TARGET): $(OBJECTS)
	$(CC) $(CFLAGS) $(LDFLAGS) $(OBJECTS) -o $@

validate: $(TARGET)
	wasm-tools validate $(TARGET)

clean:
	rm -f main.o cJSON.o parser_plugin.o $(TARGET)
