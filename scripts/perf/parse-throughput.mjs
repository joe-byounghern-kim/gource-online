#!/usr/bin/env node
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Throughput of the wasm LogParser (the parser worker's code) and of
// GourceApp-side batch decoding, outside the browser: feeds a custom log in
// fixed-size chunks to LogParser.push and reports MB/s.
//
//   node scripts/perf/parse-throughput.mjs <custom.log> [chunk KiB=64]
//
// Needs scripts/build-web.sh (packages/viewer/src/wasm).
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const wasmDir = resolve(here, '../../packages/viewer/src/wasm');
const mod = await import(resolve(wasmDir, 'gource_web.js'));
mod.initSync({ module: readFileSync(resolve(wasmDir, 'gource_web_bg.wasm')) });

const bytes = readFileSync(process.argv[2]);
const chunk = Number(process.argv[3] ?? 64) * 1024;
for (let run = 0; run < 3; run++) {
  const p = new mod.LogParser();
  let out = 0;
  let batches = 0;
  const t0 = performance.now();
  for (let off = 0; off < bytes.length; off += chunk) {
    const b = p.push(bytes.subarray(off, Math.min(bytes.length, off + chunk)));
    out += b.byteLength;
    if (b.byteLength) batches++;
  }
  out += p.finish().byteLength;
  const ms = performance.now() - t0;
  console.log(
    `run ${run}: ${(bytes.length / 1048576).toFixed(1)} MiB in ${ms.toFixed(1)} ms = ` +
      `${(bytes.length / 1048576 / (ms / 1000)).toFixed(1)} MiB/s; ${p.commits()} commits, ` +
      `${batches} batches, ${(out / 1048576).toFixed(1)} MiB encoded`,
  );
  p.free();
}
