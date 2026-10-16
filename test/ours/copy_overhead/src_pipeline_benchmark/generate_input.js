#!/usr/bin/env node

const { once } = require("events");

const batch = Number(process.argv[2]);
const calls = Number(process.argv[3]);
if (!Number.isInteger(batch) || batch <= 0 || !Number.isInteger(calls) || calls <= 0) {
  console.error("usage: generate_input.js BATCH_RECORDS CALLS");
  process.exit(2);
}

function fixture(level) {
  const prefix = `{"ts":"2026-08-14T00:00:00Z","level":"${level}","msg":"`;
  const suffix = `","att":{"source":"parse-benchmark","sequence":"fixed"}}`;
  const json = `${prefix}${"x".repeat(1024 - prefix.length - suffix.length)}${suffix}`;
  if (Buffer.byteLength(json) !== 1024) throw new Error("fixture is not 1024 bytes");
  return `${json}\n`;
}

async function main() {
  const info = fixture("info");
  const warn = fixture("warn");
  const records = batch * calls;
  for (let index = 0; index < records; index++) {
    if (!process.stdout.write(index % 2 === 0 ? info : warn)) {
      await once(process.stdout, "drain");
    }
  }
}

main().catch((error) => {
  console.error(error);
  process.exitCode = 1;
});
