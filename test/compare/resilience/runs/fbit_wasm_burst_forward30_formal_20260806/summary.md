# Fluent Bit WASM deterministic burst/recovery experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, single TCP connection and HTTP worker.
Baseline is 2,750 input EPS; burst is 8,250 input EPS.
Recovery is the first complete 30-second forward interval with total output rate within ±5% of baseline,
median per-second p99 returned to its baseline threshold, and end-of-interval output backlog and source pending
each no more than one baseline second.

| Scenario | Rep | Burst | Status | Recovery s | Confirm s | Max backlog | Max source pending | Base/Max p99 ms | Missing | CPU avg/max | MEM avg/max MiB |
|:--|--:|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|
| B1 | 1 | 5s | RECOVERED | 9.874 | 39.874 | 8,269 | 13,027 | 1014.5 / 4761.9 | 0 | 63.86% / 101.90% | 302.23 / 307.93 |
| B2 | 1 | 15s | RECOVERED | 22.075 | 52.075 | 21,116 | 44,008 | 1011.7 / 10742.6 | 0 | 55.48% / 100.80% | 281.46 / 287.44 |
| B3 | 1 | 30s | RECOVERED | 33.069 | 63.069 | 35,938 | 79,413 | 1011.2 / 16755.3 | 0 | 59.62% / 100.80% | 281.50 / 288.34 |

## Aggregate

- B1: 1/1 recovered; mean recovery 9.87s (range 9.87–9.87s).
- B2: 1/1 recovered; mean recovery 22.07s (range 22.07–22.07s).
- B3: 1/1 recovered; mean recovery 33.07s (range 33.07–33.07s).
