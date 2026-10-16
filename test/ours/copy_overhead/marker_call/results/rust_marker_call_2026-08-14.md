# Rust marker-call result — 2026-08-14

## Final repeated experiment

Conditions:

- Rust component (`wasm32-wasip2`) and Rust Wasmtime host
- rustc 1.93.1
- Wasmtime 42.0.1
- Cranelift optimization: speed
- CPU affinity: core 2
- 500,000 callbacks per round
- 100 measured rounds after 10 warm-up rounds
- Three independent runs
- 50,000,000 calls per callback kind per run
- 150,000,000 calls per callback kind in the combined result

Raw result files:

- `rust_marker_call_20260814_232852.csv`
- `rust_marker_call_20260814_232904.csv`
- `rust_marker_call_20260814_232913.csv`

Combined summary across 300 measured rounds:

| Measurement | Mean | Median | p95 | Min | Max | Stddev |
|---|---:|---:|---:|---:|---:|---:|
| Empty component→host→component callback | 23.436 ns | 23.185 ns | 25.067 ns | 21.916 ns | 33.459 ns | 1.226 ns |
| Full timestamp callback (`Instant::now` + save) | 44.225 ns | 43.965 ns | 47.575 ns | 41.426 ns | 62.092 ns | 2.009 ns |
| Timestamp increment over empty callback | 20.789 ns | 20.621 ns | 24.367 ns | 14.234 ns | 33.428 ns | 1.873 ns |

Per-run full timestamp callback results:

| Run | Mean | Median | p95 |
|---|---:|---:|---:|
| `232852` | 44.755 ns | 44.098 ns | 48.158 ns |
| `232904` | 43.201 ns | 42.643 ns | 46.977 ns |
| `232913` | 44.720 ns | 44.273 ns | 46.750 ns |

## Interpretation

The approximately 23 ns p95 value is not the complete `time-sign()` cost. It
is the `clock_read_increment_ns` column: the extra cost of reading and saving
`Instant::now()` after subtracting the separately measured empty callback.

The relevant overhead introduced by one real `time-sign()` is the
`time_sign_ns_per_call` column. Across all three runs its combined mean is
44.225 ns, median is 43.965 ns, and p95 is 47.575 ns.

For later copy-in/copy-out experiments use:

- 44.225 ns as the average overhead of one marker.
- 47.575 ns as the conservative p95 overhead of one marker.
- 95.150 ns as the conservative combined overhead of two markers in one
  instrumented component call.

The earlier 100,000 × 30 pilot produced a 70.346 ns p95 due to several slow
rounds. It is superseded by these three longer repeated runs, which provide
substantially more samples and consistent per-run p95 values of 46.750–48.158
ns.
