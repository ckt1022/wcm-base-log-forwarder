# Isolated WCM component benchmarks

This runner measures Parse, Filter, and Format independently. There is no
production pipeline and no upstream/downstream worker running during H0..H3.
All three stages use the production WIT function signatures plus the same two
benchmark-only `time-sign` markers.

```text
H0 host immediately before the component call
H1 guest after Canonical ABI input lowering
H2 guest after the stage result is complete
H3 host immediately after Canonical ABI output lifting
```

Every sample uses a pre-instantiated fresh instance exactly once. Replacement
instantiation occurs after H3 and is not timed. Inputs use the same deterministic
1024-byte JSON fixture and corresponding `LogEntry` shape as the existing WCM
experiments.

Short test of all stages:

```bash
cd /home/ckt1022/wcm-base-log-forwarder/test/ours/copy_overhead/wcm_isolated_benchmark
for stage in parse filter format; do
    CPU=2 ./run_one.sh "$stage" 64 128 3 1
done
```

Formal run: ten non-zero powers of two from 64 through 32768, a zero-record
control, 100 samples plus 10 warm-ups per size, three independent runs, and a
20-second interval between sweeps:

```bash
CPU=2 ROUNDS=100 WARMUP=10 RUNS=3 INTERVAL=20 ./run_all.sh
```

Use `STAGES="parse filter format"` to select stages. Results are written to a
timestamped directory under `results/`.
