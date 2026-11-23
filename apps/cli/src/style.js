// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

// A little emphasis in a terminal, and none anywhere else: plain text when
// the output is piped or redirected, when NO_COLOR or NODE_DISABLE_COLORS is
// set (no-color.org), when FORCE_COLOR is "0" or "false", or TERM is dumb.
// FORCE_COLOR never adds codes to piped output: CI services and test runners
// set it for their own output, and scripts read the link from ours.

/**
 * @typedef {object} Style
 * @property {(s: string) => string} bold
 * @property {(s: string) => string} dim
 * @property {(s: string) => string} red
 */

/**
 * Whether to colour what goes to `stream`.
 * @param {{ isTTY?: boolean }} stream
 * @param {NodeJS.ProcessEnv} env
 */
export function useColor(stream, env) {
  if (env.NO_COLOR || env.NODE_DISABLE_COLORS) return false;
  if (env.FORCE_COLOR === '0' || env.FORCE_COLOR?.toLowerCase() === 'false') return false;
  if (env.TERM === 'dumb') return false;
  return Boolean(stream.isTTY);
}

/** @param {string} open @param {string} close */
const sgr = (open, close) => (/** @type {string} */ s) => `\x1b[${open}m${s}\x1b[${close}m`;
const plain = (/** @type {string} */ s) => s;

/**
 * @param {boolean} color
 * @returns {Style}
 */
export function style(color) {
  if (!color) return { bold: plain, dim: plain, red: plain };
  return { bold: sgr('1', '22'), dim: sgr('2', '22'), red: sgr('31', '39') };
}
