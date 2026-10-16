# Rust Core Wasm JSON ABI Benchmark — Implementation Guide

## 1. Purpose

Build a control group for the existing WebAssembly Component Model (WCM)
copy-overhead experiments. The control uses:

- a Rust host;
- Wasmtime's **core module API**, not the component API;
- Rust guests compiled to `wasm32-unknown-unknown`;
- JSON as the application data ABI;
- separate parse, filter, and format experiments;
- the same H0, H1, H2, and H3 timing boundaries as the WCM experiments;
- a fresh, one-shot instance for every measured call.

The first implementation must be created entirely under this directory. Do
not modify the repository's original `src` directory.

This guide is a design specification only. No host or guest implementation is
part of the initial guide-writing task.

## 2. Confirmed decisions

| Item | Decision |
|---|---|
| Host language | Rust |
| Guest language | Rust, first version only |
| Wasm model | Core WebAssembly module |
| Wasm target | `wasm32-unknown-unknown` |
| Data ABI | UTF-8 JSON request/response envelopes |
| Operations | Parse, filter, and format, tested independently |
| Timing | H0/H1/H2/H3 |
| Lifecycle | Fresh instance, call once, then discard |
| Input log | Same deterministic 1,024-byte JSON log used by WCM tests |
| Batch sizes | 64, 128, 256, 512, 1024, 2048, 4096, 8192 |
| Formal sampling | 10 warm-ups + 100 measured calls, three runs |
| Inter-experiment delay | 20 seconds |

## 3. What this comparison means

Core Wasm has no WIT or canonical ABI. Selecting JSON therefore creates a
different application ABI from WCM.

The primary comparison will be:

```text
WCM typed canonical ABI
versus
Core Wasm JSON ABI
```

It can answer which complete boundary method is cheaper for this workload. It
must **not** be described as a measurement in which the Component Model is the
only independent variable. Differences also include JSON serialization,
deserialization, and, for format output, Base64 encoding/decoding.

The guest business logic between H1 and H2 must nevertheless match the Rust
WCM component exactly. This makes the guest interval comparable while keeping
the two boundary methods explicit.

## 4. Implemented directory layout

```text
core_wasm_json_benchmark/
├── IMPLEMENTATION_GUIDE.md
├── README.md
├── host/
│   ├── Cargo.toml
│   ├── Cargo.lock
│   └── src/
│       ├── main.rs
│       ├── fixture.rs
│       └── stats.rs
├── guest-common/
│   ├── Cargo.toml
│   └── src/lib.rs
├── guest-parse/
│   ├── Cargo.toml
│   └── src/lib.rs
├── guest-filter/
│   ├── Cargo.toml
│   └── src/lib.rs
├── guest-format/
│   ├── Cargo.toml
│   └── src/lib.rs
├── scripts/
│   ├── build.sh
│   ├── run_one.sh
│   └── run_all.sh
└── results/
```

Use separate guest crates so each can match the corresponding WCM component's
business logic and release optimization without feature interactions.

## 5. Source-of-truth WCM implementations

The Core Wasm guest logic must be copied or mechanically adapted from these
files:

```text
../parse_benchmark/component/src/lib.rs
../filter_benchmark/component/src/lib.rs
../format_benchmark/component/src/lib.rs
```

The logical types come from:

```text
../../../../wit/log_plugin.wit
```

Do not call one stage to prepare another stage during measurement. Parse,
filter, and format must each receive a preconstructed input before H0 so their
results remain independent.

### 5.1 Parse logic

Keep these behaviors identical to the Rust WCM parse component:

- parse every raw record with `serde_json::from_str::<Value>`;
- use the same `value_to_string`, `map_level`, and `collect_tags` behavior;
- include `("lang", "Rust")` as the first tag;
- use `ts`, `level`, `msg`, and `att` exactly as the WCM implementation does;
- use target tag `"A"`;
- return the same invalid-format error for malformed JSON.

Do not replace the parser with typed `serde` deserialization. The inner-log
parsing must continue using `serde_json::Value`, because that is the business
logic being controlled.

### 5.2 Filter logic

Keep the same rule:

```text
keep = entry.level >= warn
```

Return one `{id, keep}` result for every input entry, preserving order.

### 5.3 Format logic

Use the default `json-flat` behavior from the Rust WCM format component:

- field order: `id`, `timestamp`, `level`, `message`, then flattened tags;
- same `level_name` implementation;
- same `json_escape` implementation;
- same `[4-byte little-endian length][JSON bytes]` framing per entry;
- preserve tag order and allow duplicate tag keys exactly as the component
  does.

The H1-H2 output must be byte-for-byte identical to WCM format output for the
same logical input.

## 6. Stable JSON ABI

Do not expose Rust's default enum serialization as an undocumented protocol.
Define explicit request and response structs and add golden JSON tests.

All JSON is UTF-8, compact, and contains no optional whitespace. Object field
order must follow the examples below so ABI byte sizes are deterministic.

Log levels are lowercase strings:

```text
debug, info, warn, error, crit, alert, emerg
```

Tags are arrays of two-element arrays, not JSON objects. This preserves order
and duplicate keys:

```json
[["lang","Rust"],["service","api"]]
```

### 6.1 Parse request

```json
{"records":["{...1024-byte raw log...}","{...}"]}
```

The inner logs remain JSON strings. Consequently they are escaped inside the
outer JSON request. Record both logical input bytes and JSON ABI input bytes.

### 6.2 Parse response

Success:

```json
{"ok":{"entries":[{"timestamp":"...","level":"info","message":"...","tags":[["lang","Rust"]],"targettag":"A"}]}}
```

Failure:

```json
{"err":{"kind":"invalid-format","message":"JSON Parse Error"}}
```

### 6.3 Filter request

```json
{"entries":[{"id":1,"timestamp":"...","level":"info","message":"...","tags":[["lang","Rust"]]}]}
```

### 6.4 Filter response

```json
{"ok":{"results":[{"id":1,"keep":false}]}}
```

### 6.5 Format request

Use the same `entries` schema as the filter request.

### 6.6 Format response

The WCM format contract returns arbitrary `list<u8>`, and its output contains
binary four-byte frame lengths. To keep a strict JSON response without
changing the H1-H2 formatter, encode the exact framed bytes as standard Base64:

```json
{"ok":{"framed_output_base64":"..."}}
```

H2 occurs **before** Base64 and response JSON serialization. Therefore Base64
encoding belongs to output-boundary cost, not guest format logic. Host Base64
decoding also belongs to H2-H3.

This cost must be disclosed in every Core-Wasm-vs-WCM interpretation. Do not
claim that H2-H3 is only a memory copy.

## 7. Core Wasm exports and ownership

Each guest module exports:

```rust
extern "C" fn alloc(len: u32) -> u32;
extern "C" fn dealloc(ptr: u32, len: u32);
extern "C" fn process(ptr: u32, len: u32) -> u64;
```

Each module contains only one operation:

- parse module `process` performs parse;
- filter module `process` performs filter;
- format module `process` performs format.

`process` returns a packed pointer and length:

```text
bits 63..32 = output pointer
bits 31..0  = output length
```

The experiment caps memory below 4 GiB, so `u32` pointers and lengths are
sufficient. Treat pointer 0 with length 0 as invalid for this protocol; errors
must be returned as JSON error responses instead of an out-of-band sentinel.

Ownership rules:

1. Host calls guest `alloc` for request bytes.
2. Host writes the JSON request into guest linear memory.
3. Guest borrows request memory during `process`; it does not free it.
4. Guest allocates response bytes and transfers ownership to host.
5. Host copies and decodes the response.
6. Host calls `dealloc` for both request and response buffers.
7. Host discards the entire instance after the call.

Implement alloc/dealloc using a symmetric allocation mechanism. Do not assume
that `Vec::with_capacity(len)` always has capacity exactly equal to `len` when
reconstructing it. A boxed slice or explicit `std::alloc::Layout` is safer.

## 8. Time-sign import

The guest imports one core Wasm function:

```text
module: bench
name:   time-sign
type:   () -> ()
```

The host records the first two calls using `Instant::now()`:

- first import call = H1;
- second import call = H2;
- any other count is an invalid sample.

The previously measured time-sign p95 is approximately 23 ns. Retain the raw
timings; do not subtract this value from each sample. Report it as measurement
instrumentation overhead.

## 9. Exact four-point timing semantics

### H0

Record immediately before host JSON request serialization. H0-H1 includes:

- host `serde_json::to_vec`;
- guest `alloc` call;
- host `Memory::write`;
- core Wasm function-call transition;
- guest `serde_json::from_slice` into Rust values;
- any fresh Rust guest initialization that occurs before the first marker.

Therefore call this interval **input boundary latency**, not pure copy-in.

### H1

Guest calls `time-sign` immediately after the complete request has been decoded
into the Rust values consumed by the business function.

### H2

Guest calls `time-sign` after the business result is completely constructed but
before response JSON serialization:

- parse: after `Vec<ParsedEntry>` or parse error exists;
- filter: after all `FilterResult` values exist;
- format: after all length-prefixed output bytes exist.

H1-H2 must contain only the logic corresponding to the WCM component.

### H3

Record after all required ABI output work is complete:

- guest response serialization;
- guest response-buffer allocation;
- return to host;
- host `Memory::read`;
- host response JSON deserialization;
- format Base64 decoding;
- request and response `dealloc` calls.

Correctness hashing, result comparison, CSV printing, statistics, and instance
drop happen after H3.

Reported intervals:

```text
input_boundary_ns  = H1 - H0
guest_ns           = H2 - H1
output_boundary_ns = H3 - H2
total_ns           = H3 - H0
```

For compatibility with existing analysis scripts, CSV may additionally alias
the boundary fields as `copy_in_ns` and `copy_out_ns`, but documentation must
use the more accurate names.

## 10. Fresh lifecycle

Match the WCM fresh experiment:

```text
compile/load module once
→ instantiate an unused instance before H0
→ call process once
→ record H3
→ validate output
→ discard instance
→ create the replacement outside H0-H3
```

Prefer a three-instance fresh pool like the parse benchmark. A measured call
acquires an unused instance, consumes it once, and never returns it to the pool.
Replenishment occurs outside H0-H3.

The primary four-point CSV excludes instantiate and drop. A later extension
may add `instantiate_ns`, `drop_ns`, and `fresh_total_ns`, but those metrics must
not change H0-H3 semantics.

## 11. Host execution model

Use Wasmtime `=42.0.1`, matching the existing benchmark host. Use the core APIs:

- `Engine`;
- `Module`;
- `Linker`;
- `Store`;
- typed core functions;
- exported `Memory`.

Do not use:

- `wasmtime::component`;
- WIT;
- `wit-bindgen`;
- `wasmtime-wasi`, unless the guest later demonstrates a required WASI import.

Prebuild every request fixture before starting measured calls except the JSON
serialization deliberately included after H0. The logical Rust values may be
prepared beforehand; the JSON byte vector may not.

The standalone host should use one measurement thread. Pin the process to one
CPU because it has no parse/filter/format pipeline workers competing with the
measured call:

```bash
taskset -c 2 ...
```

Do not run all languages or stages concurrently.

## 12. Fixture alignment

Use the same deterministic raw-log generator as:

```text
../src_pipeline_benchmark/generate_input.js
```

Requirements:

- every raw log is exactly 1,024 bytes before the outer JSON ABI encoding;
- levels alternate in the same Info/Warn pattern;
- parse gets the exact raw strings;
- filter and format get preconstructed `LogEntry` values with deterministic
  IDs and the same strings/tags as the matching WCM test;
- record `logical_input_bytes` and `abi_input_bytes` separately.

Do not use Core parse output to feed a measured Core filter/format call. That
would make preparation order and parser implementation part of those tests.

Before formal execution, compare a small Core input fixture against the WCM
fixture field-by-field and store a stable fixture hash.

## 13. Output validation

Validation is outside H3 and mandatory for every sample during smoke tests.
For formal runs, at minimum validate every warm-up and the first measured call
per batch, then verify size/hash for all remaining calls.

### Parse

- output entry count equals batch count;
- every timestamp, level, message, tag, and targettag matches WCM;
- serialized logical result hash matches the reference.

### Filter

- result count equals batch count;
- IDs preserve input order;
- exactly half are kept for the alternating fixture;
- logical result hash matches WCM.

### Format

- Base64 decodes successfully;
- frame parser consumes the complete buffer;
- frame count equals batch count;
- every frame byte matches WCM output;
- framed-output hash matches WCM.

Never accept timing data from an incorrect output.

## 14. Memory measurement

Track at least:

- initial memory pages;
- H0 memory pages;
- H3 memory pages;
- peak pages through a Wasmtime resource limiter if practical;
- memory growth count and delta during H0-H1, H1-H2, and H2-H3.

Since every instance is fresh, growth on every sample can be normal. Interpret
it as per-call allocation demand, not cross-call accumulation.

Use the same 256 MiB per-instance limit as the WCM copied-src benchmark unless
a smaller explicit limit is proven sufficient for batch 8192.

## 15. CSV schema

The first implementation should emit:

```text
stage,
batch_count,
round,
logical_input_bytes,
abi_input_bytes,
abi_output_bytes,
output_entries,
input_boundary_ns,
guest_ns,
output_boundary_ns,
total_ns,
copy_in_ns,
copy_out_ns,
wasm_grow_count,
wasm_grow_delta_bytes,
wasm_mem_peak_bytes,
instance_mode,
module_mode,
marker_count,
fixture_hash,
output_hash
```

Constant labels:

```text
instance_mode = fresh-pool
module_mode   = core-wasm-json
marker_count  = 2
```

Enforce for every sample:

```text
input_boundary_ns + guest_ns + output_boundary_ns == total_ns
copy_in_ns == input_boundary_ns
copy_out_ns == output_boundary_ns
```

## 16. Build alignment

Host release profile:

```toml
[profile.release]
opt-level = 3
lto = "thin"
codegen-units = 1
```

Guest parse should initially match its WCM component profile:

```toml
opt-level = "s"
lto = "fat"
codegen-units = 1
strip = true
```

Guest filter should use the same profile as WCM filter. Guest format should use
`opt-level = 3`, matching WCM format. All guest crates should use
`panic = "abort"` if compatible with the selected toolchain.

Core guest dependencies will include `serde`, `serde_json`, and, for format,
`base64`. JSON ABI work is intentionally outside H1-H2 but remains part of the
measured boundary.

## 17. Execution scripts

`run_one.sh` should accept:

```text
STAGE BATCH ROUNDS WARMUP
```

Example future command:

```bash
CPU=2 ./scripts/run_one.sh parse 64 5 2
```

`run_all.sh` should default to:

```text
stages: parse filter format
runs: 3
batches: 64 128 256 512 1024 2048 4096 8192
rounds: 100
warm-up: 10
interval: 20 seconds between complete stage/run sweeps
```

It must write into a new timestamped result directory and atomically move a
CSV into its final filename only after line-count validation.

## 18. Implementation order

1. Create common explicit JSON types and golden JSON fixtures.
2. Implement and test alloc/dealloc with zero-length and normal buffers.
3. Implement the time-sign import and H0-H3 marker validation.
4. Implement parse guest and host path.
5. Compare parse H1-H2 output with Rust WCM parse.
6. Implement filter guest and host path.
7. Implement format guest, Base64 response, and byte-for-byte validation.
8. Add fresh-instance pool and memory limiter.
9. Add smoke runner for all three stages at batch 64.
10. Add the full batch/run orchestration script.
11. Run formula, marker, output-hash, and memory checks.
12. Only after all checks pass, run the formal experiment.

## 19. Smoke-test acceptance criteria

Before formal measurement, all of these must pass:

- all three core modules pass `wasm-tools validate`;
- modules are core Wasm modules, not components;
- no WIT/component-model custom section is required for execution;
- host uses Wasmtime core module APIs only;
- parse/filter/format each complete batch 64 with 3 measured calls and 1
  warm-up;
- every sample has exactly two markers;
- interval sum equals total exactly;
- every result matches the WCM reference logically;
- format output matches WCM byte-for-byte after Base64 decode;
- fresh instance is never reused;
- original repository `src` remains unchanged.

## 20. Required interpretation in the paper

Use these names:

```text
Core Wasm JSON input boundary
Core Wasm guest logic
Core Wasm JSON output boundary
WCM canonical input boundary
WCM guest logic
WCM canonical output boundary
```

Do not label the Core JSON boundary as a simple memory copy. It contains JSON
encoding/decoding. Do not label the WCM H0-H1 interval as a simple memory copy
either; it contains canonical lowering, allocation, bindings, and possible
runtime initialization.

The strongest supported comparison will be:

> For the same logical workload and Rust guest business implementation, compare
> the observed cost of a typed WCM canonical boundary with a manually defined
> Core Wasm JSON boundary, while keeping H1-H2 business logic equivalent.
