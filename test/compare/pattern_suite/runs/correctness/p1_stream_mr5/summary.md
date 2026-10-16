# WCM correctness experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, 1 s max wait, single TCP connection and HTTP worker.
C1: valid inputs at 2,500 EPS; C2: valid inputs at 17,500 EPS; C3: deterministic 5% malformed JSON;
C4: boolean, integer, float, null, Unicode, quote and backslash edge values. Each formal scenario was repeated three times.

| Scenario | Rep | Status | Expected | Received | Missing | Dup | Corrupt | Reorder | CPU avg/max | MEM avg/max MiB | Parse-error batches |
|:--|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| C3 | 1 | PASS | 37,894 | 37,894 | 0 | 0 | 0 | 0 | 23.26% / 24.50% | 16.40 / 19.17 | 1807 |
| C3 | 2 | PASS | 37,894 | 37,894 | 0 | 0 | 0 | 0 | 23.14% / 23.90% | 15.10 / 17.95 | 1798 |
| C3 | 3 | PASS | 37,894 | 37,894 | 0 | 0 | 0 | 0 | 23.87% / 27.20% | 16.54 / 19.03 | 1814 |

## Result

- C3: 3/3 runs passed; aggregate expected=113,682, received=113,682, missing=0.
- C3 logged 1798/1807/1814 parse-error batches per run. The parser still returns an error for the entire batch, but the host bisected every contaminated batch (src/isolation.rs) and delivered the valid records: rep1 salvaged=88,240 rejected=5,176 parse calls=43,236; rep2 salvaged=87,736 rejected=5,176 parse calls=43,366; rep3 salvaged=88,449 rejected=5,176 parse calls=43,290 (malformed inputs per run: 5176, 5176, 5176).
- No run was OOM-killed and the WCM container remained alive until the harness deliberately stopped it.
