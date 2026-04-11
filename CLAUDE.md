# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

`wcm-base-log-forwarder` 這項專案是我想用來寫論文的。架構上是利用wasm component model，把log forwarder中的每個
stage都插件化，想利用wasm的安全性，提出一個具有市面上的大部分log forwarder功能、吞吐量等等指標不輸、更加彈性化的日誌轉發。所以目標上希望傳輸吞吐量高、CPU使用率低、MEM占比低，彼此權衡。
請你以你是架構設計者的角度給建議，對於plugin的修改也沒關係，重點是wit提供的介面要乾淨明確，好實作。

## Commands

### Build

```bash
cargo build           # debug
#不要用release進行測試
cargo build --release # optimized
```

### Run

```bash
go run tools/gen/main.go -rate 5000 -duration 120 | target/debug/wcm-base-log-forwarder
```

Log generator flags: `-rate <logs/sec>`, `-duration <seconds>`, `-mode <simple|complex|mixed|invalid>`

### Build WASM Plugins (TinyGo)

From within a plugin directory (e.g. `test-plugins/go-plugin/parser/`):

```bash
tinygo build -o <output>.wasm -target wasm ...
```

See `test-plugins/go-plugin/README.md` for the exact TinyGo + `wit-bindgen-go` workflow.

### Benchmark

```bash
bash tools/bench/run.sh
python3 tools/bench/analyze.py tools/bench/results/results_<timestamp>.csv
```

## Architecture

### Pipeline (threaded, channel-based)

```
stdin → [reader thread] → LineItem channel (cap 5000)
                              ↓
                        [parse thread]
                          batches lines → calls WASM parser plugin → LogEntry structs
                              ↓
                        ParsedBatch channel (cap 32)
                              ↓
                        [format loop – main thread]
                          chunked 300 entries → calls WASM format plugin → stdout
```

### Key Source Files

| File | Role |
|---|---|
| [src/main.rs](src/main.rs) | Entry point; wires pipeline stages |
| [src/app.rs](src/app.rs) | WASM runtime init, stdin reader, component instantiation |
| [src/runtime.rs](src/runtime.rs) | `parse_loop` and `format_loop` implementations |
| [src/config.rs](src/config.rs) | `BatchConfig` (memory limits, timeouts, batch sizes) |
| [src/output.rs](src/output.rs) | Stats/diagnostic printing |

### WIT Interface

- [wit/log_host.wit](wit/log_host.wit) — host-side type definitions (`LogEntry`, `LogLevel`, `ParseError`)
- [wit/log_plugin.wit](wit/log_plugin.wit) — plugin-side interface (7 plugin types: `parser`, `format`, `enricher`, `reduction`, `route`, `masking`, `transport`)

### Batching & Memory

- Batch flush triggers: accumulated size ≥ 30% of 256 MB limit, elapsed time ≥ 250 ms, line count ≥ 50 000, or EOF.
- Format stage chunks large batches into 300-entry slices to limit TinyGo GC pause impact.
- `MyLimiter` (in [src/app.rs](src/app.rs)) implements `ResourceLimiter` to enforce per-WASM-instance memory budgets.

### Active Plugins

| Plugin | Path | Language | Status |
|---|---|---|---|
| Parser | `test-plugins/go-plugin/parser/` | TinyGo | Working |
| Formatter | `test-plugins/go-plugin/format/` | TinyGo | Working |
| Enricher | `test-plugins/go-plugin/enrich/` | TinyGo | Stub |
| Transport | `test-plugins/go-plugin/transport/` | TinyGo | Stub |

### Performance Baseline
Parse: ~12 500 entries/sec (current bottleneck). Format: ~18 000 entries/sec.

## Research Context

This is a 5-phase, 30-week research project (see [RESEARCH_PLAN.md](RESEARCH_PLAN.md)) aiming to produce a paper comparing the WCM plugin model against Fluent Bit (WASM + Lua) and native Rust for log forwarding. Phases: (1) full pipeline construction, (2) engineering hardening, (3) experiment design, (4) quantitative testing, (5) paper writing.

## Gotchas（過去踩過的坑）
