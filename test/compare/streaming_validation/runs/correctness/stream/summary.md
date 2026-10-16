# WCM correctness experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, 1 s max wait, single TCP connection and HTTP worker.
C1: valid inputs at 2,500 EPS; C2: valid inputs at 17,500 EPS; C3: deterministic 5% malformed JSON;
C4: boolean, integer, float, null, Unicode, quote and backslash edge values. Each formal scenario was repeated three times.

| Scenario | Rep | Status | Expected | Received | Missing | Dup | Corrupt | Reorder | CPU avg/max | MEM avg/max MiB | Parse-error batches |
|:--|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| C1 | 1 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 15.82% / 17.20% | 19.96 / 20.36 | 0 |
| C1 | 2 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 15.34% / 16.60% | 17.24 / 17.68 | 0 |
| C1 | 3 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 15.62% / 17.00% | 17.04 / 17.54 | 0 |
| C2 | 1 | PASS | 400,000 | 400,000 | 0 | 0 | 0 | 0 | 83.54% / 87.60% | 17.55 / 18.09 | 0 |
| C2 | 2 | PASS | 400,000 | 400,000 | 0 | 0 | 0 | 0 | 84.70% / 91.60% | 18.89 / 19.54 | 0 |
| C2 | 3 | PASS | 400,000 | 400,000 | 0 | 0 | 0 | 0 | 84.84% / 90.80% | 18.83 / 19.44 | 0 |
| C3 | 1 | PASS | 38,000 | 38,000 | 0 | 0 | 0 | 0 | 20.36% / 22.00% | 19.96 / 22.53 | 1949 |
| C3 | 2 | PASS | 38,000 | 38,000 | 0 | 0 | 0 | 0 | 20.86% / 22.40% | 19.93 / 22.43 | 1954 |
| C3 | 3 | PASS | 38,000 | 38,000 | 0 | 0 | 0 | 0 | 20.55% / 22.50% | 19.44 / 21.71 | 1938 |
| C4 | 1 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 15.80% / 17.70% | 17.22 / 17.64 | 0 |
| C4 | 2 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 15.68% / 17.00% | 17.33 / 17.83 | 0 |
| C4 | 3 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 15.93% / 17.60% | 17.25 / 17.84 | 0 |

## Result

- C1: 3/3 runs passed; aggregate expected=120,000, received=120,000, missing=0.
- C2: 3/3 runs passed; aggregate expected=1,200,000, received=1,200,000, missing=0.
- C3: 3/3 runs passed; aggregate expected=114,000, received=114,000, missing=0.
- C4: 3/3 runs passed; aggregate expected=120,000, received=120,000, missing=0.
- C3 logged 1938/1949/1954 parse-error batches per run. The parser still returns an error for the entire batch, but the host bisected every contaminated batch (src/isolation.rs) and delivered the valid records: rep1 salvaged=95,000 rejected=5,000 parse calls=49,236; rep2 salvaged=95,000 rejected=5,000 parse calls=49,202; rep3 salvaged=95,000 rejected=5,000 parse calls=49,310 (malformed inputs per run: 5000, 5000, 5000).
- No run was OOM-killed and the WCM container remained alive until the harness deliberately stopped it.
