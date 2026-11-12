// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

import assert from 'node:assert/strict';
import { mkdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';

import { CommitCounter, GitError, LOG_ARGS, PRETTY_AI, findRepository, readHistory } from '../src/git.js';
import { git, makeRepo, tempDir } from './helpers.js';

test('the log format is the protocol\'s (PRETTY_AI in packages/protocol/src/git-log.ts)', () => {
  const src = readFileSync(new URL('../../../packages/protocol/src/git-log.ts', import.meta.url), 'utf8');
  const pretty = /export const PRETTY_AI =\s*'([^']+)';/.exec(src)?.[1];
  assert.equal(PRETTY_AI, pretty);
  assert.ok(LOG_ARGS.includes(PRETTY_AI));
  for (const flag of ['--reverse', '--raw', '--encoding=UTF-8', '--no-renames', '--no-show-signature']) {
    assert.ok(LOG_ARGS.includes(flag), flag);
  }
  assert.deepEqual(LOG_ARGS.slice(0, 3), ['-c', 'core.quotepath=off', 'log']);
});

test('the repository is found from any folder inside it', () => {
  const repo = makeRepo('my-project');
  const sub = join(repo, 'src');
  for (const cwd of [repo, sub]) {
    const found = findRepository(cwd);
    assert.equal(found.name, 'my-project');
    assert.equal(found.root, git(repo, ['rev-parse', '--show-toplevel']).trim());
  }
});

test('a folder outside any repository is refused', () => {
  const dir = tempDir();
  assert.throws(() => findRepository(dir), (err) => err instanceof GitError && err.code === 'not_a_repo');
});

test('the history holds headers, the AI line and raw changes', async () => {
  const repo = makeRepo();
  const seen = [];
  const history = await readHistory(repo, (commits, bytes) => seen.push([commits, bytes]));
  assert.equal(history.commits, 2);
  const text = Buffer.concat(history.chunks).toString('utf8');
  assert.equal(history.bytes, Buffer.byteLength(text));
  assert.ok(text.startsWith('user:Jane Doe\n1700000000\nm1:Jane Doe\x1fjane@example.com\x1f'), text);
  assert.ok(text.includes('Co-authored-by: Claude <noreply@anthropic.com>\x1eAdd main'));
  assert.match(text, /:000000 100644 0+ [0-9a-f]+ A\tsrc\/main\.c\n/);
  assert.match(text, /\nuser:Jane Doe\n1700000100\n/);
  assert.match(text, / D\tREADME/);
  assert.ok(seen.length > 0);
});

test('a repository without commits is reported as such', async () => {
  const dir = join(tempDir(), 'empty');
  mkdirSync(dir);
  git(dir, ['init', '-q', '-b', 'main']);
  await assert.rejects(readHistory(dir), (err) => err instanceof GitError && err.code === 'no_commits');
});

test('commit headers are counted across any chunk boundary', () => {
  const text = Buffer.from('user:a\n1\n:x A\tuser:file\n\nuser:b\n2\nm1:user:\n\nxuser:c\nuser:');
  for (let size = 1; size <= text.length; size++) {
    const counter = new CommitCounter();
    for (let i = 0; i < text.length; i += size) counter.push(text.subarray(i, i + size));
    assert.equal(counter.commits, 3, `chunk size ${size}`);
  }
});
