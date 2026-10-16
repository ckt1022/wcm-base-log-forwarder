# Rust parse copy benchmark — 1024-byte JSON

> Superseded by `rust_parse_1024b_three_runs_20260815.md`, which combines the
> user's three independent runs. This file is retained as a single-run record.

## Conditions

- Standalone Rust Wasmtime host and Rust `wasm32-wasip2` component
- Wasmtime 42.0.1, Cranelift `speed`
- Component build aligned with the production Rust parser: Rust edition 2024,
  wit-bindgen 0.59, release `opt-level = "s"`
- CPU affinity: core 2
- One reused component instance
- One deterministic valid JSON object per record, exactly 1024 bytes
- Batch count: 0, 64, 128, 256, 512, 1024, 2048, 4096, 8192
- 100 measured rounds after 10 warm-up rounds for every batch size
- Marker reference: 47.575 ns p95 per complete `time-sign()` callback

The final raw samples are in
`rust_parse_1024b_production_build_20260814.csv`. The earlier
`rust_parse_1024b_100rounds_20260814.csv` used `opt-level = 3` and is retained
only as a preliminary comparison; it is superseded by the production-aligned
build.

## Median result

300 ns copy-out-side fixed interval. `Adjusted` values below subtract those
The zero-record control measured a 300 ns copy-in-side fixed interval and a
300 ns copy-out-side fixed interval. `Adjusted` values below subtract those
300 ns copy-out-side fixed interval. `Adjusted` values below subtract those
empty-call medians. Marker percentages use the adjusted values, which is more
conservative than comparing against the unadjusted observed intervals.

| Records | Input | Copy-in observed | Copy-in adjusted | Marker / adjusted copy-in | Copy-out observed | Copy-out adjusted | Marker / adjusted copy-out |
|---:|---:|---:|---:|---:|---:|---:|---:|
| 64 | 64 KiB | 6.9 µs | 6.6 µs | 0.721% | 35.6 µs | 35.3 µs | 0.135% |
| 128 | 128 KiB | 11.7 µs | 11.4 µs | 0.417% | 68.5 µs | 68.2 µs | 0.070% |
| 256 | 256 KiB | 20.4 µs | 20.1 µs | 0.237% | 135.1 µs | 134.8 µs | 0.035% |
| 512 | 512 KiB | 39.8 µs | 39.5 µs | 0.120% | 608.6 µs | 608.3 µs | 0.008% |
| 1024 | 1 MiB | 135.5 µs | 135.2 µs | 0.035% | 1.340 ms | 1.339 ms | 0.004% |
| 2048 | 2 MiB | 370.9 µs | 370.6 µs | 0.013% | 3.114 ms | 3.114 ms | 0.002% |
| 4096 | 4 MiB | 936.5 µs | 936.2 µs | 0.005% | 6.924 ms | 6.923 ms | 0.001% |
| 8192 | 8 MiB | 1.962 ms | 1.961 ms | 0.002% | 14.297 ms | 14.297 ms | <0.001% |

## Conclusion

At the smallest non-empty batch (64 records, 64 KiB), the conservative p95
marker cost is 0.721% of adjusted copy-in and 0.135% of adjusted copy-out. Both
are below 1%. The ratios decrease further as the batch grows.

For this selected sweep, the two `time-sign()` callbacks are small enough to
ignore when reporting copy-in and copy-out at microsecond precision. Keep
47.575 ns per marker as the documented systematic upper-bound estimate. This
conclusion does not apply to the zero-record control or to future batches below
64 records without measuring them separately.

The reported intervals are Canonical ABI/binding intervals, not pure `memcpy`:
copy-in includes input lowering and Rust guest binding work; copy-out includes
guest result lowering, host lifting, nested record/string handling, and
post-return cleanup.
