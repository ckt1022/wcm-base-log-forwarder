# Format copy-overhead components

The Rust and C implementations are isolated copies of the production flat-JSON
format plugins. The C# implementation emits the same fields, ordering, JSON
representation, and `[4-byte little-endian length][JSON bytes]` framing.

All three use the production `format` signature. The only WIT change is the
benchmark-only imported `time-sign()` function.

```text
H1 first time-sign: complete LogEntry input is available in the component
H2 second time-sign: all framed flat-JSON output bytes are constructed
```

Component artifacts:

- Rust: `component/target/wasm32-wasip2/release/format_copy_benchmark_component.wasm`
- C: `c_component/format_c_benchmark.wasm`
- C#: `csharp_component/format_csharp_benchmark.wasm`

They can be exercised through the copied production source under the sibling
`src_pipeline_benchmark` directory. Rust also has a standalone host that runs
no Parse, Filter, or pipeline worker while H0..H3 is measured:

```bash
taskset -c 2 ./run.sh 64 32768 100 10 > results/format_rust_isolated.csv
```

The standalone host uses a three-instance fresh pool, validates both markers,
compares every returned byte with a host reference built before H0, and checks
the number of length-prefixed frames.
