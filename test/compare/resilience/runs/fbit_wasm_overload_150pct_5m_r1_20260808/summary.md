# Fluent Bit WASM deterministic burst/recovery experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, single TCP connection and HTTP worker.
Baseline is 2,750 input EPS; burst is 8,250 input EPS.
Recovery is the first complete 30-second forward interval with total output rate within ±5% of baseline,
median per-second p99 returned to its baseline threshold, and end-of-interval output backlog and source pending
each no more than one baseline second.

| Scenario | Rep | Burst | Status | Recovery s | Confirm s | Max backlog | Max source pending | Base/Max p99 ms | Missing | CPU avg/max | MEM avg/max MiB |
|:--|--:|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|
| B3 | 1 | 300s | RECOVERED | 656.866 | 686.866 | 453,144 | 1,124,852 | 1020.0 / 253593.2 | 0 | 86.44% / 103.30% | 304.85 / 309.38 |

## Aggregate

- B3: 1/1 recovered; mean recovery 656.87s (range 656.87–656.87s).
