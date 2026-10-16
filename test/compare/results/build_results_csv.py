#!/usr/bin/env python3
"""Build the canonical experiment CSV from checked-in raw benchmark artifacts.

No benchmark is executed by this script. Missing measurements remain empty.
"""

from __future__ import annotations

import csv
import hashlib
import math
import re
import subprocess
from datetime import datetime, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
OUTPUT = ROOT / "實驗結果.csv"

COLUMNS = [
    "run_id", "experiment_type", "run_time", "product", "product_version",
    "image", "filter_version", "log_mode", "log_bytes", "field_count",
    "target_eps", "actual_eps", "input_mb_s", "duration_s",
    "steady_start_s", "steady_end_s", "cpu_limit_core", "mem_limit_mib",
    "swap_mib", "input_transport", "input_connections", "batch_bytes",
    "flush_ms", "workers", "concurrency", "retry", "timeout_ms",
    "received_count", "expected_count", "dropped_count", "backlog_count",
    "send_lag_p99_ms", "latency_mean_ms", "latency_min_ms",
    "latency_p50_ms", "latency_p90_ms", "latency_p95_ms",
    "latency_p99_ms", "latency_p99_9_ms", "latency_max_ms",
    "cpu_scope", "cpu_mean_pct", "cpu_max_pct", "mem_scope",
    "mem_mean_mib", "mem_max_mib", "verdict", "na_reason", "config_id",
    "config_path", "result_path", "data_quality", "notes",
]

PRODUCTS = {
    "wcm": {
        "product": "WCM Rust WASM",
        "version": "historical run commit not recorded; repository_current=" + subprocess.check_output(
            ["git", "rev-parse", "HEAD"], cwd=ROOT, text=True
        ).strip(),
        "image": "wcm-base-log-forwarder:latest (historical local image; digest not recorded)",
        "filter": "embedded Rust/WASM pipeline",
    },
    "fluentd": {
        "product": "Fluentd Ruby",
        "version": "1.16.3",
        "image": "fluent/fluentd:v1.16-debian-1 (digest not recorded)",
        "filter": "plugins/filter_latency_test.rb",
    },
    "vector": {
        "product": "Vector VRL",
        "version": "0.49.0",
        "image": "timberio/vector:0.49.0-alpine (digest not recorded)",
        "filter": "VRL transform in latency/capacity config",
    },
    "fbit-lua": {
        "product": "Fluent Bit Lua",
        "version": "5.0.9",
        "image": "fluent/fluent-bit:5.0.9 (digest not recorded)",
        "filter": "test/compare/fluentbit/latency/latency_filter.lua",
    },
    "fbit-wasm": {
        "product": "Fluent Bit WASM",
        "version": "5.0.9",
        "image": "fluent/fluent-bit:5.0.9 (digest not recorded)",
        "filter": "test/compare/fluentbit/latency/wasm_file/latency_filter.wasm",
    },
}


def sha256(path: Path) -> str:
    if not path.exists():
        return ""
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def rel(path: Path) -> str:
    return str(path.resolve().relative_to(ROOT))


def fmt(value):
    if value is None or value == "":
        return ""
    if isinstance(value, float):
        return f"{value:.3f}"
    return value


def percentile(values: list[float], p: float) -> float:
    k = (len(values) - 1) * p / 100.0
    lo = math.floor(k)
    hi = min(lo + 1, len(values) - 1)
    return values[lo] + (k - lo) * (values[hi] - values[lo])


def latency_from_csv(path: Path, start: float = 30, end: float = 230):
    samples = []
    with path.open(newline="") as stream:
        for row in csv.DictReader(stream):
            samples.append((float(row["receive_unix"]), float(row["latency_ms"])))
    if not samples:
        raise ValueError(f"empty latency CSV: {path}")
    t0 = min(item[0] for item in samples)
    values = sorted(latency for ts, latency in samples if start <= ts - t0 <= end)
    if not values:
        raise ValueError(f"empty steady window: {path}")
    return {
        "received_count": len(samples),
        "expected_count": len(samples),
        "latency_mean_ms": sum(values) / len(values),
        "latency_min_ms": values[0],
        "latency_p50_ms": percentile(values, 50),
        "latency_p90_ms": percentile(values, 90),
        "latency_p95_ms": percentile(values, 95),
        "latency_p99_ms": percentile(values, 99),
        "latency_p99_9_ms": percentile(values, 99.9),
        "latency_max_ms": values[-1],
        "notes": f"steady_window_records={len(values)}; total_records={len(samples)}",
    }


def resources_from_csv(path: Path):
    if not path.exists():
        return {}
    cpu, memory = [], []
    with path.open(newline="") as stream:
        for row in csv.DictReader(stream):
            try:
                cpu.append(float(row["cpu_pct"]))
                memory.append(float(row["rss_mb"]))
            except (KeyError, TypeError, ValueError):
                continue
    if not cpu or not memory:
        return {}
    return {
        "cpu_scope": "DUT processing process sampled once per second",
        "cpu_mean_pct": sum(cpu) / len(cpu),
        "cpu_max_pct": max(cpu),
        "mem_scope": "DUT processing process RSS sampled once per second",
        "mem_mean_mib": sum(memory) / len(memory),
        "mem_max_mib": max(memory),
    }


def base_row(product_key: str):
    product = PRODUCTS[product_key]
    filter_version = product["filter"]
    if filter_version.startswith("test/"):
        filter_path = ROOT / filter_version
        filter_version += "; sha256=" + sha256(filter_path)
    row = {column: "" for column in COLUMNS}
    row.update({
        "product": product["product"],
        "product_version": product["version"],
        "image": product["image"],
        "filter_version": filter_version,
        "cpu_limit_core": 1,
        "mem_limit_mib": 512,
        "swap_mib": 0,
        "input_transport": "TCP",
        "input_connections": 1,
        "workers": 1,
        "concurrency": 1,
        "retry": "disabled",
    })
    return row


def add_latency_run(rows, product_key, csv_path, stats_path, flush_s, rate,
                    mode="160b_five_cycle", size=160, fields="varies",
                    batch_bytes=134217728, config_path=""):
    if not csv_path.exists():
        return
    row = base_row(product_key)
    run_name = csv_path.parent.name
    row.update({
        "run_id": run_name,
        "experiment_type": "low_rate_flush_latency",
        "run_time": datetime.fromtimestamp(csv_path.stat().st_mtime, timezone.utc).isoformat(),
        "log_mode": mode,
        "log_bytes": size,
        "field_count": fields,
        "target_eps": rate,
        "actual_eps": rate,
        "input_mb_s": rate * size / 1_000_000,
        "duration_s": 300,
        "steady_start_s": 30,
        "steady_end_s": 230,
        "batch_bytes": batch_bytes,
        "flush_ms": int(flush_s * 1000),
        "verdict": "PASS",
        "config_path": config_path,
        "config_id": sha256(ROOT / config_path) if config_path else "",
        "result_path": rel(csv_path),
        "data_quality": "recalculated_from_raw_csv",
    })
    row.update(latency_from_csv(csv_path))
    resource = resources_from_csv(stats_path)
    row.update(resource)
    if not resource:
        row["notes"] += "; CPU/MEM raw samples unavailable"
    if product_key == "fluentd" and flush_s < 1:
        row["notes"] += "; Fluentd sub-second interval is truncated internally and polls near 100ms"
    rows.append(row)


def parse_latency_text(path: Path):
    values = {}
    aliases = {"mean": "latency_mean_ms", "min": "latency_min_ms", "p50": "latency_p50_ms",
               "p90": "latency_p90_ms", "p95": "latency_p95_ms", "p99": "latency_p99_ms",
               "p99.9": "latency_p99_9_ms", "max": "latency_max_ms"}
    for line in path.read_text(errors="replace").splitlines():
        match = re.search(r"\b(mean|min|p50|p90|p95|p99|p99\.9|max)\s*=\s*([0-9.]+)\s*ms", line)
        if match:
            values[aliases[match.group(1)]] = float(match.group(2))
    return values


def add_relative_latency(rows, product_key, rate, path, config_path):
    if not path.exists():
        return
    row = base_row(product_key)
    row.update({
        "run_id": f"{product_key}_relative80_{rate}",
        "experiment_type": "capacity_80pct_latency",
        "run_time": datetime.fromtimestamp(path.stat().st_mtime, timezone.utc).isoformat(),
        "log_mode": "160b_five_cycle",
        "log_bytes": 160,
        "field_count": "varies",
        "target_eps": rate,
        "actual_eps": rate,
        "input_mb_s": rate * 160 / 1_000_000,
        "duration_s": 300 if product_key != "wcm" else 90,
        "steady_start_s": 30 if product_key != "wcm" else 20,
        "steady_end_s": 230 if product_key != "wcm" else 80,
        "batch_bytes": 1048576 if product_key in {"wcm", "fluentd", "vector"} else 2097152,
        "flush_ms": 1000,
        "verdict": "PASS",
        "config_path": config_path,
        "config_id": sha256(ROOT / config_path),
        "result_path": rel(path),
        "data_quality": "imported_from_generated_latency_stats",
        "notes": "Relative-load result; do not use as an equal-EPS product ranking. CPU/MEM samples unavailable.",
    })
    row.update(parse_latency_text(path))
    rows.append(row)


CAPACITY = [
    # product, mode, bytes, fields, capacity, input MB/s, first fail, search file, result directory
    ("wcm", "160b_five_cycle", 160, "varies", 136000, 21.708, 140000,
     "test/compare/capacity/runs/wcm-v2/search_result_v3_final.txt", "test/compare/capacity/runs/wcm-b1/cap_rate_136000"),
    ("fluentd", "160b_five_cycle", 160, "varies", 152000, 24.273, 160000,
     "test/compare/capacity/runs/fluentd/search_result_160b_final.txt", "test/compare/capacity/runs/fluentd/cap_rate_152000_160b"),
    ("vector", "160b_five_cycle", 160, "varies", 66000, 10.537, 68000,
     "test/compare/capacity/runs/vector/search_result_160b.txt", "test/compare/capacity/runs/vector/cap_rate_66000_160b"),
    ("fbit-lua", "160b_five_cycle", 160, "varies", 288000, 45.989, 296000,
     "test/compare/capacity/runs/fbit-lua/search_result_160b.txt", "test/compare/capacity/runs/fbit-lua/cap_rate_288000_160b"),
    ("fbit-wasm", "160b_five_cycle", 160, "varies", 28000, 4.474, 29000,
     "test/compare/capacity/runs/fbit-wasm/search_result_160b.txt", "test/compare/capacity/runs/fbit-wasm/cap_rate_28000_160b"),
    ("wcm", "256b_multi_field", 256, "8-9", 90000, 23.129, 92500,
     "test/compare/capacity/runs/wcm-b1/search_result_fields256.txt", "test/compare/capacity/runs/wcm-b1/cap_rate_90000_fields256"),
    ("wcm", "256b_five_fields", 256, "5", 120000, 30.839, 130000,
     "test/compare/capacity/runs/wcm-b1/search_result_few256.txt", "test/compare/capacity/runs/wcm-b1/cap_rate_120000_few256"),
    ("fluentd", "256b_multi_field", 256, "8-9", 138100, 35.491, 142100,
     "test/compare/capacity/runs/fluentd/search_result_fields256.txt", "test/compare/capacity/runs/fluentd/cap_rate_138100_fields256"),
    ("fluentd", "256b_five_fields", 256, "5", 160000, 41.119, 165000,
     "test/compare/capacity/runs/fluentd/search_result_few256.txt", "test/compare/capacity/runs/fluentd/cap_rate_160000_few256"),
    ("fbit-lua", "256b_multi_field", 256, "8-9", 172500, 44.331, 180000,
     "test/compare/capacity/runs/fbit-lua/search_result_fields256.txt", "test/compare/capacity/runs/fbit-lua/cap_rate_172500_fields256"),
    ("fbit-lua", "256b_five_fields", 256, "5", 240000, 61.679, 247500,
     "test/compare/capacity/runs/fbit-lua/search_result_few256.txt", "test/compare/capacity/runs/fbit-lua/cap_rate_240000_few256"),
    ("vector", "256b_multi_field", 256, "8-9", 56000, 14.392, 58000,
     "test/compare/capacity/runs/vector/search_result_fields256.txt", "test/compare/capacity/runs/vector/cap_rate_56000_fields256"),
    ("vector", "256b_five_fields", 256, "5", 76000, 19.531, 78000,
     "test/compare/capacity/runs/vector/search_result_few256.txt", "test/compare/capacity/runs/vector/cap_rate_76000_few256"),
    ("fbit-wasm", "256b_multi_field", 256, "8-9", 19000, 4.883, 19500,
     "test/compare/capacity/runs/fbit-wasm/search_result_fields256.txt", "test/compare/capacity/runs/fbit-wasm/cap_rate_19000_fields256"),
    ("fbit-wasm", "256b_five_fields", 256, "5", 27000, 6.939, 28000,
     "test/compare/capacity/runs/fbit-wasm/search_result_few256.txt", "test/compare/capacity/runs/fbit-wasm/cap_rate_27000_few256"),
    ("wcm", "512b_multi_field", 512, "17-18", 46200, 23.700, 47500,
     "test/compare/capacity/runs/wcm-b1/search_result_fields512.txt", "test/compare/capacity/runs/wcm-b1/cap_rate_46200_fields512"),
    ("wcm", "512b_five_fields", 512, "5", 105000, 53.864, 110000,
     "test/compare/capacity/runs/wcm-b1/search_result_few512.txt", "test/compare/capacity/runs/wcm-b1/cap_rate_105000_few512"),
    ("fluentd", "512b_multi_field", 512, "17-18", 67500, 34.627, 69300,
     "test/compare/capacity/runs/fluentd/search_result_fields512.txt", "test/compare/capacity/runs/fluentd/cap_rate_67500_fields512"),
    ("fluentd", "512b_five_fields", 512, "5", 135000, 69.254, 138700,
     "test/compare/capacity/runs/fluentd/search_result_few512.txt", "test/compare/capacity/runs/fluentd/cap_rate_135000_few512"),
    ("fbit-lua", "512b_multi_field", 512, "17-18", 125000, 64.124, 130000,
     "test/compare/capacity/runs/fbit-lua/search_result_fields512.txt", "test/compare/capacity/runs/fbit-lua/cap_rate_125000_fields512"),
    ("fbit-lua", "512b_five_fields", 512, "5", 202500, 103.880, 210000,
     "test/compare/capacity/runs/fbit-lua/search_result_few512.txt", "test/compare/capacity/runs/fbit-lua/cap_rate_202500_few512"),
    ("vector", "512b_multi_field", 512, "17-18", 30000, 15.390, 31000,
     "test/compare/capacity/runs/vector/search_result_fields512.txt", "test/compare/capacity/runs/vector/cap_rate_30000_fields512"),
    ("vector", "512b_five_fields", 512, "5", 70000, 35.909, 72000,
     "test/compare/capacity/runs/vector/search_result_few512.txt", "test/compare/capacity/runs/vector/cap_rate_70000_few512"),
    ("fbit-wasm", "512b_multi_field", 512, "17-18", 10500, 5.386, 11000,
     "test/compare/capacity/runs/fbit-wasm/search_result_fields512.txt", "test/compare/capacity/runs/fbit-wasm/cap_rate_10500_fields512"),
    ("fbit-wasm", "512b_five_fields", 512, "5", 24000, 12.312, 25000,
     "test/compare/capacity/runs/fbit-wasm/search_result_few512.txt", "test/compare/capacity/runs/fbit-wasm/cap_rate_24000_few512"),
    ("wcm", "1024b_few_fields_large_payload", 1024, "few", 74000, 75.848, 76000,
     "test/compare/capacity/runs/wcm-b1/search_result_1k.txt", "test/compare/capacity/runs/wcm-b1/cap_rate_74000_1k"),
    ("fluentd", "1024b_few_fields_large_payload", 1024, "few", 74000, 75.848, 76000,
     "test/compare/capacity/runs/fluentd/search_result_1k.txt", "test/compare/capacity/runs/fluentd/cap_rate_74000_1k"),
    ("vector", "1024b_few_fields_large_payload", 1024, "few", 60000, 61.499, 62000,
     "test/compare/capacity/runs/vector/search_result_1k.txt", "test/compare/capacity/runs/vector/cap_rate_60000_1k"),
    ("fbit-lua", "1024b_few_fields_large_payload", 1024, "few", 140000, 143.497, 144000,
     "test/compare/capacity/runs/fbit-lua/search_result_1k.txt", "test/compare/capacity/runs/fbit-lua/cap_rate_140000_1k"),
    ("fbit-wasm", "1024b_few_fields_large_payload", 1024, "few", 19000, 19.475, 19500,
     "test/compare/capacity/runs/fbit-wasm/search_result_1k.txt", "test/compare/capacity/runs/fbit-wasm/cap_rate_19000_1k"),
    ("wcm", "1024b_multi_field", 1024, "36", 25000, 25.624, 25500,
     "test/compare/capacity/runs/wcm-b1/cap_rate_25000_fields1k_capacity_manual_r3_20260809/verdict.txt",
     "test/compare/capacity/runs/wcm-b1/cap_rate_25000_fields1k_capacity_manual_r1_20260809"),
    ("fluentd", "1024b_multi_field", 1024, "36", 52000, 53.299, 52500,
     "test/compare/capacity/runs/fluentd/cap_rate_52000_fields1k_capacity_manual_r3_20260809/verdict.txt",
     "test/compare/capacity/runs/fluentd/cap_rate_52000_fields1k_capacity_manual_r1_20260809"),
    ("vector", "1024b_multi_field", 1024, "36", 15000, 15.375, 15500,
     "test/compare/capacity/runs/vector/cap_rate_15000_fields1k_capacity_manual_r3_20260809/verdict.txt",
     "test/compare/capacity/runs/vector/cap_rate_15000_fields1k_capacity_manual_r1_20260809"),
    ("fbit-lua", "1024b_multi_field", 1024, "36", 67500, 69.186, 68000,
     "test/compare/capacity/runs/fbit-lua/cap_rate_67500_fields1k_capacity_manual_r3_20260809/verdict.txt",
     "test/compare/capacity/runs/fbit-lua/cap_rate_67500_fields1k_capacity_manual_r1_20260809"),
    ("fbit-wasm", "1024b_multi_field", 1024, "36", 5400, 5.535, 5500,
     "test/compare/capacity/runs/fbit-wasm/cap_rate_5400_fields1k_capacity_manual_r3_20260809/verdict.txt",
     "test/compare/capacity/runs/fbit-wasm/cap_rate_5400_fields1k_capacity_manual_r1_20260809"),
]


def add_capacity(rows):
    for product_key, mode, size, fields, capacity, mbps, first_fail, search, result_dir in CAPACITY:
        search_path = ROOT / search
        result_path = ROOT / result_dir
        row = base_row(product_key)
        if product_key == "wcm" and mode == "1024b_multi_field":
            row["image"] = "wcm-base-log-forwarder:memguard (manual run; digest not recorded)"
        row.update({
            "run_id": f"{product_key}_capacity_{mode}",
            "experiment_type": "capacity_search",
            "run_time": datetime.fromtimestamp(search_path.stat().st_mtime, timezone.utc).isoformat(),
            "log_mode": mode,
            "log_bytes": size,
            "field_count": fields,
            "target_eps": capacity,
            "actual_eps": capacity,
            "input_mb_s": mbps,
            "duration_s": 60,
            "batch_bytes": 1048576 if product_key in {"wcm", "fluentd", "vector"} else 2097152,
            "flush_ms": 1000,
            "verdict": "PASS",
            "config_path": "test/compare/capacity/capacity.sh",
            "config_id": sha256(ROOT / "test/compare/capacity/capacity.sh"),
            "result_path": rel(search_path),
            "data_quality": ("confirmed_capacity_from_manual_repetitions"
                             if "capacity_manual" in search else
                             "confirmed_capacity_from_search_result"),
            "notes": f"confirmed capacity; first_failed_eps={first_fail}; raw_run={rel(result_path) if result_path.exists() else result_dir}; CPU/MEM summary unavailable",
        })
        verdict = result_path / "verdict.txt"
        if verdict.exists():
            text = verdict.read_text(errors="replace")
            for key, column in (("gen_avg", "actual_eps"), ("lag_p99", "send_lag_p99_ms"),
                                ("sink_total", "received_count"), ("expected", "expected_count"),
                                ("backlog", "backlog_count")):
                match = re.search(rf"\b{key}=([0-9.]+)", text)
                if match:
                    row[column] = float(match.group(1))
        rows.append(row)


def main():
    rows = []
    flushes = (0.1, 0.5, 1, 2, 5, 10, 20)
    latency_products = [
        ("fluentd", "test/compare/fluentd/latency/fluentd_time", "fluentd_time", "test/compare/fluentd/latency/conf/latency.conf", 134217728),
        ("fbit-lua", "test/compare/fluentbit/latency/fluentbit_lua_time", "fluentbit_lua_time", "test/compare/fluentbit/latency/conf/fluentbit_latency.conf", 2097152),
        ("fbit-wasm", "test/compare/fluentbit/latency/fluentbit_wasm_time", "fluentbit_wasm_time", "test/compare/fluentbit/latency/conf/fluentbit_latency_wasm.conf", 2097152),
        ("wcm", "test/ours/latency-benchmark-Rust/Rust_time_500", "Rust_time", "test/ours/latency-benchmark-Rust/forwarder.yaml", 134217728),
    ]
    for product, base, prefix, config, batch in latency_products:
        for flush in flushes:
            token = f"{flush:g}"
            directory = ROOT / base / f"{prefix}_{token}_500"
            add_latency_run(rows, product, directory / f"{prefix}_{token}_500.csv",
                            directory / f"{prefix}_{token}_500_stats.csv", flush, 500,
                            batch_bytes=batch, config_path=config)

    wcm_modes = [
        ("test/ours/latency-benchmark-Rust/Rust_time_5000", 5000, "160b_five_cycle", 160, "varies"),
        ("test/ours/latency-benchmark-Rust/Rust_time_500_1024_less", 500, "1024b_few_fields_large_payload", 1024, "few"),
        ("test/ours/latency-benchmark-Rust/Rust_time_500_1024_large", 500, "1024b_multi_field", 1024, 36),
        ("test/ours/latency-benchmark-Rust/Rust_time_500_512_large", 500, "512b_multi_field", 512, "17-18"),
    ]
    for base, rate, mode, size, fields in wcm_modes:
        for flush in flushes:
            token = f"{flush:g}"
            directory = ROOT / base / f"Rust_time_{token}_{rate}"
            add_latency_run(rows, "wcm", directory / f"Rust_time_{token}_{rate}.csv",
                            Path("/nonexistent"), flush, rate, mode, size, fields,
                            134217728, "test/ours/latency-benchmark-Rust/forwarder.yaml")

    add_capacity(rows)
    relative = [
        ("wcm", 105600, "test/compare/capacity/runs/wcm-b1/lat_rate_105600/latency_stats.txt"),
        ("fluentd", 140800, "test/compare/capacity/runs/fluentd/lat_rate_140800/latency_stats.txt"),
        ("vector", 49600, "test/compare/capacity/runs/vector/lat_rate_49600/latency_stats.txt"),
        ("fbit-lua", 236800, "test/compare/capacity/runs/fbit-lua/lat_rate_236800/latency_stats.txt"),
        ("fbit-wasm", 19200, "test/compare/capacity/runs/fbit-wasm/lat_rate_19200/latency_stats.txt"),
    ]
    for product, rate, path in relative:
        add_relative_latency(rows, product, rate, ROOT / path, "test/compare/capacity/capacity.sh")

    rows.sort(key=lambda row: (row["experiment_type"], row["log_bytes"], row["product"], row["target_eps"], row["flush_ms"]))
    with OUTPUT.open("w", newline="", encoding="utf-8-sig") as stream:
        writer = csv.DictWriter(stream, fieldnames=COLUMNS, lineterminator="\n")
        writer.writeheader()
        for row in rows:
            writer.writerow({key: fmt(row.get(key, "")) for key in COLUMNS})
    print(f"wrote {len(rows)} rows to {OUTPUT}")


if __name__ == "__main__":
    main()
