#!/usr/bin/env node

const fs = require("fs");
const path = require("path");

const resultsDir = path.resolve(process.argv[2] || "results/all_stages_20260815_230405");
const stages = ["parse", "filter", "format"];
const languages = ["rust", "c", "csharp"];
const labels = { rust: "Rust", c: "C", csharp: "C#" };
const colors = { rust: "#2563eb", c: "#ea580c", csharp: "#059669" };
const batches = [64, 128, 256, 512, 1024, 2048, 4096, 8192];

function readCsv(filename) {
  const lines = fs.readFileSync(path.join(resultsDir, filename), "utf8").trim().split(/\r?\n/);
  const header = lines.shift().split(",");
  return lines.map((line) => Object.fromEntries(
    line.split(",").map((value, index) => [header[index], value]),
  ));
}

function quantile(values, fraction) {
  const sorted = [...values].sort((a, b) => a - b);
  return sorted[Math.ceil(fraction * sorted.length) - 1];
}

const data = {};
for (const stage of stages) {
  data[stage] = {};
  for (const language of languages) {
    const rows = [1, 2, 3].flatMap((run) => readCsv(`${stage}_${language}_run${run}.csv`));
    data[stage][language] = {};
    for (const batch of batches) {
      const group = rows.filter((row) => Number(row.batch_count) === batch);
      const values = {
        copyIn: group.map((row) => Number(row.copy_in_ns)),
        guest: group.map((row) => Number(row.guest_ns)),
        copyOut: group.map((row) => Number(row.copy_out_ns)),
        boundary: group.map((row) => Number(row.copy_in_ns) + Number(row.copy_out_ns)),
        total: group.map((row) => Number(row.total_ns)),
      };
      data[stage][language][batch] = Object.fromEntries(
        Object.entries(values).map(([name, samples]) => [name, {
          q25: quantile(samples, 0.25),
          median: quantile(samples, 0.50),
          q75: quantile(samples, 0.75),
          p95: quantile(samples, 0.95),
        }]),
      );
    }
  }
}

const summary = [[
  "stage", "language", "batch_count", "samples",
  "copy_in_median_ns", "guest_median_ns", "copy_out_median_ns",
  "boundary_median_ns", "total_median_ns", "total_p95_ns", "boundary_share_percent",
].join(",")];
for (const stage of stages) {
  for (const language of languages) {
    for (const batch of batches) {
      const s = data[stage][language][batch];
      summary.push([
        stage, labels[language], batch, 300,
        s.copyIn.median, s.guest.median, s.copyOut.median,
        s.boundary.median, s.total.median, s.total.p95,
        (100 * s.boundary.median / s.total.median).toFixed(3),
      ].join(","));
    }
  }
}
fs.writeFileSync(path.join(resultsDir, "fresh_all_stages_summary.csv"), `${summary.join("\n")}\n`);

function linePath(points) {
  return points.map(([x, y], index) => `${index ? "L" : "M"}${x.toFixed(1)},${y.toFixed(1)}`).join(" ");
}

function areaPath(top, bottom) {
  return `${linePath(top)} ${[...bottom].reverse().map(([x, y]) => `L${x.toFixed(1)},${y.toFixed(1)}`).join(" ")} Z`;
}

function panel(stage, metric, x, title) {
  const y = 105;
  const width = 570;
  const height = 485;
  const margin = { left: 76, right: 18, top: 45, bottom: 62 };
  const px = x + margin.left;
  const py = y + margin.top;
  const pw = width - margin.left - margin.right;
  const ph = height - margin.top - margin.bottom;
  const all = languages.flatMap((language) => batches.flatMap((batch) => {
    const s = data[stage][language][batch][metric];
    return [s.q25, s.median, s.q75];
  })).filter((value) => value > 0).map((value) => value / 1e6);
  const lo = Math.min(...all) * 0.75;
  const hi = Math.max(...all) * 1.25;
  const xAt = (index) => px + pw * index / (batches.length - 1);
  const yAt = (valueNs) => {
    const value = valueNs / 1e6;
    return py + ph - ph * (Math.log10(value) - Math.log10(lo)) / (Math.log10(hi) - Math.log10(lo));
  };
  let svg = `<g><rect x="${x}" y="${y}" width="${width}" height="${height}" rx="10" fill="#fff" stroke="#cbd5e1"/>`;
  svg += `<text x="${x + width / 2}" y="${y + 27}" text-anchor="middle" font-size="19" font-weight="700">${title}</text>`;
  for (let tick = 0; tick <= 4; tick++) {
    const ratio = tick / 4;
    const gy = py + ph - ph * ratio;
    const value = 10 ** (Math.log10(lo) + ratio * (Math.log10(hi) - Math.log10(lo)));
    svg += `<line x1="${px}" y1="${gy}" x2="${px + pw}" y2="${gy}" stroke="#e2e8f0"/>`;
    svg += `<text x="${px - 9}" y="${gy + 4}" text-anchor="end" font-size="11" fill="#475569">${value < 0.1 ? value.toFixed(3) : value < 10 ? value.toFixed(2) : value.toFixed(0)}</text>`;
  }
  batches.forEach((batch, index) => {
    const gx = xAt(index);
    svg += `<text x="${gx}" y="${py + ph + 22}" text-anchor="middle" font-size="11" fill="#334155">${batch}</text>`;
  });
  svg += `<line x1="${px}" y1="${py}" x2="${px}" y2="${py + ph}" stroke="#334155"/>`;
  svg += `<line x1="${px}" y1="${py + ph}" x2="${px + pw}" y2="${py + ph}" stroke="#334155"/>`;
  svg += `<text x="${px + pw / 2}" y="${y + height - 12}" text-anchor="middle" font-size="12">Batch records (1,024 B/log)</text>`;
  svg += `<text x="${x + 17}" y="${py + ph / 2}" transform="rotate(-90 ${x + 17} ${py + ph / 2})" text-anchor="middle" font-size="12">Latency (ms, log scale)</text>`;
  for (const language of languages) {
    const median = batches.map((batch, index) => [xAt(index), yAt(data[stage][language][batch][metric].median)]);
    const q25 = batches.map((batch, index) => [xAt(index), yAt(data[stage][language][batch][metric].q25)]);
    const q75 = batches.map((batch, index) => [xAt(index), yAt(data[stage][language][batch][metric].q75)]);
    svg += `<path d="${areaPath(q75, q25)}" fill="${colors[language]}" opacity="0.13"/>`;
    svg += `<path d="${linePath(median)}" fill="none" stroke="${colors[language]}" stroke-width="2.6"/>`;
    median.forEach(([cx, cy]) => { svg += `<circle cx="${cx}" cy="${cy}" r="3.2" fill="${colors[language]}"/>`; });
  }
  return `${svg}</g>`;
}

function chart(metric, title, subtitle, filename) {
  let svg = `<svg xmlns="http://www.w3.org/2000/svg" width="1800" height="620" viewBox="0 0 1800 620">`;
  svg += `<rect width="1800" height="620" fill="#f8fafc"/>`;
  svg += `<text x="900" y="32" text-anchor="middle" font-size="24" font-weight="700">${title}</text>`;
  svg += `<text x="900" y="55" text-anchor="middle" font-size="13" fill="#475569">${subtitle}</text>`;
  languages.forEach((language, index) => {
    const lx = 685 + index * 145;
    svg += `<line x1="${lx}" y1="82" x2="${lx + 30}" y2="82" stroke="${colors[language]}" stroke-width="4"/>`;
    svg += `<text x="${lx + 38}" y="87" font-size="14">${labels[language]}</text>`;
  });
  svg += panel("parse", metric, 15, "Parse");
  svg += panel("filter", metric, 615, "Filter");
  svg += panel("format", metric, 1215, "Format");
  svg += `</svg>`;
  fs.writeFileSync(path.join(resultsDir, filename), svg);
}

chart(
  "total",
  "Fresh WCM Component Call Latency",
  "Median line and interquartile band; 300 measured calls per point",
  "fresh_total_latency.svg",
);
chart(
  "boundary",
  "Fresh WCM Canonical-ABI Boundary Overhead",
  "H0-H1 copy-in plus H2-H3 copy-out; guest work excluded",
  "fresh_boundary_overhead.svg",
);

console.log(path.join(resultsDir, "fresh_all_stages_summary.csv"));
console.log(path.join(resultsDir, "fresh_total_latency.svg"));
console.log(path.join(resultsDir, "fresh_boundary_overhead.svg"));
