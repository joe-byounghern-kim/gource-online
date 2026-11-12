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
  if (out.error) {
    throw new GitError('git_missing', 'git was not found. Install git and run this again.');
  }
  const root = out.stdout.trim();
  if (out.status !== 0 || root === '') {
    const detail = out.stderr.trim().split('\n').pop() ?? '';
    throw new GitError(
      'not_a_repo',
      `${cwd} is not inside a git repository. Run this in your project's folder.${detail ? `\n  (${detail})` : ''}`,
    );
  }
  return { root, name: basename(root) || 'repository' };
}

/**
 * @typedef {object} History
 * @property {Buffer[]} chunks  git's output, in order
 * @property {number} bytes
 * @property {number} commits
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
    git.on('error', () => reject(new GitError('git_missing', 'git was not found. Install git and run this again.')));
    git.on('close', (status) => {
      if (status !== 0) {
        const detail = stderr.trim().split('\n').pop() ?? '';
        if (/does not have any commits yet|bad default revision/.test(stderr)) {
          reject(new GitError('no_commits', 'This repository has no commits yet.'));
        } else {
          reject(new GitError('git_failed', `git log failed${detail ? `: ${detail}` : ` (exit status ${status})`}`));
        }
        return;
      }
      if (counter.commits === 0) {
        reject(new GitError('no_commits', 'This repository has no commits yet.'));
        return;
      }
      resolve({ chunks, bytes, commits: counter.commits });
    });
  });
}

const USER = Buffer.from('user:');

/** Counts commit headers (lines starting with `user:`) across chunk boundaries. */
export class CommitCounter {
  commits = 0;
  /** How much of `user:` the current line matched so far; -1 once it cannot match. */
  #pos = 0;

  /** @param {Uint8Array} chunk */
  push(chunk) {
    let pos = this.#pos;
    for (let i = 0; i < chunk.length; i++) {
      const b = chunk[i];
      if (b === 10) {
        pos = 0;
      } else if (pos >= 0) {
        if (b === USER[pos]) {
          pos++;
          if (pos === USER.length) {
            this.commits++;
            pos = -1;
          }
        } else {
          pos = -1;
        }
      }
    }
    this.#pos = pos;
  }
}
