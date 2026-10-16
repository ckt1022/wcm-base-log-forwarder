# Rust Core Wasm JSON ABI benchmark

This is the Core WebAssembly control group for the WCM copy-overhead tests.
It uses a Rust Wasmtime core-module host and three Rust guests. Parse, filter,
and format are measured independently with fresh one-shot instances.

The four markers are:

```text
H0 host before JSON request serialization
H1 guest after JSON request deserialization
H2 guest after business result construction
H3 host after JSON response decoding and buffer deallocation
```

Thus `H0-H1` and `H2-H3` are JSON ABI boundary latency, not pure memory-copy
time. Format preserves the WCM length-prefixed bytes inside a Base64 JSON
response; Base64 work belongs to H2-H3.

Detailed design and interpretation rules are in `IMPLEMENTATION_GUIDE.md`.

## Build

```bash
cd /home/ckt1022/wcm-base-log-forwarder/test/ours/copy_overhead/core_wasm_json_benchmark
./scripts/build.sh
```

## Smoke test

```bash
CPU=2 ./scripts/run_smoke.sh
```

## One stage

Arguments are `STAGE START_BATCH END_BATCH ROUNDS WARMUP`:

```bash
CPU=2 ./scripts/run_one.sh parse 64 32768 20 3 > results/parse.csv
CPU=2 ./scripts/run_one.sh filter 64 32768 20 3 > results/filter.csv
CPU=2 ./scripts/run_one.sh format 64 32768 20 3 > results/format.csv
```

## Formal run

The default runs all three stages over ten power-of-two batch sizes (64 through
32768), three times, with 100 measured calls and 10 warm-ups per batch and a
20-second interval between stage/run sweeps. The benchmark memory limit is
512 MiB so Format 32768 can hold its transient JSON and Base64 representations:

```bash
CPU=2 ./scripts/run_all.sh
```

Override with `STAGES`, `ROUNDS`, `WARMUP`, `RUNS`, or `INTERVAL`.
