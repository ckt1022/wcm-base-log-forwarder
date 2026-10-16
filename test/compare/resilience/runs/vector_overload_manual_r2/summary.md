# Vector deterministic burst/recovery experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, single TCP connection and HTTP worker.
Baseline is 7,625 input EPS; burst is 22,875 input EPS.
Recovery is the first complete 30-second forward interval with total output rate within ±5% of baseline,
median per-second p99 returned to its baseline threshold, and end-of-interval output backlog and source pending
each no more than one baseline second.

| Scenario | Rep | Burst | Status | Recovery s | Confirm s | Max backlog | Max source pending | Base/Max p99 ms | Missing | CPU avg/max | MEM avg/max MiB |
|:--|--:|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|
| B3 | 1 | 300s | RECOVERED | 351.072 | 381.072 | 982,545 | 2,443,801 | 354.0 / 168204.2 | 0 | 72.60% / 100.40% | 49.05 / 63.44 |

## Aggregate

- B3: 1/1 recovered; mean recovery 351.07s (range 351.07–351.07s).
