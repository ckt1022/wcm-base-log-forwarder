# WCM correctness experiment

1024 B / 36 fields, native process (no container, no cgroup limit; see meta.txt for taskset pinning), 1 MiB batch, 1 s max wait, single TCP connection and HTTP worker.
C1: valid inputs at 2,500 EPS; C2: valid inputs at 17,500 EPS; C3: deterministic 5% malformed JSON;
C4: boolean, integer, float, null, Unicode, quote and backslash edge values. Each formal scenario was repeated three times.

| Scenario | Rep | Status | Expected | Received | Missing | Dup | Corrupt | Reorder | CPU avg/max | MEM avg/max MiB | Parse-error batches |
|:--|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|
| C1 | 1 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 20.08% / 28.30% | 40.27 / 40.27 | 0 |
| C1 | 2 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 20.34% / 29.40% | 41.55 / 41.58 | 0 |
| C1 | 3 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 19.39% / 30.40% | 41.91 / 41.93 | 0 |
| C3 | 1 | PASS | 38,000 | 38,000 | 0 | 0 | 0 | 0 | 29.45% / 40.10% | 40.95 / 41.00 | 99 |
| C3 | 2 | PASS | 38,000 | 38,000 | 0 | 0 | 0 | 0 | 30.05% / 42.00% | 41.11 / 42.07 | 99 |
| C3 | 3 | PASS | 38,000 | 38,000 | 0 | 0 | 0 | 0 | 30.20% / 45.00% | 40.93 / 41.00 | 99 |
| C4 | 1 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 21.26% / 31.20% | 39.29 / 41.48 | 0 |
| C4 | 2 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 22.04% / 31.30% | 38.91 / 41.47 | 0 |
| C4 | 3 | PASS | 40,000 | 40,000 | 0 | 0 | 0 | 0 | 21.90% / 32.30% | 41.24 / 41.27 | 0 |

## Result

- C1: 3/3 runs passed; aggregate expected=120,000, received=120,000, missing=0.
- C3: 3/3 runs passed; aggregate expected=114,000, received=114,000, missing=0.
- C4: 3/3 runs passed; aggregate expected=120,000, received=120,000, missing=0.
- C3 logged 99 parse-error batches per run. The parser still returns an error for the entire batch, but the host bisected every contaminated batch (src/isolation.rs) and delivered the valid records: rep1 salvaged=95,000 rejected=5,000 parse calls=52,308; rep2 salvaged=95,000 rejected=5,000 parse calls=52,308; rep3 salvaged=95,000 rejected=5,000 parse calls=52,308 (malformed inputs per run: 5000, 5000, 5000).
- No run was OOM-killed and the native forwarder process remained alive until the harness deliberately stopped it.
