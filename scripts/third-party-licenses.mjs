#!/usr/bin/env node
// SPDX-License-Identifier: GPL-3.0-or-later
//
// The licence notices of the Rust crates compiled into a shipped artifact
// (the wasm engine in the page and the CLI's copy of it, or a gource-rs
// binary). Their licences (MIT, Apache-2.0, …) ask for the notices to travel
// with the binary; LICENSING.md points here.
//
//   node scripts/third-party-licenses.mjs --package gource-web --target wasm32-unknown-unknown > NOTICES.txt
//   node scripts/third-party-licenses.mjs --package gource-cli [--target <triple>] > NOTICES.txt

import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parseArgs } from 'node:util';

/** Non-workspace packages reachable from `roots` through normal (not build or dev) dependencies, sorted by name. */
export function closure(metadata, roots) {
  const byId = new Map(metadata.packages.map((p) => [p.id, p]));
  const nodes = new Map(metadata.resolve.nodes.map((n) => [n.id, n]));
  const members = new Set(metadata.workspace_members);
  const start = roots.map((name) => {
    const id = metadata.workspace_members.find((m) => byId.get(m)?.name === name);
    if (!id) throw new Error(`no workspace package named ${name}`);
    return id;
  });
  const seen = new Set(start);
  const queue = [...start];
  while (queue.length) {
    const node = nodes.get(queue.pop());
    for (const d of node?.deps ?? []) {
      if (!d.dep_kinds.some((k) => k.kind === null) || seen.has(d.pkg)) continue;
      seen.add(d.pkg);
      queue.push(d.pkg);
    }
  }
  return [...seen]
    .filter((id) => !members.has(id))
    .map((id) => byId.get(id))
    .sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version));
}

/** LICENSE*, LICENCE*, COPYING*, NOTICE* files directly in `dir`. */
export function licenceFiles(dir) {
  return readdirSync(dir)
    .filter((name) => /^(licen[cs]e|copying|notice)/i.test(name) && statSync(join(dir, name)).isFile())
    .sort()
    .map((name) => ({ name, text: readFileSync(join(dir, name), 'utf8') }));
}

export function render(artifact, packages, { musl = false } = {}) {
  const out = [
    `Third-party software in ${artifact}`,
    '',
    'These Rust crates are compiled into it. Each is used under the licence named here;',
    'the licence texts follow each entry.',
    '',
  ];
  for (const p of packages) {
    out.push('='.repeat(78), `${p.name} ${p.version} — ${p.license ?? 'licence not stated'}`);
    if (p.repository) out.push(p.repository);
    if (p.files.length === 0) out.push('(no licence file in the package; see its licence identifier)');
    for (const f of p.files) out.push('', `--- ${f.name} ---`, f.text.trimEnd());
    out.push('');
  }
  out.push(
    '='.repeat(78),
    'Rust standard library — MIT OR Apache-2.0',
    'https://github.com/rust-lang/rust (COPYRIGHT, LICENSE-MIT, LICENSE-APACHE)',
    '',
  );
  if (musl) {
    out.push('='.repeat(78), 'musl libc — MIT', 'https://musl.libc.org (COPYRIGHT)', '');
  }
  return `${out.join('\n')}\n`;
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const { values } = parseArgs({ options: { package: { type: 'string' }, target: { type: 'string' } } });
  if (!values.package) {
    console.error('usage: third-party-licenses.mjs --package <workspace crate> [--target <triple>]');
    process.exit(2);
  }
  const args = ['metadata', '--format-version', '1', '--locked'];
  if (values.target) args.push('--filter-platform', values.target);
  const metadata = JSON.parse(execFileSync('cargo', args, { maxBuffer: 256 << 20 }).toString());
  const packages = closure(metadata, [values.package]).map((p) => ({ ...p, files: licenceFiles(dirname(p.manifest_path)) }));
  const label = values.target ? `${values.package} (${values.target})` : values.package;
  process.stdout.write(render(label, packages, { musl: values.target?.endsWith('-musl') ?? false }));
}
