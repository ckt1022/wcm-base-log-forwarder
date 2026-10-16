# WCM correctness experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, 1 s max wait, single TCP connection and HTTP worker.
C1: valid inputs at 2,500 EPS; C2: valid inputs at 17,500 EPS; C3: deterministic 5% malformed JSON;
C4: boolean, integer, float, null, Unicode, quote and backslash edge values. Each formal scenario was repeated three times.

| Scenario | Rep | Status | Expected | Received | Missing | Dup | Corrupt | Reorder | CPU avg/max | MEM avg/max MiB | Parse-error batches |
|:--|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| C3 | 1 | PASS | 37,894 | 37,894 | 0 | 0 | 0 | 0 | 19.74% / 23.00% | 18.73 / 21.45 | 400 |
| C3 | 2 | PASS | 37,894 | 37,894 | 0 | 0 | 0 | 0 | 19.24% / 21.30% | 18.77 / 21.93 | 400 |
| C3 | 3 | PASS | 37,894 | 37,894 | 0 | 0 | 0 | 0 | 19.98% / 21.20% | 19.01 / 22.18 | 400 |

## Result

- C3: 3/3 runs passed; aggregate expected=113,682, received=113,682, missing=0.
- C3 logged 400 parse-error batches per run. The parser still returns an error for the entire batch, but the host bisected every contaminated batch (src/isolation.rs) and delivered the valid records: rep1 salvaged=94,798 rejected=5,176 parse calls=46,036; rep2 salvaged=94,809 rejected=5,176 parse calls=45,782; rep3 salvaged=94,809 rejected=5,176 parse calls=45,914 (malformed inputs per run: 5176, 5176, 5176).
- No run was OOM-killed and the WCM container remained alive until the harness deliberately stopped it.
