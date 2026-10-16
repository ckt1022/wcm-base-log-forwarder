# WCM deterministic burst/recovery experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, single TCP connection and HTTP worker.
Baseline is 11,125 input EPS; burst is 33,375 input EPS.
Recovery is the first complete 30-second forward interval with total output rate within ±5% of baseline,
median per-second p99 returned to its baseline threshold, and end-of-interval output backlog and source pending
each no more than one baseline second.

| Scenario | Rep | Burst | Status | Recovery s | Confirm s | Max backlog | Max source pending | Base/Max p99 ms | Missing | CPU avg/max | MEM avg/max MiB |
|:--|--:|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|
| B3 | 1 | 300s | RECOVERED | 264.346 | 294.346 | 1,246,766 | 2,992,550 | 150.0 / 135924.5 | 0 | 66.99% / 102.70% | 122.33 / 179.72 |

## Aggregate

- B3: 1/1 recovered; mean recovery 264.35s (range 264.35–264.35s).
