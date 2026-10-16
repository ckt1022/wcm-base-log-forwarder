# WCM correctness experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, 1 s max wait, single TCP connection and HTTP worker.
C1: valid inputs at 2,500 EPS; C2: valid inputs at 17,500 EPS; C3: deterministic 5% malformed JSON;
C4: boolean, integer, float, null, Unicode, quote and backslash edge values. Each formal scenario was repeated three times.

| Scenario | Rep | Status | Expected | Received | Missing | Dup | Corrupt | Reorder | CPU avg/max | MEM avg/max MiB | Parse-error batches |
|:--|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| C3 | 1 | PASS | 31,964 | 31,964 | 0 | 0 | 0 | 0 | 21.98% / 23.10% | 23.84 / 33.87 | 1937 |
| C3 | 2 | PASS | 31,964 | 31,964 | 0 | 0 | 0 | 0 | 22.35% / 23.50% | 24.22 / 34.18 | 1933 |
| C3 | 3 | PASS | 31,964 | 31,964 | 0 | 0 | 0 | 0 | 22.38% / 23.60% | 22.88 / 32.83 | 1936 |

## Result

- C3: 3/3 runs passed; aggregate expected=95,892, received=95,892, missing=0.
- C3 logged 1933/1936/1937 parse-error batches per run. The parser still returns an error for the entire batch, but the host bisected every contaminated batch (src/isolation.rs) and delivered the valid records: rep1 salvaged=80,028 rejected=19,972 parse calls=109,208; rep2 salvaged=80,028 rejected=19,972 parse calls=109,200; rep3 salvaged=80,028 rejected=19,972 parse calls=108,928 (malformed inputs per run: 19972, 19972, 19972).
- No run was OOM-killed and the WCM container remained alive until the harness deliberately stopped it.
