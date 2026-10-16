# Vector deterministic burst/recovery experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, single TCP connection and HTTP worker.
Baseline is 11,000 input EPS; burst is 33,500 input EPS. Recovery requires 30 consecutive seconds
with five-second output rate within ±5% of baseline, p99 returned to its baseline threshold,
output backlog no more than one baseline second, and source pending no more than one baseline second.

| Scenario | Rep | Burst | Status | Recovery s | Confirm s | Max backlog | Max source pending | Base/Max p99 ms | Missing | CPU avg/max | MEM avg/max MiB |
|:--|--:|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|
| B1 | 1 | 5s | RECOVERED | 30.093 | 59.093 | 39,061 | 84,800 | 264.1 / 6682.1 | 0 | 73.10% / 100.10% | 37.70 / 56.92 |
| B2 | 1 | 15s | RECOVERED | 79.125 | 108.125 | 113,143 | 270,097 | 264.4 / 19249.0 | 0 | 75.20% / 100.30% | 38.70 / 57.67 |
| B3 | 1 | 30s | RECOVERED | 159.134 | 188.134 | 226,097 | 552,318 | 264.3 / 38518.0 | 0 | 82.03% / 100.40% | 40.63 / 56.76 |

## Aggregate

- B1: 1/1 recovered; mean recovery 30.09s (range 30.09–30.09s).
- B2: 1/1 recovered; mean recovery 79.12s (range 79.12–79.12s).
- B3: 1/1 recovered; mean recovery 159.13s (range 159.13–159.13s).
