// SPDX-License-Identifier: GPL-3.0-or-later
//
//   node --test scripts/third-party-licenses.test.mjs

import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

import { closure, licenceFiles, render } from './third-party-licenses.mjs';

const pkg = (name, extra = {}) => ({
  id: `registry#${name}@1.0.0`,
  name,
  version: '1.0.0',
  license: 'MIT',
  repository: `https://example.invalid/${name}`,
  manifest_path: `/reg/${name}/Cargo.toml`,
  ...extra,
});
const dep = (name, kinds = [{ kind: null }]) => ({ pkg: `registry#${name}@1.0.0`, dep_kinds: kinds });

const metadata = {
  workspace_members: ['path#app@0.1.0', 'path#core@0.1.0'],
  packages: [
    pkg('app', { id: 'path#app@0.1.0' }),
    pkg('core', { id: 'path#core@0.1.0' }),
    pkg('serde', { license: 'MIT OR Apache-2.0' }),
    pkg('itoa'),
    pkg('cc'),
    pkg('proptest'),
  ],
  resolve: {
    nodes: [
      { id: 'path#app@0.1.0', deps: [{ pkg: 'path#core@0.1.0', dep_kinds: [{ kind: null }] }, dep('cc', [{ kind: 'build' }])] },
      { id: 'path#core@0.1.0', deps: [dep('serde'), dep('proptest', [{ kind: 'dev' }])] },
      { id: 'registry#serde@1.0.0', deps: [dep('itoa')] },
      { id: 'registry#itoa@1.0.0', deps: [] },
      { id: 'registry#cc@1.0.0', deps: [] },
      { id: 'registry#proptest@1.0.0', deps: [] },
    ],
  },
};

test('the closure follows normal dependencies only and leaves out the workspace', () => {
  assert.deepEqual(
    closure(metadata, ['app']).map((p) => p.name),
    ['itoa', 'serde'],
  );
});

test('an unknown root is an error, not an empty list', () => {
  assert.throws(() => closure(metadata, ['nope']), /no workspace package named nope/);
});

test('licence files are found case-insensitively and in a stable order', () => {
  const dir = mkdtempSync(join(tmpdir(), 'tpl-'));
  writeFileSync(join(dir, 'LICENSE-MIT'), 'mit text');
  writeFileSync(join(dir, 'license-apache'), 'apache text');
  writeFileSync(join(dir, 'COPYING'), 'copying text');
  writeFileSync(join(dir, 'NOTICE'), 'notice text');
  writeFileSync(join(dir, 'README.md'), 'not a licence');
  mkdirSync(join(dir, 'LICENSES'));
  assert.deepEqual(
    licenceFiles(dir).map((f) => f.name),
    ['COPYING', 'LICENSE-MIT', 'NOTICE', 'license-apache'],
  );
});

test('the notice names every package with its licence and texts', () => {
  const text = render('gource-web (wasm)', [
    { name: 'serde', version: '1.0.0', license: 'MIT OR Apache-2.0', repository: 'https://x', files: [{ name: 'LICENSE-MIT', text: 'mit text' }] },
    { name: 'itoa', version: '1.0.0', license: 'MIT', repository: null, files: [] },
  ]);
  assert.match(text, /^Third-party software in gource-web \(wasm\)/);
  assert.match(text, /serde 1\.0\.0 — MIT OR Apache-2\.0\nhttps:\/\/x\n\n--- LICENSE-MIT ---\nmit text/);
  assert.match(text, /itoa 1\.0\.0 — MIT\n\(no licence file in the package; see its licence identifier\)/);
  assert.match(text, /Rust standard library — MIT OR Apache-2\.0/);
  assert.doesNotMatch(text, /musl/);
});

test('a musl binary also names the C library linked into it', () => {
  assert.match(render('gource-cli (x86_64-unknown-linux-musl)', [], { musl: true }), /musl libc — MIT/);
});
