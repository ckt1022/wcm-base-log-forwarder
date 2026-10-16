# WCM burst experiment figures

All figures are vector SVG files and can be inserted directly into a thesis.

1. `01_latency_timeline.svg`: p50, p99, and p99.9 latency around burst onset. The yellow region is the nominal burst interval; the green dashed line marks the first recovered second.
2. `02_throughput_timeline.svg`: actual source send rate and filtered output rate. WCM intentionally retains two of every five deterministic records in this workload.
3. `03_queue_timeline.svg`: output backlog and source-side pending records, showing queue accumulation and drainage.
4. `04_burst_summary.svg`: relationship between burst duration, maximum latency, recovery time, and queue depth.
5. `05_resource_summary.svg`: container CPU and resident-memory mean and maximum.

Suggested thesis caveat: each scenario currently has one run. In B3 the generator's send-lag p99 was 10.58 s, so its nominal 33,500 EPS for 30 s was not a perfectly rectangular input pulse. Use B3 as evidence of recovery from accumulated pressure, and repeat it with a better-isolated generator before making precise duration-effect claims.
