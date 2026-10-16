# Parse batch threshold experiment — ascending vs descending

## Source data

Six independent runs, 100 measured calls and 10 warm-up calls per size:

- `threshold_ascending_run1_20260815_161115.csv`
- `threshold_ascending_run2_20260815_161116.csv`
- `threshold_ascending_run3_20260815_161118.csv`
- `threshold_descending_run1_20260815_161136.csv`
- `threshold_descending_run2_20260815_161137.csv`
- `threshold_descending_run3_20260815_161138.csv`

Every direction therefore contributes 300 measured samples per batch size.
Every record is exactly 1024 bytes. Times below are pooled medians of complete
batch calls.

## Ascending order

| Records / KiB | Copy-in | Guest | Copy-out | Total | Input throughput |
|---:|---:|---:|---:|---:|---:|
| 256 | 20.9 µs | 308.6 µs | 133.6 µs | 469.0 µs | 533.0 MiB/s |
| 288 | 23.8 µs | 346.4 µs | 151.4 µs | 524.7 µs | 536.0 MiB/s |
| 320 | 26.2 µs | 395.9 µs | 167.4 µs | 592.7 µs | 527.2 MiB/s |
| 384 | 34.2 µs | 466.5 µs | 202.1 µs | 705.7 µs | 531.4 MiB/s |
| 448 | 37.9 µs | 562.2 µs | 220.8 µs | 825.5 µs | 530.0 MiB/s |
| 512 | 53.8 µs | 608.5 µs | 624.6 µs | 1.293 ms | 386.5 MiB/s |
| 640 | 74.3 µs | 766.1 µs | 812.8 µs | 1.661 ms | 376.2 MiB/s |
| 768 | 89.7 µs | 929.6 µs | 992.3 µs | 2.023 ms | 370.7 MiB/s |

The ascending run has a sharp copy-out transition between 448 and 512 KiB.

## Descending order

| Records / KiB | Copy-in | Guest | Copy-out | Total | Input throughput |
|---:|---:|---:|---:|---:|---:|
| 256 | 20.8 µs | 303.3 µs | 136.9 µs | 463.6 µs | 539.3 MiB/s |
| 288 | 24.0 µs | 345.3 µs | 155.9 µs | 533.2 µs | 527.5 MiB/s |
| 320 | 26.1 µs | 390.1 µs | 171.8 µs | 591.8 µs | 528.1 MiB/s |
| 384 | 30.7 µs | 459.8 µs | 206.4 µs | 703.8 µs | 532.8 MiB/s |
| 448 | 37.6 µs | 540.9 µs | 240.6 µs | 827.8 µs | 528.5 MiB/s |
| 512 | 41.8 µs | 614.4 µs | 273.4 µs | 934.4 µs | 535.1 MiB/s |
| 640 | 62.7 µs | 767.1 µs | 670.8 µs | 1.506 ms | 415.1 MiB/s |
| 768 | 96.8 µs | 928.7 µs | 949.9 µs | 1.990 ms | 376.8 MiB/s |

When the instance is pre-grown by running 768 KiB first, 512 KiB retains the
same approximately 530 MiB/s plateau as the smaller sizes. The sharp transition
moves to between 512 and 640 KiB.

## Memory growth and order effect

No measured call triggered `memory.grow` in any of the six runs. All growth
happened during warm-up:

- Ascending: the store grew incrementally at each larger batch, reaching
  2,883,584 bytes at 768 records.
- Descending: the 768-record warm-up performed all 27 growth events and added
  1,769,472 bytes. Every later smaller size ran with the already-grown
  2,883,584-byte linear memory and needed no growth.

At 512 records:

| Direction | Copy-in | Guest | Copy-out | Total | Throughput |
|---|---:|---:|---:|---:|---:|
| Ascending | 53.8 µs | 608.5 µs | 624.6 µs | 1.293 ms | 386.5 MiB/s |
| Descending | 41.8 µs | 614.4 µs | 273.4 µs | 0.934 ms | 535.1 MiB/s |

Guest time changes by less than 1%, while descending copy-out is 56% lower.
This strongly localizes the 512-KiB cliff to Canonical ABI output
lowering/lifting and allocator/linear-memory state, not JSON parsing.

The fact that measured calls do not themselves grow memory means the latency
difference is not the direct cost of a `memory.grow` syscall during those
calls. Pre-growing changes subsequent allocation/layout behavior and shifts
the throughput cliff from 448–512 KiB to 512–640 KiB. Cache and allocator size
thresholds may still contribute.

## Scope warning

This standalone benchmark reuses one instance across all sizes. The production
parse path currently discards and replenishes its parse instance after each
batch. Production behavior therefore cannot be inferred directly from either
ordering. A fresh-instance benchmark (and later the copied production `src`)
is required to determine the operational threshold.
