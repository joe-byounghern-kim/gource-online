// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

// Opens a link in the default browser, without a shell (the link is passed
// as one argument, so no characters in it are interpreted).

import { spawn } from 'node:child_process';

/**
 * The command that opens `url` on this platform, or null when there is
 * nothing to try.
 * @param {string} url
 * @param {NodeJS.Platform} [platform]
 * @param {NodeJS.ProcessEnv} [env]
 * @returns {[string, string[]] | null}
 */
export function openCommand(url, platform = process.platform, env = process.env) {
  if (platform === 'darwin') return ['open', [url]];
  // Go's pkg/browser does the same: no cmd.exe, so `&` and `%` stay literal.
  if (platform === 'win32') return ['rundll32', ['url.dll,FileProtocolHandler', url]];
  if (env.WSL_DISTRO_NAME) return ['rundll32.exe', ['url.dll,FileProtocolHandler', url]];
  if (platform === 'linux' || platform === 'freebsd' || platform === 'openbsd') return ['xdg-open', [url]];
  return null;
}

/**
 * Whether this looks like a shell on another machine (SSH), where a browser
 * here would not reach the server anyway.
 * @param {NodeJS.ProcessEnv} [env]
 */
export function isRemoteShell(env = process.env) {
  return Boolean(env.SSH_CONNECTION || env.SSH_TTY);
}

/**
 * Starts the browser; resolves to whether the command could be started.
 * @param {string} url
 * @returns {Promise<boolean>}
 */
export function openBrowser(url) {
  const command = openCommand(url);
  if (!command) return Promise.resolve(false);
  return new Promise((done) => {
    const child = spawn(command[0], command[1], { stdio: 'ignore', detached: true });
    child.once('error', () => done(false));
    child.once('spawn', () => {
      child.unref();
      done(true);
    });
  });
}
