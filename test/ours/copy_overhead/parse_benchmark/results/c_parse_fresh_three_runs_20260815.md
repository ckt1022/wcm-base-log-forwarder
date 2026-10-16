# C parse component — fresh-pool three-run result

## Setup

- Rust Wasmtime host, C component
- Same instrumented parser WIT and H0/H1/H2/H3 boundaries as Rust
- cJSON 1.7.19
- wit-bindgen C 0.58.0
- wasi-sdk clang 19.1.5, `-O2`
- 1024 bytes per JSON record
- Fresh instance pool of 3, discard and replenish after every call
- 3 independent runs, 100 measured calls and 10 warm-ups per batch per run
- 300 combined samples per non-empty batch

The output values and `output_string_bytes` exactly match the Rust component.
The `lang` tag deliberately remains `Rust` in this controlled component so the
returned byte count is identical; the CSV identifies the implementation using
`component_language=c`.

## Empty-call-adjusted combined medians

The combined C empty-call medians are 8.5 us copy-in, 2.5 us guest, 1.5 us
copy-out, and 12.5 us total. The following table subtracts those values.

| Records | Input | Copy-in | cJSON + output construction | Copy-out | Total | Throughput |
|---:|---:|---:|---:|---:|---:|---:|
| 64 | 64 KiB | 48.9 us | 230.6 us | 26.6 us | 307.9 us | 203.0 MiB/s |
| 128 | 128 KiB | 92.3 us | 468.3 us | 52.6 us | 619.1 us | 201.9 MiB/s |
| 256 | 256 KiB | 192.8 us | 952.7 us | 103.8 us | 1.261 ms | 198.3 MiB/s |
| 512 | 512 KiB | 378.2 us | 1.892 ms | 204.9 us | 2.488 ms | 201.0 MiB/s |
| 1024 | 1 MiB | 883.7 us | 3.863 ms | 437.6 us | 5.248 ms | 190.5 MiB/s |
| 2048 | 2 MiB | 1.784 ms | 7.925 ms | 2.819 ms | 12.779 ms | 156.5 MiB/s |
| 4096 | 4 MiB | 3.757 ms | 15.982 ms | 6.105 ms | 26.116 ms | 153.2 MiB/s |
| 8192 | 8 MiB | 7.562 ms | 31.969 ms | 12.669 ms | 52.236 ms | 153.2 MiB/s |

Total is the direct median of H3-H0 and is not required to equal the sum of
the independently selected stage medians.

## Immediate observations

- C copy-in is close to Rust at most sizes because the same Rust host and WIT
  input type perform the lifting/lowering.
- cJSON plus C output construction is about 1.8-1.9x the measured Rust guest
  interval for this fixed JSON shape.
- C copy-out remains approximately linear through 1024 records, then jumps at
  2048 records. Rust showed its corresponding discontinuity earlier, between
  512 and 1024 records.
- C therefore provides direct evidence that the exact transition point is not
  a fixed Component Model limit; component allocator and output-memory layout
  move the working-set transition.
- Every C H2-H3 sample recorded zero `memory.grow` calls, matching the Rust
  diagnosis that the output discontinuity is not caused by output-stage linear
  memory growth.

## Source CSVs

- `c_parse_fresh_ascending_run1_20260815.csv`
- `c_parse_fresh_ascending_run2_20260815.csv`
- `c_parse_fresh_ascending_run3_20260815.csv`

