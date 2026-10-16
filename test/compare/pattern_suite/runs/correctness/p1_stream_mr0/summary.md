# WCM correctness experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, 1 s max wait, single TCP connection and HTTP worker.
C1: valid inputs at 2,500 EPS; C2: valid inputs at 17,500 EPS; C3: deterministic 5% malformed JSON;
C4: boolean, integer, float, null, Unicode, quote and backslash edge values. Each formal scenario was repeated three times.

| Scenario | Rep | Status | Expected | Received | Missing | Dup | Corrupt | Reorder | CPU avg/max | MEM avg/max MiB | Parse-error batches |
|:--|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| C3 | 1 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 16.40% / 17.50% | 13.62 / 13.97 | 0 |
| C3 | 2 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 18.36% / 19.20% | 13.31 / 13.85 | 0 |
| C3 | 3 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 18.21% / 19.30% | 13.70 / 14.23 | 0 |

## Result

- C3: 3/3 runs passed; aggregate expected=120,000, received=120,000, missing=0.
- C3 logged 0 parse-error batches per run. The parser returns an error for the entire batch when any record is malformed and isolation was off (legacy behaviour), so every contaminated batch was discarded.
- No run was OOM-killed and the WCM container remained alive until the harness deliberately stopped it.
