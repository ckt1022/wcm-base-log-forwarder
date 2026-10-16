# WCM deterministic burst/recovery experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, single TCP connection and HTTP worker.
Baseline is 11,125 input EPS; burst is 33,375 input EPS.
Recovery is the first complete 30-second forward interval with total output rate within ±5% of baseline,
median per-second p99 returned to its baseline threshold, and end-of-interval output backlog and source pending
each no more than one baseline second.

| Scenario | Rep | Burst | Status | Recovery s | Confirm s | Max backlog | Max source pending | Base/Max p99 ms | Missing | CPU avg/max | MEM avg/max MiB |
|:--|--:|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|
| B3 | 1 | 300s | RECOVERED | 272.167 | 302.167 | 1,295,118 | 3,112,470 | 150.2 / 141279.1 | 0 | 67.11% / 102.90% | 146.37 / 179.82 |

## Aggregate

- B3: 1/1 recovered; mean recovery 272.17s (range 272.17–272.17s).
