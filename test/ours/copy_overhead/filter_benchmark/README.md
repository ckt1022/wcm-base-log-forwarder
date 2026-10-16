# Filter copy-overhead benchmark

This experiment is separate from `parse_benchmark` and does not modify the
production `src` tree. Its `filter` input/output types match the production
`reduction-plugin` world. The only WIT addition is the imported `time-sign()`
marker.

Each input `LogEntry` reproduces the dynamic payload emitted by the parse
fixture: a 917-byte message and 979 total string bytes. Including the logical
`id` and `level` values gives 988 logical input bytes per entry. The originating
JSON size (1024 bytes per record) is also recorded separately in CSV.

Levels alternate between `info` and `warn`; the production filter rule keeps
`warn` and above, so exactly half of every non-empty default batch is kept.

Timing boundaries:

```text
H0 host before call
H1 first time-sign, after LogEntry input lowering/binding
H2 second time-sign, after filter decisions/output construction
H3 host after FilterResult output lifting
```

This version uses `fresh-pool` mode: three instances are prepared outside the
timed interval, every instance is called exactly once and then discarded, and
its replacement is instantiated outside H0..H3. Component compilation and
instantiation are therefore not included in the copy timings.

## Short test

```bash
cd test/ours/copy_overhead/filter_benchmark
taskset -c 2 ./run.sh 64 128 3 1 > results/filter_short.csv
```

## Full ascending sweep

```bash
taskset -c 2 ./run.sh 64 32768 100 10 > results/filter_rust_fresh_run1.csv
```

The zero-entry baseline is always included. Arguments are
`START_COUNT END_COUNT ROUNDS WARMUP_ROUNDS [CUSTOM_SIZE_LIST]`.

## C and C# components

The C and C# variants use the same WIT, Rust host, input objects, 50% keep
rule, fresh-pool mode, and marker boundaries:

```bash
taskset -c 2 ./run_c.sh 64 8192 100 10 > results/filter_c_fresh.csv
taskset -c 2 ./run_csharp.sh 64 8192 100 10 > results/filter_csharp_fresh.csv
```

Run all three languages three times each and verify that every CSV has 901 lines:

```bash
./run_all_languages_three.sh
```

Set `CPU=<core>` to override the default CPU core 2.
