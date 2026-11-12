// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

import { execFileSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

/** A fresh temporary folder. */
export function tempDir() {
  return mkdtempSync(join(tmpdir(), 'gource-online-test-'));
}

/**
 * Runs git in `cwd` with a fixed identity and dates.
 * @param {string} cwd
 * @param {string[]} args
 * @param {number} [time] commit time, unix seconds
 */
export function git(cwd, args, time = 1700000000) {
  return execFileSync('git', args, {
    cwd,
    encoding: 'utf8',
    env: {
      ...process.env,
      GIT_AUTHOR_NAME: 'Jane Doe',
      GIT_AUTHOR_EMAIL: 'jane@example.com',
      GIT_COMMITTER_NAME: 'Jane Doe',
      GIT_COMMITTER_EMAIL: 'jane@example.com',
      GIT_AUTHOR_DATE: `@${time} +0000`,
      GIT_COMMITTER_DATE: `@${time} +0000`,
      GIT_CONFIG_NOSYSTEM: '1',
      GIT_CONFIG_GLOBAL: '/dev/null',
    },
  });
}

/**
 * A repository named `name` with two commits: one adds `src/main.c` and
 * `README`, the next changes the first and deletes the second.
 * @param {string} [name]
 * @returns {string} its folder
 */
export function makeRepo(name = 'demo') {
  const dir = join(tempDir(), name);
  mkdirSync(join(dir, 'src'), { recursive: true });
  git(dir, ['init', '-q', '-b', 'main']);
  writeFileSync(join(dir, 'src', 'main.c'), 'int main(void) { return 0; }\n');
  writeFileSync(join(dir, 'README'), 'demo\n');
  git(dir, ['add', '-A']);
  git(dir, ['commit', '-q', '-m', 'Add main', '-m', 'Co-authored-by: Claude <noreply@anthropic.com>'], 1700000000);
  writeFileSync(join(dir, 'src', 'main.c'), 'int main(void) { return 1; }\n');
  git(dir, ['rm', '-q', 'README']);
  git(dir, ['commit', '-q', '-am', 'Change main'], 1700000100);
  return dir;
}

/**
 * A stand-in for the built web app: index.html and one asset.
 * @returns {string}
 */
export function fakeDist() {
  const dir = tempDir();
  mkdirSync(join(dir, 'assets'));
  writeFileSync(join(dir, 'index.html'), '<!doctype html><html><head><title>Gource</title></head><body></body></html>\n');
  writeFileSync(join(dir, 'assets', 'index-abc.js'), 'console.log(1);\n');
  writeFileSync(join(dir, 'assets', 'engine.wasm'), Buffer.from([0, 97, 115, 109]));
  return dir;
}
