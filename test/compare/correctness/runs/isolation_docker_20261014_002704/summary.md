# WCM correctness experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, 1 s max wait, single TCP connection and HTTP worker.
C1: valid inputs at 2,500 EPS; C2: valid inputs at 17,500 EPS; C3: deterministic 5% malformed JSON;
C4: boolean, integer, float, null, Unicode, quote and backslash edge values. Each formal scenario was repeated three times.

| Scenario | Rep | Status | Expected | Received | Missing | Dup | Corrupt | Reorder | CPU avg/max | MEM avg/max MiB | Parse-error batches |
|:--|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| C1 | 1 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 13.09% / 15.80% | 25.88 / 29.65 | 0 |
| C1 | 2 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 12.88% / 17.70% | 25.76 / 30.19 | 0 |
| C1 | 3 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 12.95% / 15.90% | 26.00 / 29.70 | 0 |
| C2 | 1 | PASS | 400,000 | 400,000 | 0 | 0 | 0 | 0 | 78.90% / 84.50% | 25.38 / 31.01 | 0 |
| C2 | 2 | PASS | 400,000 | 400,000 | 0 | 0 | 0 | 0 | 79.02% / 84.50% | 25.59 / 30.88 | 0 |
| C2 | 3 | PASS | 400,000 | 400,000 | 0 | 0 | 0 | 0 | 73.58% / 78.70% | 27.71 / 30.90 | 0 |
| C3 | 1 | PASS | 38,000 | 38,000 | 0 | 0 | 0 | 0 | 18.44% / 22.70% | 30.55 / 33.17 | 99 |
| C3 | 2 | PASS | 38,000 | 38,000 | 0 | 0 | 0 | 0 | 18.25% / 22.30% | 28.04 / 30.82 | 99 |
| C3 | 3 | PASS | 38,000 | 38,000 | 0 | 0 | 0 | 0 | 19.26% / 24.00% | 28.19 / 30.67 | 99 |
| C4 | 1 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 14.73% / 26.80% | 22.62 / 28.46 | 0 |
| C4 | 2 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 16.59% / 28.00% | 25.99 / 30.11 | 0 |
| C4 | 3 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 16.51% / 22.80% | 25.77 / 28.12 | 0 |

## Result

- C1: 3/3 runs passed; aggregate expected=120,000, received=120,000, missing=0.
- C2: 3/3 runs passed; aggregate expected=1,200,000, received=1,200,000, missing=0.
- C3: 3/3 runs passed; aggregate expected=114,000, received=114,000, missing=0.
- C4: 3/3 runs passed; aggregate expected=120,000, received=120,000, missing=0.
- C3 logged 99 parse-error batches per run. The parser still returns an error for the entire batch, but the host bisected every contaminated batch (src/isolation.rs) and delivered the valid records: rep1 salvaged=95,000 rejected=5,000 parse calls=52,308; rep2 salvaged=95,000 rejected=5,000 parse calls=52,308; rep3 salvaged=95,000 rejected=5,000 parse calls=52,312 (malformed inputs per run: 5000, 5000, 5000).
- No run was OOM-killed and the WCM container remained alive until the harness deliberately stopped it.
