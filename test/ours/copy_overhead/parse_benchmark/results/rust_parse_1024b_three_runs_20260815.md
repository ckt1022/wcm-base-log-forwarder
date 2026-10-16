# Rust parse copy benchmark — three-run result

## Source data

This is the authoritative result based on the user's three independent runs:

- `rust_parse_1024b_run1_20260815_000045.csv`
- `rust_parse_1024b_run2_20260815_000052.csv`
- `rust_parse_1024b_run3_20260815_000058.csv`

Each run contains 100 measured calls for every batch size after 10 warm-up
calls. The combined statistics pool 300 samples per batch size. Every input
record is exactly 1024 bytes. The component build matches the production Rust
parser settings: Rust edition 2024, wit-bindgen 0.59, and release
`opt-level = "s"`.

## Combined observed intervals

Values are per complete `parse(batch)` call, not per record.

| Records | Input | Copy-in median / p95 | Guest median / p95 | Copy-out median / p95 | Total median / p95 |
|---:|---:|---:|---:|---:|---:|
| 0 | 0 | 0.4 / 0.8 µs | 0.1 / 0.1 µs | 0.4 / 0.7 µs | 0.7 / 1.5 µs |
| 64 | 64 KiB | 6.9 / 9.4 µs | 75.8 / 117.2 µs | 35.8 / 53.2 µs | 118.5 / 172.0 µs |
| 128 | 128 KiB | 11.5 / 19.9 µs | 150.2 / 265.2 µs | 69.0 / 113.0 µs | 231.2 / 400.8 µs |
| 256 | 256 KiB | 20.5 / 32.4 µs | 294.9 / 358.8 µs | 135.0 / 177.0 µs | 456.1 / 569.6 µs |
| 512 | 512 KiB | 43.0 / 95.1 µs | 596.5 / 723.0 µs | 602.6 / 732.8 µs | 1.247 / 1.505 ms |
| 1024 | 1 MiB | 106.4 / 218.1 µs | 1.208 / 1.448 ms | 1.303 / 1.560 ms | 2.641 / 3.140 ms |
| 2048 | 2 MiB | 313.7 / 600.5 µs | 2.420 / 2.816 ms | 2.841 / 3.400 ms | 5.600 / 6.546 ms |
| 4096 | 4 MiB | 873.8 µs / 1.281 ms | 4.852 / 5.388 ms | 6.203 / 7.141 ms | 12.014 / 13.238 ms |
| 8192 | 8 MiB | 1.977 / 2.459 ms | 9.797 / 10.932 ms | 13.370 / 14.988 ms | 25.249 / 27.534 ms |

`Total median` is calculated directly from the total-time samples. It is not
the arithmetic sum of the three independently selected medians.

## Empty-call-adjusted median and marker impact

The combined zero-record median is 400 ns on both the copy-in and copy-out
sides. The table subtracts these fixed intervals. Marker percentage uses the
previously measured 47.575 ns p95 cost of one complete `time-sign()` callback,
making it a conservative comparison for each boundary interval.

| Records | Adjusted copy-in | Marker / copy-in | Adjusted copy-out | Marker / copy-out | Copy-in + out |
|---:|---:|---:|---:|---:|---:|
| 64 | 6.5 µs | 0.732% | 35.4 µs | 0.134% | 41.9 µs |
| 128 | 11.1 µs | 0.429% | 68.6 µs | 0.069% | 79.7 µs |
| 256 | 20.1 µs | 0.237% | 134.6 µs | 0.035% | 154.7 µs |
| 512 | 42.6 µs | 0.112% | 602.2 µs | 0.008% | 644.8 µs |
| 1024 | 106.0 µs | 0.045% | 1.302 ms | 0.004% | 1.408 ms |
| 2048 | 313.3 µs | 0.015% | 2.841 ms | 0.002% | 3.154 ms |
| 4096 | 873.4 µs | 0.005% | 6.203 ms | <0.001% | 7.076 ms |
| 8192 | 1.977 ms | 0.002% | 13.369 ms | <0.001% | 15.346 ms |

At the smallest non-empty batch, two markers have a conservative combined
cost of 95.15 ns, which is approximately 0.227% of the adjusted 41.9 µs
copy-in-plus-copy-out interval. Marker correction can therefore be omitted for
all tested non-empty batch sizes while retaining 47.575 ns per marker as the
documented systematic error bound.

## Between-run variation

Copy-out medians are consistent across the three runs. Copy-in medians are
also close at 64–256 and 4096–8192 records, but run 1 is slower at the
512–2048 sizes:

| Records | Run 1 copy-in | Run 2 copy-in | Run 3 copy-in |
|---:|---:|---:|---:|
| 512 | 53.7 µs | 40.6 µs | 39.7 µs |
| 1024 | 156.6 µs | 81.3 µs | 91.0 µs |
| 2048 | 427.5 µs | 258.3 µs | 289.1 µs |

This suggests an allocator, memory-growth, cache, or system-state threshold
effect around the middle batch sizes. It should be investigated separately if
the goal is to model copy-in scaling precisely. It does not threaten the
marker-overhead conclusion because even the fastest observed medians remain
orders of magnitude above 47.575 ns.
