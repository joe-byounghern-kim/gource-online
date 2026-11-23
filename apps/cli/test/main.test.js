// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { mkdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

import { SOURCE } from '../src/args.js';
import { main, privacyNotice, summary, version } from '../src/main.js';
import { fakeDist, git, makeRepo, tempDir } from './helpers.js';

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

/**
 * Collects what main() prints.
 * @param {boolean} [isTTY]
 */
function collector(isTTY = false) {
  const sink = { text: '', isTTY, write: (/** @type {string} */ s) => (sink.text += s) };
  return sink;
}

const ANSI = /\x1b\[/;

test('--version and --help', async () => {
  const pkg = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
  assert.equal(version(), pkg.version);
  const v = await run(['--version'], tempDir());
  assert.equal(v.stdout.trim(), pkg.version);
  const h = await run(['--help'], tempDir());
  assert.equal(h.status, 0);
  assert.match(h.stdout, /Usage: npx gource-online/);
  assert.ok(h.stdout.includes(`Source (GPL-3.0-or-later): ${SOURCE}`));
  for (const line of h.stdout.split('\n')) assert.ok(line.length <= 80, line);
});

test('usage mistakes exit with status 2', async () => {
  const res = await run(['my-folder'], tempDir());
  assert.equal(res.status, 2);
  assert.equal(
    res.stderr,
    'gource-online: Unexpected argument "my-folder".\n' +
      'The command takes no folder: cd into the repository and run it there.\n' +
      'Run "npx gource-online --help" to see the options.\n',
  );
  const unknown = await run(['--frobnicate'], tempDir());
  assert.equal(unknown.status, 2);
  assert.match(unknown.stderr, /^gource-online: Unknown option "--frobnicate"\.\n/);
});

test('outside a repository it says what to do', async () => {
  const res = await run([], tempDir());
  assert.equal(res.status, 1);
  assert.match(res.stderr, /^gource-online: \S+ is not inside a git repository\.\nRun it from your project's folder/);
  // Nothing else: no notice, no version line, and plain text when piped.
  assert.equal(res.stdout, '');
  assert.doesNotMatch(res.stderr, ANSI);
});

test('a repository without commits says what to do', async () => {
  const dir = join(tempDir(), 'empty');
  mkdirSync(dir);
  git(dir, ['init', '-q', '-b', 'main']);
  const res = await run([], dir);
  assert.equal(res.status, 1);
  assert.match(res.stderr, /^gource-online: This repository has no commits yet\.\nMake a first commit, then run this again\.\n$/);
});

test('--output writes the history next to where it ran', async () => {
  const repo = makeRepo();
  const res = await run(['--output', 'history.gitlog'], join(repo, 'src'));
  assert.equal(res.status, 0, res.stderr);
  assert.match(res.stdout, /\n  2 commits, \d+ B, \d{4}-\d\d-\d\d( to \d{4}-\d\d-\d\d)?\n/);
  assert.ok(res.stdout.includes(privacyNotice(true).join('\n')), res.stdout);
  assert.match(res.stdout, /\nSaved the history to .*history\.gitlog\n/);
  assert.doesNotMatch(res.stdout, ANSI);
  const text = readFileSync(join(repo, 'src', 'history.gitlog'), 'utf8');
  assert.ok(text.startsWith('user:Jane Doe\n1700000000\n'));
});

/**
 * Starts main() on a repository with a stand-in web app and returns what it
 * printed once the server is up, plus a function that stops it.
 * @param {string[]} argv
 * @param {NodeJS.ProcessEnv} env
 */
async function serve(argv, env = {}, isTTY = false) {
  const repo = makeRepo('served');
  const out = collector(isTTY);
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
  return { repo, out, opened, stop, status };
}

test('it serves the history and opens the browser on it', async () => {
  const { repo, out, opened, stop, status } = await serve([]);
  try {
    const url = /http:\/\/127\.0\.0\.1:\d+\/#src=\S+/.exec(out.text)?.[0];
    assert.ok(url, out.text);
    assert.deepEqual(opened, [url]);
    // Piped, it is plain text, and the whole of it reads as below.
    assert.doesNotMatch(out.text, ANSI);
    const expected = [
      `gource-online ${version()}`,
      '',
      ...privacyNotice(false),
      '',
      `Reading the history of ${git(repo, ['rev-parse', '--show-toplevel']).trim()}`,
      `  ${summary({ commits: 2, bytes: Number(/2 commits, (\d+) B/.exec(out.text)?.[1]), earliest: 1700000000, latest: 1700000100 })}`,
      '',
      'Gource is running at:',
      '',
      `  ${url}`,
      '',
      'Opening it in your browser. Press Ctrl+C to stop.',
      '',
      'Public repositories and sign-in features: https://gource.online',
      '',
    ].join('\n');
    assert.equal(out.text, expected);
    // Everything but the folder and the link fits a plain 80-column terminal.
    for (const line of out.text.split('\n')) {
      if (!line.includes('#src=') && !line.startsWith('Reading the history of')) assert.ok(line.length <= 80, line);
    }
    const log = new URLSearchParams(new URL(url).hash.slice(1)).get('src') ?? '';
    const res = await fetch(new URL(log, url));
    assert.equal(res.status, 200);
    assert.match(await res.text(), /^user:Jane Doe\n1700000000\nm1:/);
  } finally {
    stop();
  }
  assert.equal(await status, 0);
});

test('the privacy notice comes before the history is read', async () => {
  const { out, stop, status } = await serve(['--no-open']);
  stop();
  await status;
  const notice = out.text.indexOf(privacyNotice(false).join('\n'));
  assert.ok(notice > 0, out.text);
  assert.ok(notice < out.text.indexOf('Reading the history of'));
  assert.ok(out.text.indexOf('Reading the history of') < out.text.indexOf('2 commits'));
});

test('in a terminal it adds a little emphasis, never on the link, and none for NO_COLOR or FORCE_COLOR=0', async () => {
  const tty = await serve(['--no-open'], {}, true);
  tty.stop();
  await tty.status;
  assert.match(tty.out.text, /^\x1b\[1mgource-online /);
  assert.match(tty.out.text, /\n\x1b\[1mGource is running at:\x1b\[22m\n\n {2}http:\/\/127\.0\.0\.1:\d+\/#src=[^\s\x1b]+\n/);
  // The count is redrawn in place, and the final line leaves no remnant of a longer one.
  assert.match(tty.out.text, /\r {2}2 commits, \d+ B, /);
  for (const env of [{ NO_COLOR: '1' }, { FORCE_COLOR: '0' }, { TERM: 'dumb' }]) {
    const plain = await serve(['--no-open'], env, true);
    plain.stop();
    await plain.status;
    assert.doesNotMatch(plain.out.text, ANSI, JSON.stringify(env));
  }
  // Piped, FORCE_COLOR (set by CI services and test runners) adds nothing: scripts read the link.
  const piped = await serve(['--no-open'], { FORCE_COLOR: '1' });
  piped.stop();
  await piped.status;
  assert.doesNotMatch(piped.out.text, ANSI);
  assert.doesNotMatch(piped.out.text, /\r/);
});

test('--no-open and SSH sessions only print the link', async () => {
  for (const [argv, env, hint] of /** @type {const} */ ([
    [['--no-open'], {}, false],
    [[], { SSH_CONNECTION: '10.0.0.1 1 10.0.0.2 22' }, true],
  ])) {
    const { out, opened, stop, status } = await serve([...argv], env);
    stop();
    assert.deepEqual(opened, []);
    assert.equal(/\n {2}ssh -L \d+:127\.0\.0\.1:\d+ <this host>\n/.test(out.text), hint, out.text);
    assert.equal(out.text.includes('Open the link in your browser. Press Ctrl+C to stop.'), !hint, out.text);
    assert.equal(await status, 0);
  }
});

test('a port in use is reported', async () => {
  const first = await serve([]);
  const port = /127\.0\.0\.1:(\d+)/.exec(first.out.text)?.[1];
  const second = await serve(['--port', String(port)]);
  first.stop();
  assert.equal(await second.status, 1);
  assert.match(second.out.text, new RegExp(`gource-online: port ${port} is already in use\\.\\nChoose another with --port <n>`));
  await first.status;
});

test('the summary counts commits and spans the days they were made on', () => {
  assert.equal(summary({ commits: 1, bytes: 512, earliest: null, latest: null }), '1 commit, 512 B');
  const day = new Date(2023, 10, 14, 12).getTime() / 1000;
  assert.equal(summary({ commits: 2, bytes: 2048, earliest: day, latest: day + 3600 }), '2 commits, 2.0 KB, 2023-11-14');
  assert.equal(
    summary({ commits: 4567, bytes: 3 * 1024 * 1024, earliest: day, latest: day + 2 * 86400 }),
    '4,567 commits, 3.0 MB, 2023-11-14 to 2023-11-16',
  );
});
