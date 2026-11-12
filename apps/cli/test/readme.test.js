// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

// The README's "What it reads, and where it goes" stays true: each claim is checked against the code.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

import { LOG_ARGS } from '../src/git.js';
import { HOSTED } from '../src/main.js';
import { CSP } from '../src/server.js';

const README = readFileSync(new URL('../README.md', import.meta.url), 'utf8');
const section = README.slice(README.indexOf('## What it reads, and where it goes'), README.indexOf('## Options'));

test('the git command in the README is the one the command runs', () => {
  const block = /```\n([\s\S]+?)\n\s*```/.exec(section)?.[1] ?? '';
  const shown = block.split('\n').map((line) => line.trim()).join(' ').replace(/\\ /g, '').trim();
  const run = ['git', ...LOG_ARGS.map((arg) => (arg.startsWith('--pretty=format:') ? `--pretty=format:"${arg.slice('--pretty=format:'.length)}"` : arg))].join(' ');
  assert.equal(shown, run);
});

test('the page may connect to this server only, and no page may embed it', () => {
  assert.match(section, /`connect-src 'self'`/);
  assert.match(CSP, /(^|; )connect-src 'self';/);
  assert.match(section, /`frame-ancestors 'none'`/);
  assert.match(CSP, /(^|; )frame-ancestors 'none';/);
});

test('the hosted site is named as plain text, which the command never contacts', () => {
  assert.match(section, /names the hosted site, gource\.online, as plain text/);
  assert.match(HOSTED, /https:\/\/gource\.online$/);
  for (const file of ['../src/main.js', '../src/server.js', '../src/git.js', '../src/open.js', '../src/args.js']) {
    const src = readFileSync(new URL(file, import.meta.url), 'utf8');
    assert.doesNotMatch(src, /\bfetch\(|https?\.request\(|https?\.get\(|'node:(?:https|net|tls|dgram|dns)'|\bconnect\(/, file);
  }
});
