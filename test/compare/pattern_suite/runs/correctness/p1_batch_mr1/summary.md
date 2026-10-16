# WCM correctness experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, 1 s max wait, single TCP connection and HTTP worker.
C1: valid inputs at 2,500 EPS; C2: valid inputs at 17,500 EPS; C3: deterministic 5% malformed JSON;
C4: boolean, integer, float, null, Unicode, quote and backslash edge values. Each formal scenario was repeated three times.

| Scenario | Rep | Status | Expected | Received | Missing | Dup | Corrupt | Reorder | CPU avg/max | MEM avg/max MiB | Parse-error batches |
|:--|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| C3 | 1 | PASS | 39,581 | 39,581 | 0 | 0 | 0 | 0 | 18.69% / 21.00% | 16.63 / 17.36 | 373 |
| C3 | 2 | PASS | 39,581 | 39,581 | 0 | 0 | 0 | 0 | 18.91% / 22.20% | 16.57 / 18.13 | 371 |
| C3 | 3 | PASS | 39,581 | 39,581 | 0 | 0 | 0 | 0 | 19.28% / 20.70% | 16.62 / 17.73 | 369 |

## Result

- C3: 3/3 runs passed; aggregate expected=118,743, received=118,743, missing=0.
- C3 logged 369/371/373 parse-error batches per run. The parser still returns an error for the entire batch, but the host bisected every contaminated batch (src/isolation.rs) and delivered the valid records: rep1 salvaged=92,201 rejected=1,090 parse calls=13,618; rep2 salvaged=91,966 rejected=1,090 parse calls=13,630; rep3 salvaged=91,165 rejected=1,090 parse calls=13,594 (malformed inputs per run: 1090, 1090, 1090).
- No run was OOM-killed and the WCM container remained alive until the harness deliberately stopped it.
