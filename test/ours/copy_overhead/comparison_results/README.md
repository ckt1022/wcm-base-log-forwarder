# WCM, Core JSON, and Core Direct ABI comparison

`wcm_core_json_direct_comparison.csv` is generated from three independent runs
per method and stage. Every reported point is the combined median of 300
samples; `total_p95_ns` uses the nearest-rank p95 across the same 300 samples.

Current raw sources:

| Method/stage | Raw result directory |
|---|---|
| WCM Rust, all stages | `src_pipeline_benchmark/results/all_stages_20260815_230405` |
| Core JSON Rust, all stages | `core_wasm_json_benchmark/results/core_wasm_json_20260816_161029` |
| Core Direct Rust, Parse | `core_wasm_binary_benchmark/results/core_wasm_direct_memory_20260817_010847` |
| Core Direct Rust, Filter/Format | `core_wasm_binary_benchmark/results/core_wasm_direct_memory_20260816_194459` |

The 2026-08-17 Direct Parse source uses the exact-size, single-allocation
compact-binary response encoder. Earlier Direct Parse runs are superseded and
must not be combined with it.

Regenerate the comparison and Parse/Filter/Format boundary charts from raw CSV files:

```bash
node test/ours/copy_overhead/comparison_results/generate_abi_comparison.js
node test/ours/copy_overhead/comparison_results/generate_parse_boundary_charts.js
```
