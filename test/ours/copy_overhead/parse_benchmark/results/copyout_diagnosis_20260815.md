# Fresh-pool copy-out discontinuity diagnosis

## Conclusion

The 512-to-1024-record increase is caused by canonical-ABI output lifting of
the object graph returned as `list<parsed-entry>`, especially the creation of
many independent host `String`/`Vec` values. It is not caused by JSON parsing,
input lowering, or a `memory.grow` in H2-H3.

The discontinuity appears when the live, pointer-rich output approaches the
512 KiB private L2 cache of the pinned CPU core. A single contiguous output
string does not show the discontinuity, so total copied bytes alone are not
sufficient: per-record allocation and pointer-chasing/locality are essential.

This is a strongly supported mechanism rather than a hardware-counter proof;
the WSL kernel does not provide a matching `perf` binary for cache-miss counts.

## Controlled experiments

All calls use `fresh-pool`, the production-aligned discard-and-replenish path.
The parse WIT signature remains unchanged.

| Variant | JSON parsed | Output | Finding |
|---|---|---|---|
| `empty-list` | yes | empty list | H2-H3 remains about 5-9 us |
| `compact-record` | yes | N entries, empty strings/tags | H2-H3 scales linearly |
| `message-only` | yes | N entries, one 917 B string each | sharp repeatable jump at 552-560 records |
| `message-256` | yes | N entries, one 256 B string each | jump moves to 1472-1536 records |
| `one-message` | yes | one concatenated string | no jump while crossing 512 KiB |
| `full` | yes | production-shaped ParsedEntry | same discontinuity near the 512 KiB working-set region |

### Repeatability around each transition

H2-H3 medians from 40 measured calls per size:

| Variant | Below transition | Above transition | Change |
|---|---:|---:|---:|
| `message-only` | 552: 80.0 us | 560: 339.4 us | 4.24x for 1.45% more records |
| `message-256` | 1472: 184.4 us | 1536: 474.6 us | 2.57x for 4.35% more records |
| `one-message` | 512: 26.3 us | 576: 28.7 us | no discontinuity despite a 528,192 B string |

The message-length experiment moves the transition when the per-record payload
shrinks. The one-message experiment removes the transition when the same total
payload is represented by one allocation. Together these distinguish the
object-graph lifting/allocation effect from raw memcpy size.

## Stage-specific memory growth

The host resource limiter records growth by marker phase. For full output:

| Records | H0-H1 grow | H1-H2 grow | H2-H3 grow |
|---:|---:|---:|---:|
| 512 | 8 pages | 10 pages | 0 pages |
| 1024 | 16 pages | 20 pages | 0 pages |

Every output-stage sample in every diagnostic variant recorded zero H2-H3
`memory.grow` calls. Growth is linear and occurs while lowering input or while
the component parses/builds its Rust result before H2.

## Hardware context

The benchmark was pinned to one core of an AMD Ryzen 5 7535HS. `lscpu -C`
reports a 512 KiB private L2 cache per core. The observed working-set transition
is consistent with that cache size: the canonical lifting loop traverses ABI
records and allocates/copies many separate host objects, while the contiguous
single-string control can use a streaming copy and remains smooth.

## Result files

- `copyout_diag_*_20260815_170132.csv`: broad four-variant sweep
- `copyout_diag_*_20260815_170455.csv`: dense 512-640 sweep
- `copyout_diag_message-256_*_20260815_171400.csv` and
  `copyout_diag_message-256_mid_20260815_171500.csv`: shifted-payload sweeps
- `copyout_diag_one-message_20260815_172400.csv`: contiguous-output control
- `copyout_confirm_*_20260815.csv`: 40-sample confirmation sweeps

