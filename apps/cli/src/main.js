// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

// `npx gource-online`: read the history of the repository around the current
// folder, serve it with the bundled web app on 127.0.0.1 and open the
// browser on it. Runs until Ctrl+C.

import { existsSync, readFileSync } from 'node:fs';
import { writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { SOURCE, USAGE, UsageError, parseOptions } from './args.js';
import { GitError, findRepository, readHistory } from './git.js';
import { isRemoteShell, openBrowser } from './open.js';
import { startServer } from './server.js';
import { style, useColor } from './style.js';

/**
 * @typedef {object} Io
 * @property {string} cwd
 * @property {NodeJS.ProcessEnv} env
 * @property {NodeJS.WriteStream | { write(s: string): unknown, isTTY?: boolean }} out
 * @property {NodeJS.WriteStream | { write(s: string): unknown, isTTY?: boolean }} err
 * @property {string} dist                 the bundled player (apps/player/dist)
 * @property {(url: string) => Promise<boolean>} open
 * @property {() => Promise<void>} stopped resolves when the user stops the server
 */

/** @returns {string} */
export function version() {
  const pkg = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
  return String(pkg.version);
}

/** @returns {Io} */
function processIo() {
  return {
    cwd: process.cwd(),
    env: process.env,
    out: process.stdout,
    err: process.stderr,
    dist: fileURLToPath(new URL('../dist/', import.meta.url)),
    open: openBrowser,
    stopped: () =>
      new Promise((done) => {
        process.once('SIGINT', () => done());
        process.once('SIGTERM', () => done());
      }),
  };
}

/**
 * What the command reads and where it goes, printed before it reads anything.
 * Every line is true of the code (README.md, "What it reads, and where it goes";
 * test/readme.test.js checks it).
 * @param {boolean} saving  --output: the history goes to a file, not to a browser
 * @returns {string[]}
 */
export function privacyNotice(saving) {
  return [
    'Your history stays on this computer.',
    '  - Reads names, emails, dates, subjects and file paths, never file contents.',
    saving
      ? '  - Saves them to the file you named. Nothing is uploaded.'
      : '  - Shows them in your browser from 127.0.0.1. Nothing is uploaded.',
    `  - Inspect the code: ${SOURCE}`,
  ];
}

/**
 * @param {string[]} argv  arguments after the command name
 * @param {Partial<Io>} [overrides]
 * @returns {Promise<number>} the exit status
 */
export async function main(argv, overrides = {}) {
  const io = { ...processIo(), ...overrides };
  const out = style(useColor(io.out, io.env));
  const errStyle = style(useColor(io.err, io.env));
  const say = (/** @type {string} */ s = '') => io.out.write(`${s}\n`);
  // An error: what happened on the first line, after the command's name; what to do next below it.
  const fail = (/** @type {string} */ s) => {
    const [first, ...rest] = s.split('\n');
    io.err.write(`${errStyle.red('gource-online:')} ${first}\n${rest.map((line) => `${line}\n`).join('')}`);
  };

  let options;
  try {
    options = parseOptions(argv);
  } catch (err) {
    if (!(err instanceof UsageError)) throw err;
    fail(`${err.message}\nRun "npx gource-online --help" to see the options.`);
    return 2;
  }
  if (options.action === 'help') {
    io.out.write(USAGE);
    return 0;
  }
  if (options.action === 'version') {
    say(version());
    return 0;
  }

  let repo;
  let history;
  /** The longest count line drawn in place so far (in a terminal). */
  let width = 0;
  try {
    repo = findRepository(io.cwd);
    say(out.bold(`gource-online ${version()}`));
    say();
    const [heading, ...points] = privacyNotice(options.output !== null);
    say(out.bold(heading));
    for (const point of points) say(point);
    say();
    say(`Reading the history of ${repo.root}`);
    // In a terminal, the counts update in place; piped, only the final line is written.
    const live = Boolean(io.out.isTTY);
    let last = 0;
    history = await readHistory(repo.root, (commits, bytes) => {
      const now = Date.now();
      if (!live || now - last < 100) return;
      last = now;
      const line = `  ${commits === 1 ? '1 commit' : `${count(commits)} commits`}, ${size(bytes)}`;
      width = Math.max(width, line.length);
      io.out.write(`\r${line}`);
    });
    const line = `  ${summary(history)}`;
    io.out.write(`${live ? '\r' : ''}${line.padEnd(width)}\n`);
  } catch (err) {
    if (!(err instanceof GitError)) throw err;
    if (width > 0) io.out.write('\n'); // end the count line drawn in place
    fail(err.message);
    return 1;
  }

  if (options.output) {
    const file = resolve(io.cwd, options.output);
    try {
      await writeFile(file, history.chunks);
    } catch (err) {
      fail(`could not save the history to ${file}.\n  ${err instanceof Error ? err.message : String(err)}`);
      return 1;
    }
    say();
    say(`Saved the history to ${file}`);
    say('Open this file in the Gource player to watch it later.');
    return 0;
  }

  if (!existsSync(resolve(io.dist, 'index.html'))) {
    fail(`the player is missing from this package (${io.dist}).\nIf you built the package from source, run scripts/build-cli.sh first.`);
    return 1;
  }
  let server;
  try {
    server = await startServer({ dist: io.dist, log: history.chunks, name: repo.name, port: options.port });
  } catch (err) {
    const code = /** @type {{ code?: string }} */ (err).code;
    if (code === 'EADDRINUSE') {
      fail(`port ${options.port} is already in use.\nChoose another with --port <n>, or leave out --port to use any free port.`);
      return 1;
    }
    if (code === 'EACCES') {
      fail(`port ${options.port} needs administrator rights on this computer.\nChoose a port above 1023, or leave out --port to use any free port.`);
      return 1;
    }
    throw err;
  }

  say();
  say(out.bold('Gource is running at:'));
  say();
  // The link on its own line and never styled, in a terminal or not: scripts and tests read it.
  say(`  ${server.url}`);
  say();
  if (isRemoteShell(io.env)) {
    say('This looks like an SSH session, so the link works on this machine only.');
    say('To open it on your computer, forward the port from there:');
    say();
    say(`  ssh -L ${server.port}:127.0.0.1:${server.port} <this host>`);
    say();
    say('Or stop this and save the history to a file with --output history.gitlog.');
    say('Press Ctrl+C to stop.');
  } else if (options.open && (await io.open(server.url))) {
    say('Opening it in your browser. Press Ctrl+C to stop.');
  } else {
    say('Open the link in your browser. Press Ctrl+C to stop.');
  }
  say();
  // A plain line only (D18): the command makes no request, the page shows no link.
  say(out.dim(HOSTED));

  await io.stopped();
  await server.close();
  return 0;
}

/** The hosted site, as one plain line in the terminal. */
export const HOSTED = 'Public repositories and sign-in features: https://gource.online';

/**
 * `456 commits, 204.0 KB, 2024-03-01 to 2026-11-17`: how much was read, and
 * the days it spans (this computer's time zone).
 * @param {{ commits: number, bytes: number, earliest: number | null, latest: number | null }} history
 */
export function summary({ commits, bytes, earliest, latest }) {
  const parts = [commits === 1 ? '1 commit' : `${count(commits)} commits`, size(bytes)];
  if (earliest !== null && latest !== null) {
    const from = day(earliest);
    const to = day(latest);
    parts.push(from === to ? from : `${from} to ${to}`);
  }
  return parts.join(', ');
}

/** @param {number} seconds  unix time */
function day(seconds) {
  const d = new Date(seconds * 1000);
  const two = (/** @type {number} */ n) => String(n).padStart(2, '0');
  return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())}`;
}

/** @param {number} n */
function count(n) {
  return n.toLocaleString('en-US');
}

/** @param {number} bytes */
function size(bytes) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}
