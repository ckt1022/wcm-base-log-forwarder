#!/usr/bin/env node

const fs = require("fs");
const path = require("path");

const resultsDir = path.join(__dirname, "results");
const languages = ["rust", "c", "csharp"];
const fields = ["copy_in_ns", "guest_ns", "copy_out_ns", "total_ns"];

function readCsv(filename) {
  const lines = fs.readFileSync(path.join(resultsDir, filename), "utf8").trim().split(/\r?\n/);
  const header = lines[0].split(",");
  return lines.slice(1).map((line) => {
    const values = line.split(",");
    return Object.fromEntries(header.map((name, index) => [name, values[index]]));
  });
}

function quantile(values, fraction) {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.ceil((sorted.length - 1) * fraction)];
}

const allFiles = fs.readdirSync(resultsDir);
const output = [];

for (const language of languages) {
  const pattern = new RegExp(`^filter_${language}_fresh_run[123]_[0-9_]+\\.csv$`);
  const files = allFiles.filter((name) => pattern.test(name)).sort().slice(-3);
  if (files.length !== 3) throw new Error(`${language}: expected three runs, found ${files.length}`);
  const runs = files.map(readCsv);
  const batches = [...new Set(runs[0].map((row) => Number(row.batch_count)))].sort((a, b) => a - b);

  for (const batch of batches) {
    for (const field of fields) {
      const perRun = runs.map((rows) => rows
        .filter((row) => Number(row.batch_count) === batch)
        .map((row) => Number(row[field])));
      const pooled = perRun.flat();
      const runMedians = perRun.map((values) => quantile(values, 0.5));
      const median = quantile(pooled, 0.5);
      const q25 = quantile(pooled, 0.25);
      const q75 = quantile(pooled, 0.75);
      const p95 = quantile(pooled, 0.95);
      const p99 = quantile(pooled, 0.99);
      output.push({
        language,
        batch,
        field,
        runMedians,
        median,
        q25,
        q75,
        p95,
        p99,
        runSpreadPct: median ? (Math.max(...runMedians) - Math.min(...runMedians)) / median * 100 : 0,
        iqrPct: median ? (q75 - q25) / median * 100 : 0,
        p95OverMedian: median ? p95 / median : 0,
        p99OverMedian: median ? p99 / median : 0,
      });
    }
  }
}

const header = [
  "language", "batch_count", "stage", "run1_median_ns", "run2_median_ns", "run3_median_ns",
  "pooled_median_ns", "q25_ns", "q75_ns", "p95_ns", "p99_ns", "run_spread_pct", "iqr_pct",
  "p95_over_median", "p99_over_median",
];
const csv = [header.join(",")];
for (const item of output) {
  csv.push([
    item.language, item.batch, item.field,
    ...item.runMedians, item.median, item.q25, item.q75, item.p95, item.p99,
    item.runSpreadPct.toFixed(3), item.iqrPct.toFixed(3),
    item.p95OverMedian.toFixed(3), item.p99OverMedian.toFixed(3),
  ].join(","));
}
fs.writeFileSync(path.join(resultsDir, "filter_fresh_stability_summary.csv"), `${csv.join("\n")}\n`);

console.log("TOTAL");
console.log("language,batch,run1_ms,run2_ms,run3_ms,pooled_ms,run_spread_pct,iqr_pct,p95_over_median,p99_over_median");
for (const item of output.filter((item) => item.field === "total_ns" && item.batch > 0)) {
  console.log([
    item.language, item.batch,
    ...item.runMedians.map((value) => (value / 1e6).toFixed(4)),
    (item.median / 1e6).toFixed(4), item.runSpreadPct.toFixed(2), item.iqrPct.toFixed(2),
    item.p95OverMedian.toFixed(2), item.p99OverMedian.toFixed(2),
  ].join(","));
}

console.log("\nTOTAL_MINUS_BATCH0");
console.log("language,batch,run1_ms,run2_ms,run3_ms,median_ms,run_spread_pct");
for (const language of languages) {
  const baseline = output.find((item) =>
    item.language === language && item.batch === 0 && item.field === "total_ns");
  for (const item of output.filter((item) =>
    item.language === language && item.batch > 0 && item.field === "total_ns")) {
    const adjusted = item.runMedians.map((value, index) => value - baseline.runMedians[index]);
    const adjustedMedian = quantile(adjusted, 0.5);
    const adjustedSpread = adjustedMedian
      ? (Math.max(...adjusted) - Math.min(...adjusted)) / adjustedMedian * 100
      : 0;
    console.log([
      language,
      item.batch,
      ...adjusted.map((value) => (value / 1e6).toFixed(4)),
      (adjustedMedian / 1e6).toFixed(4),
      adjustedSpread.toFixed(2),
    ].join(","));
  }
}

console.log("\nLARGEST_STAGE_RUN_SPREAD");
console.log("language,batch,stage,run_spread_pct,iqr_pct,p95_over_median,p99_over_median");
for (const item of output
  .filter((item) => item.batch > 0)
  .sort((a, b) => b.runSpreadPct - a.runSpreadPct)
  .slice(0, 24)) {
  console.log([
    item.language, item.batch, item.field, item.runSpreadPct.toFixed(2), item.iqrPct.toFixed(2),
    item.p95OverMedian.toFixed(2), item.p99OverMedian.toFixed(2),
  ].join(","));
}

console.log("\nC_FRESH_WITHIN_RUN_TOTAL_QUARTERS_AND_GROWTH");
console.log("run,batch,q1_median_ms,q2_median_ms,q3_median_ms,q4_median_ms,measured_grow_samples,warmup_grow_count");
{
  const files = allFiles
    .filter((name) => /^filter_c_fresh_run[123]_[0-9_]+\.csv$/.test(name))
    .sort()
    .slice(-3);
  files.forEach((file, runIndex) => {
    const rows = readCsv(file);
    for (const batch of [512, 1024, 2048, 4096, 8192]) {
      const batchRows = rows.filter((row) => Number(row.batch_count) === batch);
      const quarterMedians = [0, 1, 2, 3].map((quarter) => quantile(
        batchRows.slice(quarter * 25, (quarter + 1) * 25).map((row) => Number(row.total_ns)),
        0.5,
      ));
      const growSamples = batchRows.filter((row) => Number(row.wasm_grow_count) > 0).length;
      console.log([
        runIndex + 1,
        batch,
        ...quarterMedians.map((value) => (value / 1e6).toFixed(4)),
        growSamples,
        batchRows[0].warmup_grow_count,
      ].join(","));
    }
  });
}
