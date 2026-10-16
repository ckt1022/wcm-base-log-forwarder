# WCM deterministic burst/recovery experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, single TCP connection and HTTP worker.
Baseline is 11,125 input EPS; burst is 33,375 input EPS.
Recovery is the first complete 30-second forward interval with total output rate within ±5% of baseline,
median per-second p99 returned to its baseline threshold, and end-of-interval output backlog and source pending
each no more than one baseline second.

| Scenario | Rep | Burst | Status | Recovery s | Confirm s | Max backlog | Max source pending | Base/Max p99 ms | Missing | CPU avg/max | MEM avg/max MiB |
|:--|--:|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|
| B1 | 1 | 5s | RECOVERED | 6.194 | 36.194 | 25,532 | 33 | 152.5 / 3056.0 | 0 | 52.11% / 101.70% | 86.78 / 118.79 |
| B2 | 1 | 15s | RECOVERED | 15.133 | 45.133 | 68,238 | 44,449 | 150.9 / 7657.6 | 0 | 49.83% / 101.70% | 71.22 / 159.31 |
| B3 | 1 | 30s | RECOVERED | 53.086 | 83.086 | 172,026 | 301,937 | 191.8 / 22859.2 | 0 | 54.28% / 104.10% | 81.48 / 159.57 |

## Aggregate

- B1: 1/1 recovered; mean recovery 6.19s (range 6.19–6.19s).
- B2: 1/1 recovered; mean recovery 15.13s (range 15.13–15.13s).
- B3: 1/1 recovered; mean recovery 53.09s (range 53.09–53.09s).
