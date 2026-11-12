// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import { main, version } from '../src/main.js';
import { fakeDist, makeRepo, tempDir } from './helpers.js';

const BIN = fileURLToPath(new URL('../bin/gource-online.js', import.meta.url));

/**
 * Runs the command in `cwd`.
 * @param {string[]} args
 * @param {string} cwd
 * @returns {Promise<{ status: number, stdout: string, stderr: string }>}
 */
function run(args, cwd) {
  return new Promise((done) => {
    execFile(process.execPath, [BIN, ...args], { cwd }, (err, stdout, stderr) => {
      done({ status: err ? Number(err.code ?? 1) : 0, stdout, stderr });
    });
  });
}

/** Collects what main() prints. */
function collector() {
  const sink = { text: '', write: (/** @type {string} */ s) => (sink.text += s) };
  return sink;
}

test('--version and --help', async () => {
  const pkg = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
  assert.equal(version(), pkg.version);
  const v = await run(['--version'], tempDir());
  assert.equal(v.stdout.trim(), pkg.version);
  const h = await run(['--help'], tempDir());
  assert.equal(h.status, 0);
  assert.match(h.stdout, /Usage: npx gource-online/);
});

test('usage mistakes exit with status 2', async () => {
  const res = await run(['my-folder'], tempDir());
  assert.equal(res.status, 2);
  assert.match(res.stderr, /Run the command inside the repository/);
});

test('outside a repository it says what to do', async () => {
  const res = await run([], tempDir());
  assert.equal(res.status, 1);
  assert.match(res.stderr, /is not inside a git repository/);
});

test('--output writes the history next to where it ran', async () => {
  const repo = makeRepo();
  const res = await run(['--output', 'history.gitlog'], join(repo, 'src'));
  assert.equal(res.status, 0, res.stderr);
  assert.match(res.stdout, /2 commits/);
  const text = readFileSync(join(repo, 'src', 'history.gitlog'), 'utf8');
  assert.ok(text.startsWith('user:Jane Doe\n1700000000\n'));
});

/**
 * Starts main() on a repository with a stand-in web app and returns what it
 * printed once the server is up, plus a function that stops it.
 * @param {string[]} argv
 * @param {NodeJS.ProcessEnv} env
 */
async function serve(argv, env = {}) {
  const repo = makeRepo('served');
  const out = collector();
  /** @type {string[]} */
  const opened = [];
  /** @type {() => void} */
  let stop = () => {};
  const stopped = new Promise((done) => {
    stop = () => done(undefined);
  });
  let ready = () => {};
  const up = new Promise((done) => {
    ready = () => done(undefined);
  });
  const status = main(argv, {
    cwd: repo,
    env,
    out,
    err: out,
    dist: fakeDist(),
    open: async (url) => {
      opened.push(url);
      return true;
    },
    stopped: () => {
      ready();
      return /** @type {Promise<void>} */ (stopped);
    },
  });
  await Promise.race([up, status]);
  return { out, opened, stop, status };
}

test('it serves the history and opens the browser on it', async () => {
  const { out, opened, stop, status } = await serve([]);
  const url = /http:\/\/127\.0\.0\.1:\d+\/#src=\S+/.exec(out.text)?.[0];
  assert.ok(url, out.text);
  assert.deepEqual(opened, [url]);
  assert.match(out.text, /Reading the history of .*served\n  2 commits, \d+ B\n/);
  assert.match(out.text, /Press Ctrl\+C to stop/);
  // The hosted site, as one plain line (D18).
  assert.match(out.text, /\nPublic repositories and sign-in features: https:\/\/gource\.online\n/);
  const log = new URLSearchParams(new URL(url).hash.slice(1)).get('src') ?? '';
  const res = await fetch(new URL(log, url));
  assert.equal(res.status, 200);
  assert.match(await res.text(), /^user:Jane Doe\n1700000000\nm1:/);
  stop();
  assert.equal(await status, 0);
});

test('--no-open and SSH sessions only print the link', async () => {
  for (const [argv, env, hint] of /** @type {const} */ ([
    [['--no-open'], {}, false],
    [[], { SSH_CONNECTION: '10.0.0.1 1 10.0.0.2 22' }, true],
  ])) {
    const { out, opened, stop, status } = await serve([...argv], env);
    assert.deepEqual(opened, []);
    assert.equal(/ssh -L \d+:127\.0\.0\.1:\d+/.test(out.text), hint, out.text);
    stop();
    assert.equal(await status, 0);
  }
});

test('a port in use is reported', async () => {
  const first = await serve([]);
  const port = /127\.0\.0\.1:(\d+)/.exec(first.out.text)?.[1];
  const second = await serve(['--port', String(port)]);
  assert.equal(await second.status, 1);
  assert.match(second.out.text, new RegExp(`port ${port} is in use`));
  first.stop();
  await first.status;
});
