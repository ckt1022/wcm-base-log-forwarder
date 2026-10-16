const fs = require("node:fs");
const path = require("node:path");
const stage = process.argv[2] || "parse";

if (!["parse", "filter", "format"].includes(stage)) {
  throw new Error(`unsupported stage: ${stage}`);
}
const stageTitle = stage[0].toUpperCase() + stage.slice(1);

const root = path.resolve(__dirname, "..");
const outputDir = path.join(
  __dirname,
  "results/wcm_isolated_20260817_152531/analysis",
);
const methods = [
  {
    key: "wcm",
    label: "WCM Canonical ABI",
    color: "#0072B2",
    dash: "",
    source: "wcm_isolated_20260817_152531",
    dir: path.join(__dirname, "results/wcm_isolated_20260817_152531"),
    file: (run) => `${stage}_rust_isolated_run${run}.csv`,
    inputField: "copy_in_ns",
    outputField: "copy_out_ns",
  },
  {
    key: "json",
    label: "Core JSON ABI",
    color: "#D55E00",
    dash: "12 7",
    source: "core_wasm_json_20260817_142304",
    dir: path.join(
      root,
      "core_wasm_json_benchmark/results/core_wasm_json_20260817_142304",
    ),
    file: (run) => `${stage}_rust_core_wasm_json_run${run}.csv`,
    inputField: "input_boundary_ns",
    outputField: "output_boundary_ns",
  },
  {
    key: "direct",
    label: "Core Direct Memory ABI",
    color: "#009E73",
    dash: "3 6",
    source: "core_wasm_direct_memory_20260817_163933",
    dir: path.join(
      root,
      "core_wasm_binary_benchmark/results/core_wasm_direct_memory_20260817_163933",
    ),
    file: (run) => `${stage}_rust_core_wasm_direct_memory_run${run}.csv`,
    inputField: "input_boundary_ns",
    outputField: "output_boundary_ns",
  },
];

function parseCsv(filePath) {
  const lines = fs.readFileSync(filePath, "utf8").trim().split(/\r?\n/);
  const headers = lines.shift().split(",");
  return lines.map((line) => {
    const values = line.split(",");
    if (values.length !== headers.length) throw new Error(`CSV width mismatch: ${filePath}`);
    return Object.fromEntries(headers.map((header, index) => [header, values[index]]));
  });
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

const unifiedRows = [];
for (const method of methods) {
  for (let run = 1; run <= 3; run += 1) {
    const rows = parseCsv(path.join(method.dir, method.file(run)));
    for (const row of rows) {
      const batch = Number(row.batch_count);
      if (method.key !== "wcm") {
        if (row.stage !== stage || row.instance_mode !== "fresh-pool" || row.marker_count !== "2") {
          throw new Error(`invalid metadata: ${method.key}/run${run}/batch${batch}`);
        }
      } else if (row.instance_mode !== "fresh-pool") {
        throw new Error(`WCM sample is not fresh-pool: run${run}/batch${batch}`);
      }
      unifiedRows.push({
        method: method.key,
        method_label: method.label,
        source: method.source,
        independent_run: run,
        batch_count: batch,
        round: Number(row.round),
        logical_input_bytes: batch * 1024,
        abi_input_bytes: row.abi_input_bytes || "",
        abi_output_bytes: row.abi_output_bytes || "",
        copy_in_ns: Number(row[method.inputField]),
        guest_ns: Number(row.guest_ns),
        copy_out_ns: Number(row[method.outputField]),
        total_ns: Number(row.total_ns),
        wasm_grow_count: Number(row.wasm_grow_count),
        wasm_grow_delta_bytes: Number(row.wasm_grow_delta_bytes),
        wasm_mem_peak_bytes: Number(row.wasm_mem_peak_bytes),
        instance_mode: row.instance_mode,
        module_mode: row.module_mode || "component-canonical-abi",
        marker_count: row.marker_count || 2,
      });
    }
  }
}

fs.mkdirSync(outputDir, { recursive: true });
const rawHeader = Object.keys(unifiedRows[0]);
const rawLines = [rawHeader.join(",")];
for (const row of unifiedRows) rawLines.push(rawHeader.map((field) => row[field]).join(","));
fs.writeFileSync(path.join(outputDir, `${stage}_three_isolated_abis_raw.csv`), `${rawLines.join("\n")}\n`);

const batches = [64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768];
const summaryRows = [];
for (const method of methods) {
  for (const batch of batches) {
    const rows = unifiedRows.filter((row) => row.method === method.key && row.batch_count === batch);
    if (rows.length !== 300) {
      throw new Error(`${method.key}/${stage}/${batch}: expected 300 samples, got ${rows.length}`);
    }
    const item = {
      method: method.key,
      method_label: method.label,
      source: method.source,
      batch_count: batch,
      logical_input_bytes: batch * 1024,
      input_mib: batch / 1024,
      samples: rows.length,
    };
    for (const field of ["copy_in_ns", "copy_out_ns"]) {
      const stat = statistics(rows.map((row) => row[field]));
      item[`${field}_median_ms`] = stat.median / 1e6;
      item[`${field}_p5_ms`] = stat.p5 / 1e6;
      item[`${field}_p95_ms`] = stat.p95 / 1e6;
      item[`${field}_mean_ms`] = stat.mean / 1e6;
      item[`${field}_stddev_ms`] = stat.stddev / 1e6;
      item[`${field}_cv`] = stat.stddev / stat.mean;
      item[`${field}_min_ms`] = stat.min / 1e6;
      item[`${field}_max_ms`] = stat.max / 1e6;
      for (let run = 1; run <= 3; run += 1) {
        item[`${field}_run${run}_median_ms`] = quantile(
          rows.filter((row) => row.independent_run === run).map((row) => row[field]),
          0.5,
        ) / 1e6;
      }
    }
    summaryRows.push(item);
  }
}

const summaryHeader = Object.keys(summaryRows[0]);
const summaryLines = [summaryHeader.join(",")];
for (const row of summaryRows) summaryLines.push(summaryHeader.map((field) => row[field]).join(","));
fs.writeFileSync(path.join(outputDir, `${stage}_three_isolated_abis_summary.csv`), `${summaryLines.join("\n")}\n`);

function inputLabel(bytes) {
  if (bytes < 1024 * 1024) return `${bytes / 1024} KiB`;
  return `${bytes / (1024 * 1024)} MiB`;
}

function createChart(field, title, fileName) {
  const width = 1280;
  const height = 790;
  const margin = { top: 108, right: 55, bottom: 145, left: 120 };
  const plotWidth = width - margin.left - margin.right;
  const plotHeight = height - margin.top - margin.bottom;
  const values = summaryRows.flatMap((row) => [row[`${field}_p5_ms`], row[`${field}_p95_ms`]]);
  const logMin = Math.floor(Math.log10(Math.min(...values))) - 0.08;
  const logMax = Math.ceil(Math.log10(Math.max(...values))) + 0.08;
  const firstX = Math.log2(64 * 1024);
  const lastX = Math.log2(32768 * 1024);
  const x = (row) => margin.left + ((Math.log2(row.logical_input_bytes) - firstX) / (lastX - firstX)) * plotWidth;
  const y = (milliseconds) => margin.top + ((logMax - Math.log10(milliseconds)) / (logMax - logMin)) * plotHeight;
  const parts = [];
  parts.push(`<?xml version="1.0" encoding="UTF-8"?>`);
  parts.push(`<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" role="img" aria-labelledby="title desc">`);
  parts.push(`<title id="title">${title}</title>`);
  parts.push(`<desc id="desc">Isolated ${stageTitle} boundary latency for WCM Canonical ABI, Core JSON ABI, and Core Direct Memory ABI. Both axes are logarithmic. Lines are medians of 300 samples and bands span p5 to p95.</desc>`);
  parts.push(`<rect width="100%" height="100%" fill="#ffffff"/>`);
  parts.push(`<style>
    text { font-family: Inter, "DejaVu Sans", Arial, sans-serif; fill: #1f2937; }
    .title { font-size: 29px; font-weight: 700; }
    .subtitle { font-size: 15px; fill: #59636e; }
    .axis-label { font-size: 18px; font-weight: 600; }
    .tick { font-size: 13px; fill: #4b5563; }
    .legend { font-size: 15px; font-weight: 600; }
    .grid { stroke: #dfe5eb; stroke-width: 1; }
    .axis { stroke: #26313d; stroke-width: 1.5; }
  </style>`);
  parts.push(`<text class="title" x="${margin.left}" y="43">${title}</text>`);
  parts.push(`<text class="subtitle" x="${margin.left}" y="70">Rust host + Rust guest · isolated fresh instance · median and p5–p95 of 300 samples</text>`);
  parts.push(`<text class="subtitle" x="${margin.left}" y="91">Logical input: 1024 B per log · both axes logarithmic</text>`);

  for (let exponent = Math.ceil(logMin); exponent <= Math.floor(logMax); exponent += 1) {
    for (const multiplier of [1, 2, 5]) {
      const value = multiplier * 10 ** exponent;
      const logValue = Math.log10(value);
      if (logValue < logMin || logValue > logMax) continue;
      const py = y(value);
      parts.push(`<line class="grid" x1="${margin.left}" y1="${py}" x2="${width - margin.right}" y2="${py}" opacity="${multiplier === 1 ? 1 : 0.45}"/>`);
      const label = value < 0.1 ? value.toFixed(2) : value < 1 ? value.toFixed(1) : value;
      parts.push(`<text class="tick" x="${margin.left - 14}" y="${py + 5}" text-anchor="end">${label}</text>`);
    }
  }
  batches.forEach((batch) => {
    const row = { logical_input_bytes: batch * 1024 };
    const px = x(row);
    parts.push(`<line class="grid" x1="${px}" y1="${margin.top}" x2="${px}" y2="${margin.top + plotHeight}" opacity="0.5"/>`);
    parts.push(`<text class="tick" x="${px}" y="${margin.top + plotHeight + 29}" text-anchor="middle">${inputLabel(row.logical_input_bytes)}</text>`);
  });
  parts.push(`<line class="axis" x1="${margin.left}" y1="${margin.top + plotHeight}" x2="${width - margin.right}" y2="${margin.top + plotHeight}"/>`);
  parts.push(`<line class="axis" x1="${margin.left}" y1="${margin.top}" x2="${margin.left}" y2="${margin.top + plotHeight}"/>`);
  parts.push(`<text class="axis-label" x="${margin.left + plotWidth / 2}" y="${height - 62}" text-anchor="middle">Logical input size (log₂ scale)</text>`);
  parts.push(`<text class="axis-label" transform="translate(31 ${margin.top + plotHeight / 2}) rotate(-90)" text-anchor="middle">Boundary latency (ms, log₁₀ scale)</text>`);

  for (const method of methods) {
    const rows = summaryRows.filter((row) => row.method === method.key);
    const upper = rows.map((row) => `${x(row)},${y(row[`${field}_p95_ms`])}`);
    const lower = rows.slice().reverse().map((row) => `${x(row)},${y(row[`${field}_p5_ms`])}`);
    parts.push(`<polygon points="${[...upper, ...lower].join(" ")}" fill="${method.color}" opacity="0.09"/>`);
    const points = rows.map((row) => `${x(row)},${y(row[`${field}_median_ms`])}`).join(" ");
    const dash = method.dash ? ` stroke-dasharray="${method.dash}"` : "";
    parts.push(`<polyline points="${points}" fill="none" stroke="${method.color}" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"${dash}/>`);
    rows.forEach((row) => {
      parts.push(`<circle cx="${x(row)}" cy="${y(row[`${field}_median_ms`])}" r="4.5" fill="#ffffff" stroke="${method.color}" stroke-width="2.7"/>`);
    });
  }

  const legendY = height - 27;
  const legendX = [margin.left, margin.left + 340, margin.left + 645];
  methods.forEach((method, index) => {
    const dash = method.dash ? ` stroke-dasharray="${method.dash}"` : "";
    parts.push(`<line x1="${legendX[index]}" y1="${legendY}" x2="${legendX[index] + 48}" y2="${legendY}" stroke="${method.color}" stroke-width="4"${dash}/>`);
    parts.push(`<circle cx="${legendX[index] + 24}" cy="${legendY}" r="4" fill="#ffffff" stroke="${method.color}" stroke-width="2.5"/>`);
    parts.push(`<text class="legend" x="${legendX[index] + 60}" y="${legendY + 5}">${method.label}</text>`);
  });
  parts.push(`</svg>`);
  fs.writeFileSync(path.join(outputDir, fileName), `${parts.join("\n")}\n`);
}

createChart("copy_in_ns", `Isolated ${stageTitle} Copy-in: Three ABIs`, `${stage}_copy_in_three_isolated_abis_loglog.svg`);
createChart("copy_out_ns", `Isolated ${stageTitle} Copy-out: Three ABIs`, `${stage}_copy_out_three_isolated_abis_loglog.svg`);

console.log(outputDir);
