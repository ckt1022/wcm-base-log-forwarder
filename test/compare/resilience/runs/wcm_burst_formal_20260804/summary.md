# WCM deterministic burst/recovery experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, single TCP connection and HTTP worker.
Baseline is 11,000 input EPS; burst is 33,500 input EPS. Recovery requires 30 consecutive seconds
with five-second output rate within ±5% of baseline, p99 returned to its baseline threshold,
output backlog no more than one baseline second, and source pending no more than one baseline second.

| Scenario | Rep | Burst | Status | Recovery s | Confirm s | Max backlog | Max source pending | Base/Max p99 ms | Missing | CPU avg/max | MEM avg/max MiB |
|:--|--:|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|
| B1 | 1 | 5s | RECOVERED | 29.155 | 58.155 | 22,892 | 29 | 152.6 / 2517.2 | 0 | 48.11% / 101.00% | 28.27 / 91.79 |
| B2 | 1 | 15s | RECOVERED | 38.821 | 67.821 | 72,698 | 56,038 | 154.0 / 8324.2 | 0 | 50.83% / 102.50% | 90.69 / 177.40 |
| B3 | 1 | 30s | RECOVERED | 40.017 | 69.017 | 153,712 | 256,267 | 154.3 / 17727.4 | 0 | 52.73% / 101.70% | 86.81 / 172.71 |

## Aggregate

- B1: 1/1 recovered; mean recovery 29.16s (range 29.16–29.16s).
- B2: 1/1 recovered; mean recovery 38.82s (range 38.82–38.82s).
- B3: 1/1 recovered; mean recovery 40.02s (range 40.02–40.02s).
