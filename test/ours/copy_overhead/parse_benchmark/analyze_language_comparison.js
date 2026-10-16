#!/usr/bin/env node

const fs = require("fs");
const path = require("path");

const benchmarkDir = __dirname;
const resultsDir = path.join(benchmarkDir, "results");
const inputs = {
  Rust: [
    "threshold_fresh_ascending_run1_20260815_163422.csv",
    "threshold_fresh_ascending_run2_20260815_163432.csv",
    "threshold_fresh_ascending_run3_20260815_163442.csv",
  ],
  C: [
    "c_parse_fresh_ascending_run1_20260815_191723.csv",
    "c_parse_fresh_ascending_run2_20260815_191737.csv",
    "c_parse_fresh_ascending_run3_20260815_191749.csv",
  ],
  "C#": [
    "csharp_parse_fresh_ascending_run1_20260815_194102.csv",
    "csharp_parse_fresh_ascending_run2_20260815_194210.csv",
    "csharp_parse_fresh_ascending_run3_20260815_194317.csv",
  ],
};
const colors = { Rust: "#2563eb", C: "#ea580c", "C#": "#059669" };
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
  const index = Math.ceil((sorted.length - 1) * fraction);
  return sorted[index];
}

const data = {};
for (const [language, files] of Object.entries(inputs)) {
  const rows = files.flatMap(readCsv);
  const zero = rows.filter((row) => Number(row.batch_count) === 0);
  const baseline = Object.fromEntries(fields.map((field) => [
    field,
    quantile(zero.map((row) => Number(row[field])), 0.5),
  ]));
  const batches = [...new Set(rows.map((row) => Number(row.batch_count)))].filter(Boolean).sort((a, b) => a - b);
  const stats = {};
  for (const batch of batches) {
    const group = rows.filter((row) => Number(row.batch_count) === batch);
    stats[batch] = {};
    for (const field of fields) {
      const values = group.map((row) => Number(row[field]));
      stats[batch][field] = {
        q25: quantile(values, 0.25),
        median: quantile(values, 0.5),
        q75: quantile(values, 0.75),
        p95: quantile(values, 0.95),
      };
    }
  }
  data[language] = { baseline, batches, stats };
}

const batches = data.Rust.batches;
const languages = Object.keys(inputs);
const esc = (value) => String(value).replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");

function linePath(points) {
  return points.map(([x, y], index) => `${index ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`).join(" ");
}

function areaPath(top, bottom) {
  const forward = linePath(top);
  const reverse = [...bottom].reverse().map(([x, y]) => `L${x.toFixed(1)},${y.toFixed(1)}`).join(" ");
  return `${forward} ${reverse} Z`;
}

function panel({ x, y, width, height, title, yLabel, value, scale = "linear", maxValue, minValue }) {
  const margin = { left: 72, right: 20, top: 44, bottom: 58 };
  const px = x + margin.left;
  const py = y + margin.top;
  const pw = width - margin.left - margin.right;
  const ph = height - margin.top - margin.bottom;
  const all = languages.flatMap((language) => batches.flatMap((batch) => {
    const v = value(language, batch);
    return [v.q25, v.median, v.q75].filter(Number.isFinite);
  }));
  const positive = all.filter((number) => number > 0);
  const lo = minValue ?? (scale === "log" ? Math.min(...positive) * 0.75 : 0);
  const hi = maxValue ?? Math.max(...all) * 1.12;
  const xAt = (index) => px + (pw * index) / (batches.length - 1);
  const yAt = (raw) => {
    const v = Math.max(raw, lo);
    if (scale === "log") {
      return py + ph - ph * (Math.log10(v) - Math.log10(lo)) / (Math.log10(hi) - Math.log10(lo));
    }
    return py + ph - ph * (v - lo) / (hi - lo);
  };

  let svg = `<g><rect x="${x}" y="${y}" width="${width}" height="${height}" rx="10" fill="#ffffff" stroke="#d1d5db"/>`;
  svg += `<text x="${x + width / 2}" y="${y + 26}" text-anchor="middle" font-size="18" font-weight="600">${esc(title)}</text>`;
  for (let tick = 0; tick <= 4; tick++) {
    const ratio = tick / 4;
    const gy = py + ph - ph * ratio;
    const tickValue = scale === "log"
      ? 10 ** (Math.log10(lo) + ratio * (Math.log10(hi) - Math.log10(lo)))
      : lo + ratio * (hi - lo);
    svg += `<line x1="${px}" y1="${gy}" x2="${px + pw}" y2="${gy}" stroke="#e5e7eb"/>`;
    svg += `<text x="${px - 9}" y="${gy + 4}" text-anchor="end" font-size="11" fill="#4b5563">${tickValue >= 100 ? tickValue.toFixed(0) : tickValue >= 10 ? tickValue.toFixed(1) : tickValue.toFixed(2)}</text>`;
  }
  batches.forEach((batch, index) => {
    const gx = xAt(index);
    svg += `<line x1="${gx}" y1="${py + ph}" x2="${gx}" y2="${py + ph + 5}" stroke="#374151"/>`;
    svg += `<text x="${gx}" y="${py + ph + 21}" text-anchor="middle" font-size="11" fill="#374151">${batch}</text>`;
  });
  svg += `<line x1="${px}" y1="${py}" x2="${px}" y2="${py + ph}" stroke="#374151"/>`;
  svg += `<line x1="${px}" y1="${py + ph}" x2="${px + pw}" y2="${py + ph}" stroke="#374151"/>`;
  svg += `<text x="${px + pw / 2}" y="${y + height - 10}" text-anchor="middle" font-size="12">Batch records (1024 B each)</text>`;
  svg += `<text x="${x + 15}" y="${py + ph / 2}" transform="rotate(-90 ${x + 15} ${py + ph / 2})" text-anchor="middle" font-size="12">${esc(yLabel)}</text>`;

  for (const language of languages) {
    const med = batches.map((batch, index) => [xAt(index), yAt(value(language, batch).median)]);
    const q25 = batches.map((batch, index) => [xAt(index), yAt(value(language, batch).q25)]);
    const q75 = batches.map((batch, index) => [xAt(index), yAt(value(language, batch).q75)]);
    svg += `<path d="${areaPath(q75, q25)}" fill="${colors[language]}" opacity="0.12"/>`;
    svg += `<path d="${linePath(med)}" fill="none" stroke="${colors[language]}" stroke-width="2.5"/>`;
    med.forEach(([cx, cy]) => { svg += `<circle cx="${cx}" cy="${cy}" r="3.4" fill="${colors[language]}"/>`; });
  }
  return svg + `</g>`;
}

function legend(x, y) {
  let svg = `<g>`;
  languages.forEach((language, index) => {
    const lx = x + index * 115;
    svg += `<line x1="${lx}" y1="${y}" x2="${lx + 28}" y2="${y}" stroke="${colors[language]}" stroke-width="4"/>`;
    svg += `<text x="${lx + 36}" y="${y + 5}" font-size="14">${language}</text>`;
  });
  return svg + `<text x="${x + 350}" y="${y + 5}" font-size="12" fill="#6b7280">line: median; band: IQR</text></g>`;
}

function statValue(field, adjusted, throughput = false) {
  return (language, batch) => {
    const source = data[language].stats[batch][field];
    const baseline = adjusted ? data[language].baseline[field] : 0;
    const convert = (number) => {
      const ns = number - baseline;
      if (throughput) return (batch / 1024) / (ns / 1e9);
      return ns / 1e6;
    };
    // For throughput, lower latency maps to higher throughput, so swap quartiles.
    return throughput
      ? { q25: convert(source.q75), median: convert(source.median), q75: convert(source.q25) }
      : { q25: convert(source.q25), median: convert(source.median), q75: convert(source.q75) };
  };
}

const overview = [
  `<svg xmlns="http://www.w3.org/2000/svg" width="1500" height="990" viewBox="0 0 1500 990">`,
  `<rect width="1500" height="990" fill="#f8fafc"/>`,
  `<text x="750" y="34" text-anchor="middle" font-size="24" font-weight="700">Rust vs C vs C# — Fresh-pool Parse Comparison</text>`,
  `<text x="750" y="57" text-anchor="middle" font-size="13" fill="#475569">Three runs, 300 samples per batch and language</text>`,
  legend(500, 82),
  panel({ x: 25, y: 105, width: 710, height: 410, title: "Raw end-to-end latency", yLabel: "Latency (ms, log scale)", value: statValue("total_ns", false), scale: "log" }),
  panel({ x: 765, y: 105, width: 710, height: 410, title: "Raw end-to-end throughput", yLabel: "Throughput (MiB/s)", value: statValue("total_ns", false, true), maxValue: 330 }),
  panel({ x: 25, y: 545, width: 710, height: 410, title: "Empty-call-adjusted latency", yLabel: "Incremental latency (ms, log scale)", value: statValue("total_ns", true), scale: "log" }),
  panel({ x: 765, y: 545, width: 710, height: 410, title: "Empty-call-adjusted throughput", yLabel: "Incremental throughput (MiB/s)", value: statValue("total_ns", true, true), maxValue: 330 }),
  `</svg>`,
].join("");
fs.writeFileSync(path.join(resultsDir, "rust_c_csharp_overview_20260815.svg"), overview);

const stages = [
  `<svg xmlns="http://www.w3.org/2000/svg" width="1800" height="610" viewBox="0 0 1800 610">`,
  `<rect width="1800" height="610" fill="#f8fafc"/>`,
  `<text x="900" y="34" text-anchor="middle" font-size="24" font-weight="700">Empty-call-adjusted Stage Latency</text>`,
  `<text x="900" y="57" text-anchor="middle" font-size="13" fill="#475569">H0-H1 copy-in, H1-H2 guest parse/build, H2-H3 copy-out</text>`,
  legend(650, 82),
  panel({ x: 20, y: 105, width: 570, height: 475, title: "Copy-in (H0-H1)", yLabel: "Latency (ms, log scale)", value: statValue("copy_in_ns", true), scale: "log" }),
  panel({ x: 615, y: 105, width: 570, height: 475, title: "Guest parse/build (H1-H2)", yLabel: "Latency (ms, log scale)", value: statValue("guest_ns", true), scale: "log" }),
  panel({ x: 1210, y: 105, width: 570, height: 475, title: "Copy-out (H2-H3)", yLabel: "Latency (ms, log scale)", value: statValue("copy_out_ns", true), scale: "log" }),
  `</svg>`,
].join("");
fs.writeFileSync(path.join(resultsDir, "rust_c_csharp_stages_20260815.svg"), stages);

const summaryHeader = ["language", "batch_count", ...fields.flatMap((field) => [`${field}_raw_median`, `${field}_adjusted_median`]), "raw_throughput_mib_s", "adjusted_throughput_mib_s"];
const summaryRows = [summaryHeader.join(",")];
for (const language of languages) {
  for (const batch of batches) {
    const values = [language, batch];
    for (const field of fields) {
      const median = data[language].stats[batch][field].median;
      values.push(median, median - data[language].baseline[field]);
    }
    const rawTotal = data[language].stats[batch].total_ns.median;
    const adjustedTotal = rawTotal - data[language].baseline.total_ns;
    values.push(((batch / 1024) / (rawTotal / 1e9)).toFixed(6));
    values.push(((batch / 1024) / (adjustedTotal / 1e9)).toFixed(6));
    summaryRows.push(values.join(","));
  }
}
fs.writeFileSync(path.join(resultsDir, "rust_c_csharp_summary_20260815.csv"), `${summaryRows.join("\n")}\n`);

console.log("generated rust_c_csharp_overview_20260815.svg");
console.log("generated rust_c_csharp_stages_20260815.svg");
console.log("generated rust_c_csharp_summary_20260815.csv");
