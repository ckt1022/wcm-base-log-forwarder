# WCM deterministic burst/recovery experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, single TCP connection and HTTP worker.
Baseline is 11,125 input EPS; burst is 33,375 input EPS.
Recovery is the first complete 30-second forward interval with total output rate within ±5% of baseline,
median per-second p99 returned to its baseline threshold, and end-of-interval output backlog and source pending
each no more than one baseline second.

| Scenario | Rep | Burst | Status | Recovery s | Confirm s | Max backlog | Max source pending | Base/Max p99 ms | Missing | CPU avg/max | MEM avg/max MiB |
|:--|--:|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|
| B3 | 1 | 300s | RECOVERED | 263.097 | 293.097 | 1,246,476 | 2,988,603 | 150.0 / 135624.3 | 0 | 66.76% / 102.80% | 101.99 / 159.56 |

## Aggregate

- B3: 1/1 recovered; mean recovery 263.10s (range 263.10–263.10s).
