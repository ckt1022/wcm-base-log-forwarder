# Production-src pipeline copy benchmark

This directory is an isolated copy of the repository `src`, `wit`,
`Cargo.toml`, and `Cargo.lock`. The original production files are not modified.

The copied benchmark pipeline uses one-shot instances for all three stages:
parse acquires an unused instance from its three-instance pool and replenishes
it after the call, while filter and format instantiate before H0 and discard
the instance immediately after one measured call.
Input comes from stdin only in benchmark mode and is split into exact record
counts by `WCM_BENCH_BATCH_RECORDS`.

The benchmark uses the instrumented copy of the production C JSON parser to
create the real `ParsedEntry -> LogEntry` path. Only the original
`plugin.call_filter()` is timed:

```text
H0 copied src immediately before call_filter
H1 filter component first time-sign
H2 filter component second time-sign
H3 copied src immediately after call_filter
```

Compile/instantiate, parse, channels, and host-side FilterResult application
are outside H0..H3. Lines prefixed with `FILTER_BENCH` are extracted into CSV.

Short test:

```bash
cd /home/ckt1022/wcm-base-log-forwarder/test/ours/copy_overhead/src_pipeline_benchmark
CPU=2 ./run_one.sh rust 64 5 2 > results/src_rust_short.csv
```

Arguments are `LANGUAGE BATCH ROUNDS WARMUP`; language is `rust`, `c`, or
`csharp`.

The copied source also instruments its original `plugin.call_format()` with
the same boundaries. Format components live in the sibling
`format_benchmark` directory and emit the production flat-JSON framing.

```bash
CPU=2 ./run_format_one.sh rust 64 5 2 > results/src_format_rust_short.csv
CPU=2 ./run_format_one.sh c 64 5 2 > results/src_format_c_short.csv
CPU=2 ./run_format_one.sh csharp 64 5 2 > results/src_format_csharp_short.csv
```

Run all ten batch sizes (64 through 32768), three languages, and three independent runs for one
stage. The default is 100 measured calls plus 10 warm-up calls per batch:

```bash
CPU=2 ./run_three.sh filter
CPU=2 ./run_three.sh format
```

Use `ROUNDS=<n>` and `WARMUP=<n>` to override those defaults. Each formal CSV
contains 1001 lines: one header and 10 x 100 samples. A zero-record call is not
part of the production pipeline and is therefore not synthesized here.

Run parse, filter, and format for Rust, C, and C# in one command. By default,
each stage/language combination is repeated three times, and the script waits
20 seconds between complete batch-sweep experiments:

```bash
CPU=2 ./run_all_stages.sh
```

For the Rust-only WCM comparison against the two Rust Core Wasm controls, use:

```bash
LANGUAGES=rust CPU=2 ./run_all_stages.sh
```

This creates a timestamped directory under `results/` containing 27 CSV files.
Parse uses the standalone fresh-pool host and includes its zero-record control;
filter and format use fresh one-shot instances in this copied production
pipeline. Override the defaults
with `STAGES`, `LANGUAGES`, `ROUNDS`, `WARMUP`, `RUNS`, or `INTERVAL`, for example:

```bash
ROUNDS=50 WARMUP=5 RUNS=3 INTERVAL=20 CPU=2 ./run_all_stages.sh
```
