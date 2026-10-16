# Vector deterministic burst/recovery experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, single TCP connection and HTTP worker.
Baseline is 7,625 input EPS; burst is 22,875 input EPS.
Recovery is the first complete 30-second forward interval with total output rate within ±5% of baseline,
median per-second p99 returned to its baseline threshold, and end-of-interval output backlog and source pending
each no more than one baseline second.

| Scenario | Rep | Burst | Status | Recovery s | Confirm s | Max backlog | Max source pending | Base/Max p99 ms | Missing | CPU avg/max | MEM avg/max MiB |
|:--|--:|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|
| B3 | 1 | 300s | RECOVERED | 379.079 | 409.079 | 1,060,645 | 2,639,906 | 352.8 / 181927.6 | 0 | 72.86% / 100.50% | 44.87 / 58.81 |

## Aggregate

- B3: 1/1 recovered; mean recovery 379.08s (range 379.08–379.08s).
