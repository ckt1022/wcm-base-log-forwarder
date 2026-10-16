# Fluentd deterministic burst/recovery experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, single TCP connection and HTTP worker.
Baseline is 25,000 input EPS; burst is 75,000 input EPS.
Recovery is the first complete 30-second forward interval with total output rate within ±5% of baseline,
median per-second p99 returned to its baseline threshold, and end-of-interval output backlog and source pending
each no more than one baseline second.

| Scenario | Rep | Burst | Status | Recovery s | Confirm s | Max backlog | Max source pending | Base/Max p99 ms | Missing | CPU avg/max | MEM avg/max MiB |
|:--|--:|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|
| B3 | 1 | 300s | RECOVERED | 305.042 | 335.042 | 3,029,556 | 7,564,835 | 115.7 / 151604.9 | 0 | 70.65% / 100.40% | 157.50 / 198.59 |

## Aggregate

- B3: 1/1 recovered; mean recovery 305.04s (range 305.04–305.04s).
