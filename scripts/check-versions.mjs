#!/usr/bin/env node
// SPDX-License-Identifier: GPL-3.0-or-later
//
// One version for every public artifact (design §5.6): the Cargo workspace,
// the npm packages and the player.
//   node scripts/check-versions.mjs [--expect <version>]
import { readFileSync } from 'node:fs';

const read = (p) => readFileSync(new URL(`../${p}`, import.meta.url), 'utf8');
const found = {
  'Cargo.toml': /\[workspace\.package\][^[]*?\nversion = "([^"]+)"/s.exec(read('Cargo.toml'))?.[1],
  'apps/player/package.json': JSON.parse(read('apps/player/package.json')).version,
  'apps/cli/package.json': JSON.parse(read('apps/cli/package.json')).version,
  'packages/protocol/package.json': JSON.parse(read('packages/protocol/package.json')).version,
};
const expectIdx = process.argv.indexOf('--expect');
const expected = expectIdx > 0 ? process.argv[expectIdx + 1]?.replace(/^v/, '') : undefined;
const versions = new Set(Object.values(found));
let ok = versions.size === 1 && !versions.has(undefined);
if (expected && !versions.has(expected)) ok = false;
for (const [where, v] of Object.entries(found)) console.log(`${v ?? 'MISSING'}\t${where}`);
if (!ok) {
  console.error(expected ? `versions must all be ${expected}` : 'versions differ');
  process.exit(1);
}
