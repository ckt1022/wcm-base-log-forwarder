# WCM correctness experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, 1 s max wait, single TCP connection and HTTP worker.
C1: valid inputs at 2,500 EPS; C2: valid inputs at 17,500 EPS; C3: deterministic 5% malformed JSON;
C4: boolean, integer, float, null, Unicode, quote and backslash edge values. Each formal scenario was repeated three times.

| Scenario | Rep | Status | Expected | Received | Missing | Dup | Corrupt | Reorder | CPU avg/max | MEM avg/max MiB | Parse-error batches |
|:--|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| C4 | 1 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 13.79% / 16.90% | 25.21 / 29.19 | 0 |
| C4 | 2 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 13.59% / 17.00% | 25.53 / 28.77 | 0 |
| C4 | 3 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 13.76% / 18.40% | 25.25 / 28.26 | 0 |

## Result

- C4: 3/3 runs passed; aggregate expected=120,000, received=120,000, missing=0.
- No run was OOM-killed and the WCM container remained alive until the harness deliberately stopped it.
