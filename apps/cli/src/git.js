// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

// The repository's history, read with Gource's git log command: the same
// format a clone service writes (PRETTY_AI in packages/protocol/src/git-log.ts)
// and the landing page's command
// prints. Commit times, author names and emails, AI trailers, subjects and
// the paths each commit touched; no file contents. The output is kept in
// memory, never written to disk.

import { spawn, spawnSync } from 'node:child_process';
import { basename } from 'node:path';

/** Gource's git log format with the AI attribution line (`m1:`). */
export const PRETTY_AI =
  '--pretty=format:user:%aN%n%ct%nm1:%an%x1f%ae%x1f%cn%x1f%ce%x1f%(trailers:key=Co-authored-by,key=Made-with,unfold,separator=%x1f)%x1e%s';

/** `git log` arguments, as an argument list: no shell, so no quoting differences between shells. */
export const LOG_ARGS = [
  '-c',
  'core.quotepath=off',
  'log',
  '--reverse',
  '--raw',
  '--encoding=UTF-8',
  '--no-renames',
  '--no-show-signature',
  '--no-color',
  PRETTY_AI,
];

export class GitError extends Error {
  /**
   * @param {'git_missing' | 'not_a_repo' | 'no_commits' | 'git_failed'} code
   * @param {string} message
   */
  constructor(code, message) {
    super(message);
    this.name = 'GitError';
    this.code = code;
  }
}

/**
 * The top folder of the work tree that contains `cwd`.
 * @param {string} cwd
 * @returns {{ root: string, name: string }}
 */
export function findRepository(cwd) {
  const out = spawnSync('git', ['rev-parse', '--show-toplevel'], { cwd, encoding: 'utf8' });
  if (out.error) throw gitMissing();
  const root = out.stdout.trim();
  if (out.status !== 0 || root === '') {
    const stderr = out.stderr.trim();
    if (stderr === '' || /not a git repository/i.test(stderr)) {
      throw new GitError(
        'not_a_repo',
        `${cwd} is not inside a git repository.\nRun it from your project's folder: cd into the repository, then npx gource-online.`,
      );
    }
    // Anything else (such as git refusing a folder another user owns): git's own words.
    const detail = stderr.split('\n').find((line) => line.startsWith('fatal:')) ?? stderr.split('\n')[0];
    throw new GitError(
      'git_failed',
      `git could not read the repository in ${cwd}:\n  ${detail}\nRun "git status" there to see what git needs.`,
    );
  }
  return { root, name: basename(root) || 'repository' };
}

function gitMissing() {
  return new GitError('git_missing', 'git was not found on this computer.\nInstall git (https://git-scm.com), then run this again.');
}

function noCommits() {
  return new GitError('no_commits', 'This repository has no commits yet.\nMake a first commit, then run this again.');
}

/**
 * @typedef {object} History
 * @property {Buffer[]} chunks  git's output, in order
 * @property {number} bytes
 * @property {number} commits
 * @property {number | null} earliest  the oldest commit time, unix seconds
 * @property {number | null} latest    the newest commit time, unix seconds
 */

/**
 * Runs the log command in `root` and keeps its output.
 * @param {string} root
 * @param {(commits: number, bytes: number) => void} [progress] called as output arrives
 * @returns {Promise<History>}
 */
export function readHistory(root, progress) {
  return new Promise((resolve, reject) => {
    const git = spawn('git', LOG_ARGS, { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] });
    /** @type {Buffer[]} */
    const chunks = [];
    const counter = new CommitCounter();
    let bytes = 0;
    let stderr = '';
    git.stdout.on('data', (/** @type {Buffer} */ chunk) => {
      chunks.push(chunk);
      bytes += chunk.length;
      counter.push(chunk);
      progress?.(counter.commits, bytes);
    });
    git.stderr.setEncoding('utf8');
    git.stderr.on('data', (/** @type {string} */ text) => {
      if (stderr.length < 8192) stderr += text;
    });
    git.on('error', () => reject(gitMissing()));
    git.on('close', (status) => {
      if (status !== 0) {
        const detail = stderr.trim().split('\n').pop() ?? '';
        if (/does not have any commits yet|bad default revision/.test(stderr)) {
          reject(noCommits());
        } else {
          reject(
            new GitError(
              'git_failed',
              `git log failed${detail ? `:\n  ${detail}` : ` (exit status ${status}).`}\nRun "git log" in the repository to see what git says.`,
            ),
          );
        }
        return;
      }
      if (counter.commits === 0) {
        reject(noCommits());
        return;
      }
      resolve({ chunks, bytes, commits: counter.commits, earliest: counter.earliest, latest: counter.latest });
    });
  });
}

const USER = Buffer.from('user:');

/**
 * Counts commit headers (lines starting with `user:`) across chunk boundaries,
 * and keeps the oldest and newest commit time: the line after each header.
 */
export class CommitCounter {
  commits = 0;
  /** @type {number | null} */
  earliest = null;
  /** @type {number | null} */
  latest = null;
  /** How much of `user:` the current line matched so far; -1 once it cannot match. */
  #pos = 0;
  /** 0: any line, 1: the rest of a header line, 2: the time line after it. */
  #state = 0;
  #time = 0;
  /** Digits on the time line so far; -1 once it holds anything else. */
  #digits = 0;

  /** @param {Uint8Array} chunk */
  push(chunk) {
    let pos = this.#pos;
    let state = this.#state;
    let time = this.#time;
    let digits = this.#digits;
    for (let i = 0; i < chunk.length; i++) {
      const b = chunk[i];
      if (b === 10) {
        if (state === 2 && digits > 0) this.#seen(time);
        state = state === 1 ? 2 : 0;
        time = 0;
        digits = 0;
        pos = 0;
      } else if (state === 2) {
        if (digits >= 0 && b >= 48 && b <= 57) {
          time = time * 10 + (b - 48);
          digits++;
        } else {
          digits = -1;
        }
      } else if (pos >= 0) {
        if (b === USER[pos]) {
          pos++;
          if (pos === USER.length) {
            this.commits++;
            pos = -1;
            state = 1;
          }
        } else {
          pos = -1;
        }
      }
    }
    this.#pos = pos;
    this.#state = state;
    this.#time = time;
    this.#digits = digits;
  }

  /** @param {number} time */
  #seen(time) {
    if (this.earliest === null || time < this.earliest) this.earliest = time;
    if (this.latest === null || time > this.latest) this.latest = time;
  }
}
