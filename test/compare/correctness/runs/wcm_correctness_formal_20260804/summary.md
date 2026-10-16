# WCM correctness experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, 1 s max wait, single TCP connection and HTTP worker.
C1: valid inputs at 2,500 EPS; C2: valid inputs at 17,500 EPS; C3: deterministic 5% malformed JSON;
C4: boolean, integer, float, null, Unicode, quote and backslash edge values. Each formal scenario was repeated three times.

| Scenario | Rep | Status | Expected | Received | Missing | Dup | Corrupt | Reorder | CPU avg/max | MEM avg/max MiB | Parse-error batches |
|:--|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| C1 | 1 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 14.45% / 17.70% | 21.32 / 28.25 | 0 |
| C1 | 2 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 13.45% / 16.20% | 25.97 / 29.94 | 0 |
| C1 | 3 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 14.93% / 23.30% | 25.34 / 29.22 | 0 |
| C2 | 1 | PASS | 400,000 | 400,000 | 0 | 0 | 0 | 0 | 76.41% / 83.80% | 27.98 / 30.86 | 0 |
| C2 | 2 | PASS | 400,000 | 400,000 | 0 | 0 | 0 | 0 | 81.86% / 88.00% | 26.19 / 29.96 | 0 |
| C2 | 3 | PASS | 400,000 | 400,000 | 0 | 0 | 0 | 0 | 77.31% / 82.90% | 27.63 / 30.50 | 0 |
| C3 | 1 | FAIL | 38,000 | 0 | 38,000 | 0 | 0 | 0 | 2.54% / 3.00% | 12.36 / 12.83 | 99 |
| C3 | 2 | FAIL | 38,000 | 0 | 38,000 | 0 | 0 | 0 | 2.54% / 3.00% | 12.43 / 12.98 | 99 |
| C3 | 3 | FAIL | 38,000 | 0 | 38,000 | 0 | 0 | 0 | 2.52% / 2.90% | 12.66 / 12.95 | 99 |

## Result

- C1: 3/3 runs passed; aggregate expected=120,000, received=120,000, missing=0.
- C2: 3/3 runs passed; aggregate expected=1,200,000, received=1,200,000, missing=0.
- C3: 0/3 runs passed; aggregate expected=114,000, received=0, missing=114,000.
- C3 logged 99 parse-error batches per run. The parser returns an error for the entire batch when any record is malformed, so every contaminated batch was discarded.
- No run was OOM-killed and the WCM container remained alive until the harness deliberately stopped it.
