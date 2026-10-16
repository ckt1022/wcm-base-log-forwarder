# 五方法微基準：1024 筆（約 1 MiB）

| 階段 | 方法 | Copy-in | Logic | Copy-out | Total | Total p95 | 邊界占比 |
|---|---|---:|---:|---:|---:|---:|---:|
| Parse | WCM Canonical ABI | 0.863 ms | 2.126 ms | 1.354 ms | 4.399 ms | 6.465 ms | 50.3% |
| Parse | Core Wasm JSON ABI | 3.085 ms | 2.235 ms | 5.689 ms | 11.122 ms | 12.433 ms | 79.6% |
| Parse | Core Wasm Direct Memory | 0.803 ms | 2.304 ms | 1.090 ms | 4.220 ms | 5.143 ms | 45.1% |
| Parse | Native Rust (in-process) | 0.000 ms | 2.119 ms | 0.000 ms | 2.119 ms | 2.925 ms | 0.0% |
| Parse | Subprocess + pipe | 1.668 ms | 1.950 ms | 3.564 ms | 7.307 ms | 9.078 ms | 72.7% |
| Filter | WCM Canonical ABI | 1.790 ms | 0.002 ms | 0.143 ms | 1.945 ms | 4.759 ms | 99.9% |
| Filter | Core Wasm JSON ABI | 3.036 ms | 0.131 ms | 0.345 ms | 3.515 ms | 4.248 ms | 96.2% |
| Filter | Core Wasm Direct Memory | 0.700 ms | 0.110 ms | 0.015 ms | 0.828 ms | 0.987 ms | 86.2% |
| Filter | Native Rust (in-process) | 0.000 ms | 0.001 ms | 0.000 ms | 0.001 ms | 0.001 ms | 0.0% |
| Filter | Subprocess + pipe | 3.077 ms | 0.014 ms | 0.036 ms | 3.308 ms | 4.588 ms | 99.6% |
| Format | WCM Canonical ABI | 1.961 ms | 10.646 ms | 0.269 ms | 14.833 ms | 21.458 ms | 15.3% |
| Format | Core Wasm JSON ABI | 3.924 ms | 4.099 ms | 5.514 ms | 13.619 ms | 14.983 ms | 69.6% |
| Format | Core Wasm Direct Memory | 0.808 ms | 4.473 ms | 0.044 ms | 5.315 ms | 5.704 ms | 16.1% |
| Format | Native Rust (in-process) | 0.000 ms | 1.926 ms | 0.000 ms | 1.926 ms | 2.584 ms | 0.0% |
| Format | Subprocess + pipe | 1.503 ms | 1.696 ms | 0.910 ms | 4.250 ms | 6.593 ms | 58.4% |

Subprocess child 冷啟動（spawn → 第一個 ping 回應）：n=60，中位數 0.856 ms，p95 1.285 ms。
