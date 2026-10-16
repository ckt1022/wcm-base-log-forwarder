# Vector deterministic burst/recovery experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, single TCP connection and HTTP worker.
Baseline is 7,625 input EPS; burst is 22,875 input EPS.
Recovery is the first complete 30-second forward interval with total output rate within ±5% of baseline,
median per-second p99 returned to its baseline threshold, and end-of-interval output backlog and source pending
each no more than one baseline second.

| Scenario | Rep | Burst | Status | Recovery s | Confirm s | Max backlog | Max source pending | Base/Max p99 ms | Missing | CPU avg/max | MEM avg/max MiB |
|:--|--:|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|
| B1 | 1 | 5s | RECOVERED | 6.108 | 36.108 | 17,137 | 30,560 | 356.8 / 2993.2 | 0 | 52.06% / 100.10% | 86.28 / 105.25 |
| B2 | 1 | 15s | RECOVERED | 36.11 | 66.11 | 65,600 | 153,832 | 365.2 / 13642.7 | 0 | 65.69% / 102.10% | 37.93 / 57.01 |
| B3 | 1 | 30s | RECOVERED | 70.991 | 100.991 | 130,360 | 314,740 | 371.3 / 26823.5 | 0 | 68.45% / 101.40% | 38.63 / 56.56 |

## Aggregate

- B1: 1/1 recovered; mean recovery 6.11s (range 6.11–6.11s).
- B2: 1/1 recovered; mean recovery 36.11s (range 36.11–36.11s).
- B3: 1/1 recovered; mean recovery 70.99s (range 70.99–70.99s).
