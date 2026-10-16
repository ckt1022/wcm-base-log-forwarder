# Fluentd deterministic burst/recovery experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, single TCP connection and HTTP worker.
Baseline is 25,000 input EPS; burst is 75,000 input EPS.
Recovery is the first complete 30-second forward interval with total output rate within ±5% of baseline,
median per-second p99 returned to its baseline threshold, and end-of-interval output backlog and source pending
each no more than one baseline second.

| Scenario | Rep | Burst | Status | Recovery s | Confirm s | Max backlog | Max source pending | Base/Max p99 ms | Missing | CPU avg/max | MEM avg/max MiB |
|:--|--:|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|
| B1 | 1 | 5s | RECOVERED | 6.075 | 36.075 | 52,926 | 124,476 | 115.6 / 2806.5 | 0 | 52.57% / 100.20% | 129.27 / 171.69 |
| B2 | 1 | 15s | RECOVERED | 18.789 | 48.789 | 165,370 | 408,480 | 115.4 / 8925.7 | 0 | 55.65% / 100.50% | 142.32 / 192.97 |
| B3 | 1 | 30s | RECOVERED | 38.045 | 68.045 | 339,754 | 840,163 | 115.9 / 18101.7 | 0 | 60.22% / 100.40% | 136.90 / 178.90 |

## Aggregate

- B1: 1/1 recovered; mean recovery 6.08s (range 6.08–6.08s).
- B2: 1/1 recovered; mean recovery 18.79s (range 18.79–18.79s).
- B3: 1/1 recovered; mean recovery 38.05s (range 38.05–38.05s).
