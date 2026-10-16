# Vector deterministic burst/recovery experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, single TCP connection and HTTP worker.
Baseline is 7,625 input EPS; burst is 22,875 input EPS. Recovery requires 30 consecutive seconds
with 10-second output rate within ±5% of baseline, p99 returned to its baseline threshold,
output backlog no more than one baseline second, and source pending no more than one baseline second.

| Scenario | Rep | Burst | Status | Recovery s | Confirm s | Max backlog | Max source pending | Base/Max p99 ms | Missing | CPU avg/max | MEM avg/max MiB |
|:--|--:|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|
| B1 | 1 | 5s | RECOVERED | 15.293 | 44.293 | 15,700 | 29,331 | 356.5 / 2830.2 | 0 | 51.04% / 100.10% | 86.41 / 104.73 |
| B2 | 1 | 15s | RECOVERED | 27.132 | 56.132 | 48,650 | 111,136 | 359.2 / 8526.4 | 0 | 53.23% / 100.10% | 37.33 / 56.95 |
| B3 | 1 | 30s | RECOVERED | 45.11 | 74.11 | 99,747 | 236,176 | 358.0 / 16974.4 | 0 | 54.66% / 100.40% | 37.99 / 56.12 |

## Aggregate

- B1: 1/1 recovered; mean recovery 15.29s (range 15.29–15.29s).
- B2: 1/1 recovered; mean recovery 27.13s (range 27.13–27.13s).
- B3: 1/1 recovered; mean recovery 45.11s (range 45.11–45.11s).
