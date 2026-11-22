// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

// The README's "What it reads, and where it goes" stays true: each claim is checked against the code.

import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';

import { SOURCE, USAGE } from '../src/args.js';
import { LOG_ARGS, PRETTY_AI } from '../src/git.js';
import { HOSTED, privacyNotice } from '../src/main.js';
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
  for (const file of ['../src/main.js', '../src/server.js', '../src/git.js', '../src/open.js', '../src/args.js', '../src/style.js']) {
    const src = readFileSync(new URL(file, import.meta.url), 'utf8');
    assert.doesNotMatch(src, /\bfetch\(|https?\.request\(|https?\.get\(|'node:(?:https|net|tls|dgram|dns)'|\bconnect\(/, file);
  }
});

test('the notice printed before the history is read says only what the code does', () => {
  const [heading, reads, shows, source] = privacyNotice(false);
  assert.equal(heading, 'Your history stays on this computer.');
  // Names, emails, dates, subjects and paths: what the log format asks git for, and no contents (no -p, no --stat).
  assert.match(reads, /names, emails, dates, subjects and file paths, never file contents/);
  for (const field of ['%aN', '%an', '%ae', '%cn', '%ce', '%ct', '%s']) assert.ok(PRETTY_AI.includes(field), field);
  assert.ok(LOG_ARGS.includes('--raw'));
  for (const arg of LOG_ARGS) assert.doesNotMatch(arg, /^(-p|--patch|--stat|--numstat|-u|--unified)/);
  // Served on 127.0.0.1 only.
  assert.match(shows, /from 127\.0\.0\.1\. Nothing is uploaded\.$/);
  const server = readFileSync(new URL('../src/server.js', import.meta.url), 'utf8');
  assert.match(server, /server\.listen\(port, '127\.0\.0\.1'/);
  assert.match(privacyNotice(true)[2], /Saves them to the file you named\. Nothing is uploaded\.$/);
  // The source named is the package's own repository, also in --help and in this README.
  const pkg = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
  assert.equal(`git+${SOURCE}.git`, pkg.repository.url);
  assert.equal(source, `  - Inspect the code: ${SOURCE}`);
  assert.ok(USAGE.includes(SOURCE));
  assert.ok(README.includes(SOURCE));
  // The README quotes the notice as the command prints it.
  assert.ok(README.includes(privacyNotice(false).join('\n')));
});
