# Rust component → Rust host marker-call benchmark

This microbenchmark measures the average cost of synchronous, parameterless
calls from a Rust WebAssembly component into a Rust Wasmtime host and back.
It measures both an empty callback and the real `time-sign()` shape that calls
`Instant::now()` and saves the timestamp in host state.

The component exports two functions with identical loop arithmetic:

- `baseline(iterations, seed)`: loop only.
- `with-noop-sign(iterations, seed)`: the same loop plus one empty host callback.
- `with-time-sign(iterations, seed)`: the same loop plus one host callback that
  reads and stores `Instant::now()`.

For every round, the host measures both exports and calculates:

```text
empty callback cost = (with_noop_sign_ns - baseline_ns) / iterations
timestamp callback cost = (with_time_sign_ns - baseline_ns) / iterations
clock-read increment = (with_time_sign_ns - with_noop_sign_ns) / iterations
```

Running many marker calls inside one component invocation amortizes the host
clock-read and exported-function call overhead. This benchmark measures the
full synchronous component → host → component round trip, not merely the body
of the host callback.

## Run

```bash
cd test/ours/copy_overhead/marker_call
./run.sh
```

Optional arguments:

```bash
./run.sh ITERATIONS ROUNDS WARMUP_ROUNDS
./run.sh 100000 30 5
```

The host prints per-round data as CSV to stdout and prints the summary to
stderr. Redirect stdout to retain the raw samples:

```bash
./run.sh 100000 30 5 > results.csv
```

Use a fixed CPU when possible:

```bash
taskset -c 2 ./run.sh 100000 30 5 > results.csv
```
