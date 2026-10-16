# WCM correctness experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, 1 s max wait, single TCP connection and HTTP worker.
C1: valid inputs at 2,500 EPS; C2: valid inputs at 17,500 EPS; C3: deterministic 5% malformed JSON;
C4: boolean, integer, float, null, Unicode, quote and backslash edge values. Each formal scenario was repeated three times.

| Scenario | Rep | Status | Expected | Received | Missing | Dup | Corrupt | Reorder | CPU avg/max | MEM avg/max MiB | Parse-error batches |
|:--|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| C3 | 1 | PASS | 11,881 | 11,881 | 0 | 0 | 0 | 0 | 20.48% / 21.20% | 13.55 / 14.02 | 228 |

## Result

- C3: 1/1 runs passed; aggregate expected=11,881, received=11,881, missing=0.
- C3 logged 228 parse-error batches per run. The parser still returns an error for the entire batch, but the host bisected every contaminated batch (src/isolation.rs) and delivered the valid records: rep1 salvaged=11,477 rejected=310 parse calls=3,194 (malformed inputs per run: 310).
- No run was OOM-killed and the WCM container remained alive until the harness deliberately stopped it.
