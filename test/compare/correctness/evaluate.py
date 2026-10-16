#!/usr/bin/env python3
"""Evaluate one WCM correctness run with strict zero-error criteria."""
import json
import sys
from pathlib import Path


run = Path(sys.argv[1])
gen = json.loads((run / "generator_summary.json").read_text())
sink = json.loads((run / "validation_summary.json").read_text())
oom = (run / "oom.txt").read_text().strip()
pre_stop = json.loads((run / "pre_stop_state.json").read_text())

checks = {
    "generator_count": gen["count"] == int((run / "input_count.txt").read_text()),
    "record_size": gen["size_errors"] == 0,
    "count": sink["received"] == sink["expected"] == sink["unique"],
    "missing": sink["missing"] == 0,
    "duplicate": sink["duplicate"] == 0,
    "corrupt": sink["corrupt"] == 0,
    "unexpected": sink["unexpected"] == 0,
    "order": sink["order_inversions"] == 0,
    "output_json": sink["bad_output_json"] == 0,
    "oom": oom == "false",
    "container_alive": bool(pre_stop.get("Running")) and not bool(pre_stop.get("Restarting")),
}
status = "PASS" if all(checks.values()) else "FAIL"
failed = [name for name, ok in checks.items() if not ok]
line = (
    f"{status} expected={sink['expected']} received={sink['received']} "
    f"missing={sink['missing']} duplicate={sink['duplicate']} corrupt={sink['corrupt']} "
    f"unexpected={sink['unexpected']} reorder={sink['order_inversions']} "
    f"bad_json={sink['bad_output_json']} gen_avg={gen['average_eps']:.1f} "
    f"lag_p99={gen['send_lag_p99_ms']:.3f}ms oom={oom}"
)
if failed:
    line += " failed=" + ",".join(failed)
print(line)
(run / "verdict.txt").write_text(line + "\n")
sys.exit(0 if status == "PASS" else 1)
