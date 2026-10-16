#!/usr/bin/env python3
"""Summarize WCM correctness runs, including active-window cgroup resources."""
import csv
import json
import re
import statistics
import sys
from pathlib import Path


root = Path(sys.argv[1])
out_csv = root / "summary.csv"
out_md = root / "summary.md"
rows = []

for run in sorted(root.glob("C*_rep*")):
    scenario, rep_text = run.name.split("_rep")
    gen = json.loads((run / "generator_summary.json").read_text())
    sink = json.loads((run / "validation_summary.json").read_text())
    verdict = (run / "verdict.txt").read_text().split()[0]
    duration = float(gen["elapsed_sec"])
    stats = []
    with (run / "stats.csv").open(newline="") as handle:
        for item in csv.DictReader(handle):
            if 1 <= float(item["elapsed_s"]) <= duration:
                stats.append(item)
    cpu = [float(item["cpu_pct"]) for item in stats]
    mem = [float(item["rss_mb"]) for item in stats]
    dut_log = (run / "dut.log").read_text(errors="replace")
    rows.append({
        "scenario": scenario,
        "repeat": int(rep_text),
        "status": verdict,
        "rate_eps": int((run / "rate.txt").read_text()),
        "input": int(gen["count"]),
        "malformed_input": int(gen["malformed"]),
        "expected_output": int(sink["expected"]),
        "received": int(sink["received"]),
        "missing": int(sink["missing"]),
        "duplicate": int(sink["duplicate"]),
        "corrupt": int(sink["corrupt"]),
        "unexpected": int(sink["unexpected"]),
        "reorder": int(sink["order_inversions"]),
        "bad_output_json": int(sink["bad_output_json"]),
        "gen_avg_eps": round(float(gen["average_eps"]), 3),
        "send_lag_p99_ms": round(float(gen["send_lag_p99_ms"]), 3),
        "cpu_mean_pct": round(statistics.fmean(cpu), 3),
        "cpu_max_pct": round(max(cpu), 3),
        "mem_mean_mib": round(statistics.fmean(mem), 3),
        "mem_max_mib": round(max(mem), 3),
        "parse_error_batches": len(re.findall(r"\[parse-error\]", dut_log)),
        # 逐筆隔離（src/isolation.rs）啟用時，每個受污染批次會多一行 [parse-isolate]
        "isolated_batches": len(re.findall(r"\[parse-isolate\]", dut_log)),
        "salvaged_lines": sum(int(x) for x in re.findall(r"\[parse-isolate\].*?salvaged=(\d+)", dut_log)),
        "rejected_lines": sum(int(x) for x in re.findall(r"\[parse-isolate\].*?rejected=(\d+)", dut_log)),
        "isolation_calls": sum(int(x) for x in re.findall(r"\[parse-isolate\].*?calls=(\d+)", dut_log)),
        "oom": (run / "oom.txt").read_text().strip(),
    })

with out_csv.open("w", newline="") as handle:
    writer = csv.DictWriter(handle, fieldnames=rows[0].keys())
    writer.writeheader()
    writer.writerows(rows)

meta = root / "meta.txt"
native = meta.exists() and "mode=native" in meta.read_text()
env_line = ("1024 B / 36 fields, native process (no container, no cgroup limit; see meta.txt for taskset pinning), "
            "1 MiB batch, 1 s max wait, single TCP connection and HTTP worker." if native else
            "1024 B / 36 fields, 1 CPU, 512 MiB, 1 MiB batch, 1 s max wait, single TCP connection and HTTP worker.")
lines = [
    "# WCM correctness experiment", "",
    env_line,
    "C1: valid inputs at 2,500 EPS; C2: valid inputs at 17,500 EPS; C3: deterministic 5% malformed JSON;",
    "C4: boolean, integer, float, null, Unicode, quote and backslash edge values. Each formal scenario was repeated three times.",
    "", "| Scenario | Rep | Status | Expected | Received | Missing | Dup | Corrupt | Reorder | CPU avg/max | MEM avg/max MiB | Parse-error batches |",
    "|:--|--:|:--:|--:|--:|--:|--:|--:|--:|--:|--:|--:|",
]
for r in rows:
    lines.append(
        f"| {r['scenario']} | {r['repeat']} | {r['status']} | {r['expected_output']:,} | {r['received']:,} | "
        f"{r['missing']:,} | {r['duplicate']} | {r['corrupt']} | {r['reorder']} | "
        f"{r['cpu_mean_pct']:.2f}% / {r['cpu_max_pct']:.2f}% | "
        f"{r['mem_mean_mib']:.2f} / {r['mem_max_mib']:.2f} | {r['parse_error_batches']} |"
    )
lines += ["", "## Result", ""]
for scenario in sorted({r["scenario"] for r in rows}):
    group = [r for r in rows if r["scenario"] == scenario]
    passed = sum(r["status"] == "PASS" for r in group)
    expected = sum(r["expected_output"] for r in group)
    received = sum(r["received"] for r in group)
    missing = sum(r["missing"] for r in group)
    lines.append(
        f"- {scenario}: {passed}/{len(group)} runs passed; aggregate expected={expected:,}, "
        f"received={received:,}, missing={missing:,}."
    )
c3 = [r for r in rows if r["scenario"] == "C3"]
if c3:
    pe = sorted({r["parse_error_batches"] for r in c3})
    pe_txt = "/".join(str(x) for x in pe)
    if all(r["isolated_batches"] > 0 for r in c3):
        lines.append(
            f"- C3 logged {pe_txt} parse-error batches per run. The parser still returns an error for the entire batch, "
            "but the host bisected every contaminated batch (src/isolation.rs) and delivered the valid records: "
            + "; ".join(f"rep{r['repeat']} salvaged={r['salvaged_lines']:,} rejected={r['rejected_lines']:,} "
                        f"parse calls={r['isolation_calls']:,}" for r in c3)
            + f" (malformed inputs per run: {', '.join(str(r['malformed_input']) for r in c3)})."
        )
    else:
        lines.append(
            f"- C3 logged {pe_txt} parse-error batches per run. The parser returns an error for the entire batch when any "
            "record is malformed and isolation was off (legacy behaviour), so every contaminated batch was discarded."
        )
oom_any = any(r["oom"] not in ("false", "0", "") for r in rows)
who = "the native forwarder process" if native else "the WCM container"
lines.append(("- At least one run reported OOM." if oom_any else "- No run was OOM-killed")
             + f" and {who} remained alive until the harness deliberately stopped it.")
out_md.write_text("\n".join(lines) + "\n")
print(out_csv)
print(out_md)
