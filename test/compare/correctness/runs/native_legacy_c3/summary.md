# WCM correctness experiment

1024 B / 36 fields, native process (no container, no cgroup limit; see meta.txt for taskset pinning), 1 MiB batch, 1 s max wait, single TCP connection and HTTP worker.
C1: valid inputs at 2,500 EPS; C2: valid inputs at 17,500 EPS; C3: deterministic 5% malformed JSON;
C4: boolean, integer, float, null, Unicode, quote and backslash edge values. Each formal scenario was repeated three times.

| Scenario | Rep | Status | Expected | Received | Missing | Dup | Corrupt | Reorder | CPU avg/max | MEM avg/max MiB | Parse-error batches |
|:--|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| C3 | 1 | FAIL | 38,000 | 0 | 38,000 | 0 | 0 | 0 | 2.36% / 3.90% | 26.45 / 26.48 | 99 |
| C3 | 2 | FAIL | 38,000 | 0 | 38,000 | 0 | 0 | 0 | 2.33% / 3.90% | 26.63 / 26.65 | 99 |
| C3 | 3 | FAIL | 38,000 | 0 | 38,000 | 0 | 0 | 0 | 2.39% / 3.90% | 26.80 / 26.83 | 99 |

## Result

- C3: 0/3 runs passed; aggregate expected=114,000, received=0, missing=114,000.
- C3 logged 99 parse-error batches per run. The parser returns an error for the entire batch when any record is malformed and isolation was off (legacy behaviour), so every contaminated batch was discarded.
- No run was OOM-killed and the native forwarder process remained alive until the harness deliberately stopped it.
