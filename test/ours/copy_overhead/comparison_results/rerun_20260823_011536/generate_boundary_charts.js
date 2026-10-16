#!/usr/bin/env node

const fs = require("fs");
const path = require("path");

const root = process.argv[2] ? path.resolve(process.argv[2]) : __dirname;
const sourceLabel = path.basename(root);
const outputDir = path.join(root, "charts");
const stages = ["parse", "filter", "format"];
const boundaries = [
  { key: "copy_in_ns", slug: "copy_in", label: "Copy In" },
  { key: "copy_out_ns", slug: "copy_out", label: "Copy Out" },
];
const expectedBatches = [64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768];
const products = [
  {
    key: "wcm",
    label: "WCM Canonical ABI",
    color: "#2563eb",
    marker: "circle",
    directory: "wcm",
    filePattern: (stage) => new RegExp(`^${stage}_rust_isolated_run\\d+\\.csv$`),
  },
  {
    key: "json",
    label: "Core Wasm JSON ABI",
    color: "#d97706",
    marker: "square",
    directory: "json",
    filePattern: (stage) => new RegExp(`^${stage}_rust_core_wasm_json_run\\d+\\.csv$`),
  },
  {
    key: "binary",
    label: "Core Wasm Direct Memory ABI",
    color: "#059669",
    marker: "diamond",
    directory: "binary",
    filePattern: (stage) =>
      new RegExp(`^${stage}_rust_core_wasm_direct_memory_run\\d+\\.csv$`),
  },
];

function parseCsv(filePath) {
  const lines = fs.readFileSync(filePath, "utf8").trim().split(/\r?\n/);
  const headers = lines[0].split(",");
  return lines.slice(1).filter(Boolean).map((line, rowIndex) => {
    const values = line.split(",");
    if (values.length !== headers.length) {
      throw new Error(`${filePath}:${rowIndex + 2}: unexpected CSV column count`);
    }
    return Object.fromEntries(headers.map((header, index) => [header, values[index]]));
  });
}

function median(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 === 0
    ? (sorted[middle - 1] + sorted[middle]) / 2
    : sorted[middle];
}

function loadSeries(product, stage, boundary) {
  const directory = path.join(root, product.directory);
  const files = fs.readdirSync(directory)
    .filter((name) => product.filePattern(stage).test(name))
    .sort();
  if (files.length !== 3) {
    throw new Error(`${product.label} ${stage}: expected 3 CSV files, found ${files.length}`);
  }

  const grouped = new Map(expectedBatches.map((batch) => [batch, []]));
  for (const file of files) {
    for (const row of parseCsv(path.join(directory, file))) {
      const batch = Number(row.batch_count);
      if (!grouped.has(batch)) continue;
      const value = Number(row[boundary.key]);
      if (!Number.isFinite(value) || value <= 0) {
        throw new Error(`${file}: invalid ${boundary.key} value '${row[boundary.key]}'`);
      }
      grouped.get(batch).push(value);
    }
  }

  return expectedBatches.map((batch) => {
    const samples = grouped.get(batch);
    if (samples.length !== 300) {
      throw new Error(`${product.label} ${stage} batch ${batch}: expected 300 samples, found ${samples.length}`);
    }
    return { batch, samples: samples.length, medianNs: median(samples) };
  });
}

function escapeXml(value) {
  return String(value)
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}

function formatBatch(value) {
  return value >= 1024 ? `${value / 1024}k` : String(value);
}

function formatTime(value) {
  if (value >= 1e9) return `${(value / 1e9).toFixed(value >= 1e10 ? 0 : 1)} s`;
  if (value >= 1e6) return `${(value / 1e6).toFixed(value >= 1e7 ? 0 : 1)} ms`;
  if (value >= 1e3) return `${(value / 1e3).toFixed(value >= 1e4 ? 0 : 1)} µs`;
  return `${Math.round(value)} ns`;
}

function yTicks(min, max) {
  const ticks = [];
  const startPower = Math.floor(Math.log10(min)) - 1;
  const endPower = Math.ceil(Math.log10(max)) + 1;
  for (let power = startPower; power <= endPower; power += 1) {
    for (const multiplier of [1, 2, 5]) {
      const value = multiplier * 10 ** power;
      if (value >= min && value <= max) ticks.push(value);
    }
  }
  return ticks;
}

function markerSvg(type, x, y, color, title) {
  const safeTitle = escapeXml(title);
  if (type === "square") {
    return `<rect x="${(x - 4.5).toFixed(2)}" y="${(y - 4.5).toFixed(2)}" width="9" height="9" rx="1" fill="${color}" stroke="#fff" stroke-width="1.5"><title>${safeTitle}</title></rect>`;
  }
  if (type === "diamond") {
    return `<path d="M ${x.toFixed(2)} ${(y - 5.5).toFixed(2)} L ${(x + 5.5).toFixed(2)} ${y.toFixed(2)} L ${x.toFixed(2)} ${(y + 5.5).toFixed(2)} L ${(x - 5.5).toFixed(2)} ${y.toFixed(2)} Z" fill="${color}" stroke="#fff" stroke-width="1.5"><title>${safeTitle}</title></path>`;
  }
  return `<circle cx="${x.toFixed(2)}" cy="${y.toFixed(2)}" r="4.8" fill="${color}" stroke="#fff" stroke-width="1.5"><title>${safeTitle}</title></circle>`;
}

function renderChart(stage, boundary, allSeries) {
  const width = 1200;
  const height = 760;
  const margin = { top: 132, right: 72, bottom: 104, left: 122 };
  const plotWidth = width - margin.left - margin.right;
  const plotHeight = height - margin.top - margin.bottom;
  const values = allSeries.flatMap((series) => series.points.map((point) => point.medianNs));
  const rawLogMin = Math.log10(Math.min(...values));
  const rawLogMax = Math.log10(Math.max(...values));
  const padding = Math.max((rawLogMax - rawLogMin) * 0.1, 0.08);
  const logMin = rawLogMin - padding;
  const logMax = rawLogMax + padding;
  const yMin = 10 ** logMin;
  const yMax = 10 ** logMax;
  const xMin = Math.log2(expectedBatches[0]);
  const xMax = Math.log2(expectedBatches.at(-1));

  const xScale = (batch) =>
    margin.left + ((Math.log2(batch) - xMin) / (xMax - xMin)) * plotWidth;
  const yScale = (value) =>
    margin.top + ((logMax - Math.log10(value)) / (logMax - logMin)) * plotHeight;

  const stageLabel = stage[0].toUpperCase() + stage.slice(1);
  const elements = [];
  elements.push(`<?xml version="1.0" encoding="UTF-8"?>`);
  elements.push(`<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}" role="img" aria-labelledby="title desc">`);
  elements.push(`<title id="title">${stageLabel} ${boundary.label} ABI Comparison</title>`);
  elements.push(`<desc id="desc">Log-log chart of median ${boundary.label} time by batch size for WCM Canonical ABI, Core Wasm JSON ABI, and Core Wasm Direct Memory ABI.</desc>`);
  elements.push(`<rect width="100%" height="100%" fill="#f8fafc"/>`);
  elements.push(`<rect x="${margin.left}" y="${margin.top}" width="${plotWidth}" height="${plotHeight}" rx="8" fill="#ffffff" stroke="#cbd5e1"/>`);
  elements.push(`<text x="${margin.left}" y="52" font-family="Inter,Segoe UI,sans-serif" font-size="29" font-weight="700" fill="#0f172a">${stageLabel} ${boundary.label} — ABI Comparison</text>`);
  elements.push(`<text x="${margin.left}" y="82" font-family="Inter,Segoe UI,sans-serif" font-size="15" fill="#475569">Median of 300 samples per point (3 runs × 100) · log₂ batch axis · log₁₀ time axis</text>`);

  for (const tick of yTicks(yMin, yMax)) {
    const y = yScale(tick);
    elements.push(`<line x1="${margin.left}" y1="${y.toFixed(2)}" x2="${width - margin.right}" y2="${y.toFixed(2)}" stroke="#e2e8f0" stroke-width="1"/>`);
    elements.push(`<text x="${margin.left - 14}" y="${(y + 5).toFixed(2)}" text-anchor="end" font-family="Inter,Segoe UI,sans-serif" font-size="13" fill="#475569">${escapeXml(formatTime(tick))}</text>`);
  }

  for (const batch of expectedBatches) {
    const x = xScale(batch);
    elements.push(`<line x1="${x.toFixed(2)}" y1="${margin.top}" x2="${x.toFixed(2)}" y2="${height - margin.bottom}" stroke="#f1f5f9" stroke-width="1"/>`);
    elements.push(`<text x="${x.toFixed(2)}" y="${height - margin.bottom + 27}" text-anchor="middle" font-family="Inter,Segoe UI,sans-serif" font-size="13" fill="#475569">${formatBatch(batch)}</text>`);
  }

  elements.push(`<text x="${margin.left + plotWidth / 2}" y="${height - 34}" text-anchor="middle" font-family="Inter,Segoe UI,sans-serif" font-size="16" font-weight="600" fill="#334155">Batch size (log₂ scale)</text>`);
  elements.push(`<text x="30" y="${margin.top + plotHeight / 2}" transform="rotate(-90 30 ${margin.top + plotHeight / 2})" text-anchor="middle" font-family="Inter,Segoe UI,sans-serif" font-size="16" font-weight="600" fill="#334155">Time (log₁₀ scale)</text>`);

  allSeries.forEach((series, seriesIndex) => {
    const points = series.points.map((point) => [xScale(point.batch), yScale(point.medianNs), point]);
    const pathData = points.map(([x, y], index) => `${index === 0 ? "M" : "L"} ${x.toFixed(2)} ${y.toFixed(2)}`).join(" ");
    elements.push(`<path d="${pathData}" fill="none" stroke="${series.product.color}" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"/>`);
    for (const [x, y, point] of points) {
      elements.push(markerSvg(
        series.product.marker,
        x,
        y,
        series.product.color,
        `${series.product.label}: batch ${point.batch}, median ${formatTime(point.medianNs)} (${point.medianNs.toFixed(1)} ns)`,
      ));
    }

    const legendX = margin.left + seriesIndex * 310;
    const legendY = 108;
    elements.push(`<line x1="${legendX}" y1="${legendY}" x2="${legendX + 32}" y2="${legendY}" stroke="${series.product.color}" stroke-width="3"/>`);
    elements.push(markerSvg(series.product.marker, legendX + 16, legendY, series.product.color, series.product.label));
    elements.push(`<text x="${legendX + 43}" y="${legendY + 5}" font-family="Inter,Segoe UI,sans-serif" font-size="14" font-weight="600" fill="#334155">${escapeXml(series.product.label)}</text>`);
  });

  elements.push(`<text x="${width - margin.right}" y="${height - 12}" text-anchor="end" font-family="Inter,Segoe UI,sans-serif" font-size="11" fill="#64748b">Source: ${escapeXml(sourceLabel)}</text>`);
  elements.push(`</svg>`);
  return elements.join("\n");
}

fs.mkdirSync(outputDir, { recursive: true });
const summaryRows = ["stage,boundary,batch_count,product,samples,median_ns"];

for (const stage of stages) {
  for (const boundary of boundaries) {
    const allSeries = products.map((product) => ({
      product,
      points: loadSeries(product, stage, boundary),
    }));
    for (const series of allSeries) {
      for (const point of series.points) {
        summaryRows.push([
          stage,
          boundary.slug,
          point.batch,
          series.product.key,
          point.samples,
          point.medianNs,
        ].join(","));
      }
    }
    const filename = `${stage}_${boundary.slug}_loglog.svg`;
    fs.writeFileSync(path.join(outputDir, filename), renderChart(stage, boundary, allSeries));
    process.stdout.write(`generated charts/${filename}\n`);
  }
}

fs.writeFileSync(path.join(outputDir, "boundary_medians.csv"), `${summaryRows.join("\n")}\n`);
process.stdout.write("generated charts/boundary_medians.csv\n");
