// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

// `npx gource-online`: read the history of the repository around the current
// folder, serve it with the bundled web app on 127.0.0.1 and open the
// browser on it. Runs until Ctrl+C.

import { existsSync, readFileSync } from 'node:fs';
import { writeFile } from 'node:fs/promises';
import { resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import { USAGE, UsageError, parseOptions } from './args.js';
import { GitError, findRepository, readHistory } from './git.js';
import { isRemoteShell, openBrowser } from './open.js';
import { startServer } from './server.js';

/**
 * @typedef {object} Io
 * @property {string} cwd
 * @property {NodeJS.ProcessEnv} env
 * @property {NodeJS.WriteStream | { write(s: string): unknown, isTTY?: boolean }} out
 * @property {NodeJS.WriteStream | { write(s: string): unknown }} err
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
 * @param {string[]} argv  arguments after the command name
 * @param {Partial<Io>} [overrides]
 * @returns {Promise<number>} the exit status
 */
export async function main(argv, overrides = {}) {
  const io = { ...processIo(), ...overrides };
  const say = (/** @type {string} */ s) => io.out.write(`${s}\n`);
  const fail = (/** @type {string} */ s) => io.err.write(`${s}\n`);

  let options;
  try {
    options = parseOptions(argv);
  } catch (err) {
    if (!(err instanceof UsageError)) throw err;
    fail(`gource-online: ${err.message}`);
    fail('Run "npx gource-online --help" for the options.');
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
  try {
    repo = findRepository(io.cwd);
    say(`gource-online ${version()}`);
    say(`Reading the history of ${repo.root}`);
    const live = Boolean(io.out.isTTY);
    let last = 0;
    history = await readHistory(repo.root, (commits, bytes) => {
      const now = Date.now();
      if (!live || now - last < 100) return;
      last = now;
      io.out.write(`\r  ${count(commits)} commits, ${size(bytes)}`);
    });
    io.out.write(`${live ? '\r' : ''}  ${count(history.commits)} commits, ${size(history.bytes)}\n`);
  } catch (err) {
    if (!(err instanceof GitError)) throw err;
    fail(`gource-online: ${err.message}`);
    return 1;
  }

  if (options.output) {
    const file = resolve(io.cwd, options.output);
    try {
      await writeFile(file, history.chunks);
    } catch (err) {
      fail(`gource-online: could not write ${file}: ${err instanceof Error ? err.message : String(err)}`);
      return 1;
    }
    say(`Wrote ${file}`);
    say('Open it in Gource with "Open a local log file". It never leaves your computer.');
    return 0;
  }

  if (!existsSync(resolve(io.dist, 'index.html'))) {
    fail(`gource-online: the player is missing from this package (${io.dist}).`);
    fail('A copy built from the source tree needs scripts/build-cli.sh first.');
    return 1;
  }
  let server;
  try {
    server = await startServer({ dist: io.dist, log: history.chunks, name: repo.name, port: options.port });
  } catch (err) {
    const code = /** @type {{ code?: string }} */ (err).code;
    if (code === 'EADDRINUSE') {
      fail(`gource-online: port ${options.port} is in use. Pick another with --port, or leave --port out.`);
      return 1;
    }
    throw err;
  }

  say('');
  say('Gource is running on this computer:');
  say('');
  say(`  ${server.url}`);
  say('');
  if (isRemoteShell(io.env)) {
    say('This looks like an SSH session: the link works on this machine only. From your computer,');
    say(`forward the port (ssh -L ${server.port}:127.0.0.1:${server.port} <this host>) and open the link there,`);
    say('or stop this and use --output history.gitlog to make a file.');
  } else if (options.open) {
    const opened = await io.open(server.url);
    if (!opened) say('Open the link above in your browser.');
  }
  say('The history stays on this computer. Press Ctrl+C to stop.');
  say('');
  // A plain line only (D18): the command makes no request, the page shows no link.
  say(HOSTED);

  await io.stopped();
  await server.close();
  return 0;
}

/** The hosted site, as one plain line in the terminal. */
export const HOSTED = 'Public repositories and sign-in features: https://gource.online';

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
