# Rust parse copy-in/copy-out benchmark

This standalone benchmark does not modify the production `src` directory. Its
parse input and output types match the production `parser-plugin` world in
`wit/log_plugin.wit`; the only benchmark-specific addition is the imported
`time-sign()` marker.

Every input line is a deterministic, valid 1024-byte JSON object. The default
batch sweep is:

```text
0, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768 records
```

This corresponds to 0 bytes and 64 KiB through 32 MiB of input. For each call:

```text
H0 host before call
H1 first time-sign, after input lowering
H2 second time-sign, after JSON parsing/output construction
H3 host after output lifting
```

The reported intervals are `H1-H0`, `H2-H1`, `H3-H2`, and `H3-H0`.

## Run

```bash
cd test/ours/copy_overhead/parse_benchmark
taskset -c 2 ./run.sh 64 8192 20 3 > results.csv
```

Arguments are `START_COUNT END_COUNT ROUNDS WARMUP_ROUNDS`. Batch count doubles
from start to end. The zero-record control is always included.

## Threshold sweep

To investigate the throughput transition between 256 and 512 KiB:

```bash
taskset -c 2 ./run_threshold.sh 100 10 > results/threshold.csv
```

The default threshold sizes are:

```text
256, 288, 320, 384, 448, 512, 640, 768 records
```

`run_threshold.sh` arguments are `ROUNDS WARMUP_ROUNDS [SIZE_LIST]`. A custom
ordered list can therefore be tested with:

```bash
./run_threshold.sh 100 10 '768,640,512,448,384,320,288,256'
```

The CSV additionally reports per-call Wasm `memory.grow` count/delta, the
store-lifetime peak linear-memory size, and the growth observed during warm-up
before each measured batch size.

## Fresh-instance pool aligned with `src`

The production parse pool discards each used instance and replenishes it. To
match that behavior, use:

```bash
taskset -c 2 ./run_threshold_fresh.sh 100 10 > results/threshold_fresh.csv
```

Fresh mode pre-instantiates three unused instances. Each call acquires one,
times only `call_parse()` (H0 through H3), discards it, and creates a replacement
outside the measured interval. Consequently every warm-up and measured sample
starts with fresh linear memory. The output CSV identifies this as
`instance_mode=fresh-pool` and `pool_size=3`.

## Copy-out threshold diagnosis

The short diagnostic keeps the production parse signature and builds four
component variants: full output, an empty result list, one empty record per
input, one record containing only the large message string, and a version with
that message truncated to 256 bytes. A final variant concatenates every message
into one output record to distinguish total bytes from per-record allocations.
It also splits `memory.grow` events into H0-H1, H1-H2, and H2-H3 stages.

```bash
taskset -c 2 ./run_copyout_diagnosis.sh 20 2
```

The default sizes are `384,448,512,640,768,896,1024,1280`. Each result is
written under `results/copyout_diag_<variant>_<timestamp>.csv`.

## C component

The C component uses the same WIT, 1024-byte JSON fixture, output values,
time-sign boundaries, Rust host, and fresh-instance pool as the Rust component.
It parses JSON with the project's existing cJSON 1.7.19 source and builds with
wit-bindgen C 0.58 plus wasi-sdk.

```bash
taskset -c 2 ./run_c.sh 64 8192 100 10 > results/c_parse.csv
taskset -c 2 ./run_c_threshold.sh 100 10 > results/c_parse_threshold.csv
```

The CSV identifies these samples with `component_language=c`.

## C# component

The C# component uses the same WIT, input fixture, output bytes, marker
boundaries, Rust host, and fresh-instance pool. It uses .NET 9 NativeAOT for
WASI and `System.Text.Json`.

```bash
taskset -c 2 ./run_csharp.sh 64 8192 100 10 > results/csharp_parse.csv
taskset -c 2 ./run_csharp_threshold.sh 100 10 > results/csharp_parse_threshold.csv
```

The CSV identifies these samples with `component_language=csharp`.

## Go component

The Go component uses the same WIT, input fixture, output payload, marker
boundaries, Rust host, and fresh-instance pool. It is compiled with Go 1.25 for
WASI Preview 1, adapted into a Preview 2 component, and uses Go's
`encoding/json` package. The generated bindings come from wit-bindgen Go 0.58.

```bash
taskset -c 2 ./run_go.sh 64 8192 100 10 > results/go_parse.csv
taskset -c 2 ./run_go_threshold.sh 100 10 > results/go_parse_threshold.csv
```

The CSV identifies these samples with `component_language=go`.
