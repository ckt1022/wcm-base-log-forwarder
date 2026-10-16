const fs = require("node:fs");
const path = require("node:path");

const resultName = process.argv[2] || "wcm_isolated_20260817_152531";
const benchmarkDir = __dirname;
const resultDir = path.join(benchmarkDir, "results", resultName);
const outputDir = path.join(resultDir, "analysis");
const fields = ["copy_in_ns", "copy_out_ns"];

function parseCsv(filePath) {
  const lines = fs.readFileSync(filePath, "utf8").trim().split(/\r?\n/);
  const headers = lines.shift().split(",");
  return {
    headers,
    rows: lines.map((line) => {
      const values = line.split(",");
      if (values.length !== headers.length) {
        throw new Error(`CSV width mismatch: ${filePath}`);
      }
      return Object.fromEntries(headers.map((header, index) => [header, values[index]]));
    }),
  };
}

function quantile(values, fraction) {
  const sorted = values.map(Number).sort((left, right) => left - right);
  const position = (sorted.length - 1) * fraction;
  const lower = Math.floor(position);
  const upper = Math.ceil(position);
  return sorted[lower] + (sorted[upper] - sorted[lower]) * (position - lower);
}

function statistics(values) {
  const numbers = values.map(Number);
  const mean = numbers.reduce((sum, value) => sum + value, 0) / numbers.length;
  const variance = numbers.reduce((sum, value) => sum + (value - mean) ** 2, 0) / numbers.length;
  return {
    mean,
    median: quantile(numbers, 0.5),
    p5: quantile(numbers, 0.05),
    p95: quantile(numbers, 0.95),
    stddev: Math.sqrt(variance),
    min: Math.min(...numbers),
    max: Math.max(...numbers),
  };
}

const runs = [];
let sourceHeaders;
for (let run = 1; run <= 3; run += 1) {
  const parsed = parseCsv(path.join(resultDir, `parse_rust_isolated_run${run}.csv`));
  sourceHeaders ||= parsed.headers;
  for (const row of parsed.rows) runs.push({ run, ...row });
}

fs.mkdirSync(outputDir, { recursive: true });
const rawHeader = ["independent_run", ...sourceHeaders];
const rawLines = [rawHeader.join(",")];
for (const row of runs) {
  rawLines.push(rawHeader.map((field) => row[field]).join(","));
}
fs.writeFileSync(path.join(outputDir, "parse_isolated_raw.csv"), `${rawLines.join("\n")}\n`);

const batches = [...new Set(runs.map((row) => Number(row.batch_count)))].sort((a, b) => a - b);
const summaryRows = [];
for (const batch of batches) {
  const rows = runs.filter((row) => Number(row.batch_count) === batch);
  const item = {
    batch_count: batch,
    input_bytes: batch * 1024,
    input_mib: batch / 1024,
    samples: rows.length,
  };
  for (const field of fields) {
    const stat = statistics(rows.map((row) => row[field]));
    const runMedians = [1, 2, 3].map((run) =>
      quantile(rows.filter((row) => row.run === run).map((row) => row[field]), 0.5),
    );
    item[`${field}_run1_median_ns`] = runMedians[0];
    item[`${field}_run2_median_ns`] = runMedians[1];
    item[`${field}_run3_median_ns`] = runMedians[2];
    item[`${field}_mean_ns`] = stat.mean;
    item[`${field}_median_ns`] = stat.median;
    item[`${field}_p5_ns`] = stat.p5;
    item[`${field}_p95_ns`] = stat.p95;
    item[`${field}_stddev_ns`] = stat.stddev;
    item[`${field}_cv`] = stat.stddev / stat.mean;
    item[`${field}_min_ns`] = stat.min;
    item[`${field}_max_ns`] = stat.max;
  }
  summaryRows.push(item);
}

const summaryHeader = Object.keys(summaryRows[0]);
const summaryLines = [summaryHeader.join(",")];
for (const row of summaryRows) {
  summaryLines.push(summaryHeader.map((field) => row[field]).join(","));
}
fs.writeFileSync(path.join(outputDir, "parse_isolated_summary.csv"), `${summaryLines.join("\n")}\n`);

const chartDataHeader = [
  "batch_count",
  "input_bytes",
  "input_mib",
  "samples",
  "copy_in_median_ms",
  "copy_in_p5_ms",
  "copy_in_p95_ms",
  "copy_out_median_ms",
  "copy_out_p5_ms",
  "copy_out_p95_ms",
];
const chartDataLines = [chartDataHeader.join(",")];
for (const row of summaryRows) {
  chartDataLines.push([
    row.batch_count,
    row.input_bytes,
    row.input_mib,
    row.samples,
    row.copy_in_ns_median_ns / 1e6,
    row.copy_in_ns_p5_ns / 1e6,
    row.copy_in_ns_p95_ns / 1e6,
    row.copy_out_ns_median_ns / 1e6,
    row.copy_out_ns_p5_ns / 1e6,
    row.copy_out_ns_p95_ns / 1e6,
  ].join(","));
}
fs.writeFileSync(path.join(outputDir, "parse_isolated_chart_data.csv"), `${chartDataLines.join("\n")}\n`);

function inputLabel(bytes) {
  if (bytes < 1024 * 1024) return `${bytes / 1024} KiB`;
  return `${bytes / (1024 * 1024)} MiB`;
}

function createChart(field, title, fileName) {
  const rows = summaryRows.filter((row) => row.batch_count > 0);
  const width = 1280;
  const height = 760;
  const margin = { top: 105, right: 55, bottom: 115, left: 120 };
  const plotWidth = width - margin.left - margin.right;
  const plotHeight = height - margin.top - margin.bottom;
  const values = rows.flatMap((row) => [row[`${field}_p5_ns`], row[`${field}_p95_ns`]])
    .map((value) => value / 1e6);
  const logMin = Math.floor(Math.log10(Math.min(...values))) - 0.08;
  const logMax = Math.ceil(Math.log10(Math.max(...values))) + 0.08;
  const x = (row) => {
    const first = Math.log2(rows[0].input_bytes);
    const last = Math.log2(rows.at(-1).input_bytes);
    return margin.left + ((Math.log2(row.input_bytes) - first) / (last - first)) * plotWidth;
  };
  const y = (milliseconds) =>
    margin.top + ((logMax - Math.log10(milliseconds)) / (logMax - logMin)) * plotHeight;
  const parts = [];
  parts.push(`<?xml version="1.0" encoding="UTF-8"?>`);
  parts.push(`<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" role="img" aria-labelledby="title desc">`);
  parts.push(`<title id="title">${title}</title>`);
  parts.push(`<desc id="desc">Isolated WCM Rust Parse boundary latency. Both input size and time use logarithmic scales. The line is the median of 300 samples and the band spans p5 to p95.</desc>`);
  parts.push(`<rect width="100%" height="100%" fill="#ffffff"/>`);
  parts.push(`<style>
    text { font-family: Inter, "DejaVu Sans", Arial, sans-serif; fill: #1f2937; }
    .title { font-size: 29px; font-weight: 700; }
    .subtitle { font-size: 15px; fill: #59636e; }
    .axis-label { font-size: 18px; font-weight: 600; }
    .tick { font-size: 13px; fill: #4b5563; }
    .grid { stroke: #dfe5eb; stroke-width: 1; }
    .axis { stroke: #26313d; stroke-width: 1.5; }
  </style>`);
  parts.push(`<text class="title" x="${margin.left}" y="43">${title}</text>`);
  parts.push(`<text class="subtitle" x="${margin.left}" y="70">Rust host + Rust component · fresh instance · median and p5–p95 of 300 samples</text>`);
  parts.push(`<text class="subtitle" x="${margin.left}" y="91">Input: deterministic 1024 B JSON records · source: ${resultName}</text>`);

  for (let exponent = Math.ceil(logMin); exponent <= Math.floor(logMax); exponent += 1) {
    for (const multiplier of [1, 2, 5]) {
      const value = multiplier * 10 ** exponent;
      const logValue = Math.log10(value);
      if (logValue < logMin || logValue > logMax) continue;
      const py = y(value);
      parts.push(`<line class="grid" x1="${margin.left}" y1="${py}" x2="${width - margin.right}" y2="${py}" opacity="${multiplier === 1 ? 1 : 0.45}"/>`);
      parts.push(`<text class="tick" x="${margin.left - 14}" y="${py + 5}" text-anchor="end">${value < 0.1 ? value.toFixed(2) : value < 1 ? value.toFixed(1) : value}</text>`);
    }
  }
  rows.forEach((row) => {
    const px = x(row);
    parts.push(`<line class="grid" x1="${px}" y1="${margin.top}" x2="${px}" y2="${margin.top + plotHeight}" opacity="0.5"/>`);
    parts.push(`<text class="tick" x="${px}" y="${margin.top + plotHeight + 29}" text-anchor="middle">${inputLabel(row.input_bytes)}</text>`);
  });
  parts.push(`<line class="axis" x1="${margin.left}" y1="${margin.top + plotHeight}" x2="${width - margin.right}" y2="${margin.top + plotHeight}"/>`);
  parts.push(`<line class="axis" x1="${margin.left}" y1="${margin.top}" x2="${margin.left}" y2="${margin.top + plotHeight}"/>`);
  parts.push(`<text class="axis-label" x="${margin.left + plotWidth / 2}" y="${height - 35}" text-anchor="middle">Logical input size (log₂ scale)</text>`);
  parts.push(`<text class="axis-label" transform="translate(31 ${margin.top + plotHeight / 2}) rotate(-90)" text-anchor="middle">Boundary latency (ms, log₁₀ scale)</text>`);

  const upper = rows.map((row) => `${x(row)},${y(row[`${field}_p95_ns`] / 1e6)}`);
  const lower = rows.slice().reverse().map((row) => `${x(row)},${y(row[`${field}_p5_ns`] / 1e6)}`);
  parts.push(`<polygon points="${[...upper, ...lower].join(" ")}" fill="#56B4E9" opacity="0.22"/>`);
  const medianPoints = rows.map((row) => `${x(row)},${y(row[`${field}_median_ns`] / 1e6)}`).join(" ");
  parts.push(`<polyline points="${medianPoints}" fill="none" stroke="#0072B2" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"/>`);
  rows.forEach((row) => {
    const px = x(row);
    const py = y(row[`${field}_median_ns`] / 1e6);
    parts.push(`<line x1="${px}" y1="${y(row[`${field}_p5_ns`] / 1e6)}" x2="${px}" y2="${y(row[`${field}_p95_ns`] / 1e6)}" stroke="#0072B2" stroke-width="1.5" opacity="0.7"/>`);
    parts.push(`<circle cx="${px}" cy="${py}" r="5" fill="#ffffff" stroke="#0072B2" stroke-width="3"/>`);
  });
  parts.push(`<rect x="${margin.left}" y="${height - 77}" width="28" height="13" fill="#56B4E9" opacity="0.22"/>`);
  parts.push(`<line x1="${margin.left}" y1="${height - 70}" x2="${margin.left + 28}" y2="${height - 70}" stroke="#0072B2" stroke-width="3"/>`);
  parts.push(`<text class="tick" x="${margin.left + 38}" y="${height - 65}">Median; shaded/error range = p5–p95</text>`);
  parts.push(`</svg>`);
  fs.writeFileSync(path.join(outputDir, fileName), `${parts.join("\n")}\n`);
}

createChart("copy_in_ns", "Isolated WCM Parse Copy-in Latency", "parse_isolated_copy_in_loglog.svg");
createChart("copy_out_ns", "Isolated WCM Parse Copy-out Latency", "parse_isolated_copy_out_loglog.svg");

console.log(outputDir);
