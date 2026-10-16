SHELL := /bin/bash

TOOLS_PATH := /usr/local/go/bin:/home/ckt1022/.cargo/bin:/usr/local/bin:/usr/bin:/bin
GO := PATH=$(TOOLS_PATH) GOCACHE=/tmp/wcm-go-build-cache GOOS=wasip1 GOARCH=wasm go
WASM_TOOLS := /home/ckt1022/.cargo/bin/wasm-tools
CORE := parser_go_core.wasm
EMBEDDED := parser_go_embedded.wasm
COMPONENT := parser_go_benchmark.wasm
ADAPTER := wasi_snapshot_preview1.reactor.wasm
SOURCES := $(shell find . -type f \( -name '*.go' -o -name '*.s' -o -name '*.wit' \)) go.mod go.sum Makefile

.PHONY: all check

all: $(COMPONENT)

$(CORE): $(SOURCES)
	$(GO) build -buildmode=c-shared -trimpath \
		-ldflags='-s -w -checklinkname=0' -o $@ .

$(EMBEDDED): $(CORE) $(wildcard wit/*.wit)
	$(WASM_TOOLS) component embed ./wit $(CORE) --world parser-plugin -o $@

$(COMPONENT): $(EMBEDDED) $(ADAPTER)
	$(WASM_TOOLS) component new $(EMBEDDED) --adapt $(ADAPTER) -o $@

check: $(COMPONENT)
	$(WASM_TOOLS) validate --features component-model $(COMPONENT)
	$(WASM_TOOLS) component wit $(COMPONENT)
