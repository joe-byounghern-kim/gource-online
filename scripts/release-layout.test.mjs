// SPDX-License-Identifier: GPL-3.0-or-later
// Exercise the release workflow's real pnpm pack destination without publishing.
import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';

test('protocol pack lands at the root consumed by upload and publish', () => {
  const workflow = readFileSync(new URL('../.github/workflows/release.yml', import.meta.url), 'utf8');
  const destination = /pnpm -C packages\/protocol pack --pack-destination ([^\n]+)/.exec(workflow)?.[1]?.trim();
  assert.ok(destination, 'release protocol packing command exists');
  const root = mkdtempSync(join(process.env.JCODE_SCRATCH_DIR ?? tmpdir(), 'gource-release-layout-'));
  const dir = join(root, 'packages', 'protocol');
  mkdirSync(dir, { recursive: true });
  const version = JSON.parse(readFileSync(new URL('../packages/protocol/package.json', import.meta.url), 'utf8')).version;
  writeFileSync(join(dir, 'package.json'), JSON.stringify({ name: '@gource-online/protocol', version, files: ['README.md'], license: 'MIT' }));
  writeFileSync(join(dir, 'README.md'), 'Release packaging layout fixture.');
  try {
    const dest = destination === '"$GITHUB_WORKSPACE"' ? root : destination;
    execFileSync('pnpm', ['-C', dir, 'pack', '--pack-destination', dest], { cwd: root, stdio: 'pipe' });
    assert.ok(existsSync(join(root, `gource-online-protocol-${version}.tgz`)), 'protocol tarball must be at the upload root, not inside packages/protocol');
    assert.match(workflow, /out\/gource-online-protocol-\*\.tgz/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
