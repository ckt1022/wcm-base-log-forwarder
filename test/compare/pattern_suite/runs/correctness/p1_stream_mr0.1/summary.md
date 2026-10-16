# WCM correctness experiment

1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, 1 s max wait, single TCP connection and HTTP worker.
C1: valid inputs at 2,500 EPS; C2: valid inputs at 17,500 EPS; C3: deterministic 5% malformed JSON;
C4: boolean, integer, float, null, Unicode, quote and backslash edge values. Each formal scenario was repeated three times.

| Scenario | Rep | Status | Expected | Received | Missing | Dup | Corrupt | Reorder | CPU avg/max | MEM avg/max MiB | Parse-error batches |
|:--|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| C3 | 1 | PASS | 39,955 | 39,955 | 0 | 0 | 0 | 0 | 17.61% / 19.10% | 13.46 / 14.01 | 118 |
| C3 | 2 | PASS | 39,955 | 39,955 | 0 | 0 | 0 | 0 | 18.68% / 19.90% | 13.78 / 14.18 | 119 |
| C3 | 3 | PASS | 39,955 | 39,955 | 0 | 0 | 0 | 0 | 19.14% / 20.40% | 13.55 / 14.06 | 116 |

## Result

- C3: 3/3 runs passed; aggregate expected=119,865, received=119,865, missing=0.
- C3 logged 116/118/119 parse-error batches per run. The parser still returns an error for the entire batch, but the host bisected every contaminated batch (src/isolation.rs) and delivered the valid records: rep1 salvaged=5,951 rejected=122 parse calls=1,382; rep2 salvaged=6,019 rejected=122 parse calls=1,398; rep3 salvaged=5,880 rejected=122 parse calls=1,388 (malformed inputs per run: 122, 122, 122).
- No run was OOM-killed and the WCM container remained alive until the harness deliberately stopped it.
