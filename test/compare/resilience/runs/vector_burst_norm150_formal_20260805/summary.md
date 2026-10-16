# Vector deterministic burst/recovery experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, single TCP connection and HTTP worker.
Baseline is 11,000 input EPS; burst is 33,500 input EPS. Recovery requires 30 consecutive seconds
with five-second output rate within ±5% of baseline, p99 returned to its baseline threshold,
output backlog no more than one baseline second, and source pending no more than one baseline second.

| Scenario | Rep | Burst | Status | Recovery s | Confirm s | Max backlog | Max source pending | Base/Max p99 ms | Missing | CPU avg/max | MEM avg/max MiB |
|:--|--:|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|
| B1 | 1 | 5s | NOT_RECOVERED | None | None | 15,700 | 29,331 | 356.5 / 2830.2 | 0 | 51.04% / 100.10% | 86.41 / 104.73 |
| B2 | 1 | 15s | NOT_RECOVERED | None | None | 48,650 | 111,136 | 359.2 / 8526.4 | 0 | 53.23% / 100.10% | 37.33 / 56.95 |
| B3 | 1 | 30s | NOT_RECOVERED | None | None | 99,747 | 236,176 | 358.0 / 16974.4 | 0 | 54.66% / 100.40% | 37.99 / 56.12 |

## Aggregate

- B1: 0/1 recovered; mean recovery n/a.
- B2: 0/1 recovered; mean recovery n/a.
- B3: 0/1 recovered; mean recovery n/a.
