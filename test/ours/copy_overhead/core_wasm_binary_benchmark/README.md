# Rust Core Wasm direct-memory ABI benchmark

This directory now implements the shortest Core Wasm path. Historical
`core_wasm_binary_*` result directories were produced by the former owned
binary-serialization implementation and must not be combined with new data.
New CSV rows use `module_mode=core-wasm-direct-memory`.

```text
H0 host before guest allocation and direct memory write
H1 guest after opening the input as the Direct Memory ABI's zero-copy
`RecordViews`/`EntryViews` (the payload is already in linear memory; parse,
filter, and format work starts after this marker)
H2 guest after the complete business result is constructed
H3 host after raw response bytes are copied out and both buffers are freed
```

The measured path has no request JSON, Base64, temporary host request `Vec`,
zero-filled guest input buffer, or guest-side owned reconstruction of input
strings/entries. The host writes offset tables and payload bytes directly into
guest linear memory. The guest reads them through borrowed views.

Parse and filter encode their business results after H2. Format output is
already the business result. The host copies raw response bytes back and
compares them against a reference prepared before H0; it does not rebuild host
Rust result structures inside H2-H3. This deliberately measures the minimum
Core Wasm ABI path rather than API-equivalent owned lifting.

The Parse response encoder computes the exact compact-binary length first and
uses one exactly sized allocation. It then writes every length prefix and
payload byte once. This avoids geometric `Vec` growth and prevents
`into_boxed_slice()` from needing to shrink the response allocation.

The first formal three-run result using this encoder is
`results/core_wasm_direct_memory_20260817_010847`. Direct Parse results from
earlier directories use the superseded growing-`Vec` encoder and must not be
combined with the new result.

## Run

```bash
cd /home/ckt1022/wcm-base-log-forwarder/test/ours/copy_overhead/core_wasm_binary_benchmark
./scripts/build.sh
CPU=2 ./scripts/run_smoke.sh
CPU=2 ./scripts/run_all.sh
```

The formal run performs three stages times three runs over ten power-of-two
batch sizes (64 through 32768), 100 measured samples plus 10 warm-ups per
batch, with a 20-second interval between sweeps. The benchmark memory limit is
512 MiB for the two added high-volume points.

After changing only one stage, select it without rerunning the others:

```bash
STAGES=parse CPU=2 ./scripts/run_all.sh
```
