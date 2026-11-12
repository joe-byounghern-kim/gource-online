// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

// Command-line options. The command takes no arguments: it always reads the
// repository of the current folder.

import { parseArgs } from 'node:util';

export const USAGE = `Usage: npx gource-online [options]

Run it anywhere inside a git repository. It reads the history with git,
starts Gource on this computer and opens it in your browser. Nothing is
uploaded.

Options:
  --port <n>         Port on 127.0.0.1 (default: any free port)
  --no-open          Print the link instead of opening the browser
  --output <file>    Write the history to <file> and exit, to open it in
                     Gource later ("Open a local log file")
  -h, --help         Show this help
  -v, --version      Show the version
`;

/**
 * @typedef {object} Options
 * @property {'run' | 'help' | 'version'} action
 * @property {number} port
 * @property {boolean} open
 * @property {string | null} output
 */

export class UsageError extends Error {
  /** @param {string} message */
  constructor(message) {
    super(message);
    this.name = 'UsageError';
  }
}

/**
 * @param {string[]} argv  arguments after the command name
 * @returns {Options}
 */
export function parseOptions(argv) {
  let parsed;
  try {
    parsed = parseArgs({
      args: argv,
      allowPositionals: true,
      strict: true,
      options: {
        port: { type: 'string' },
        'no-open': { type: 'boolean' },
        output: { type: 'string' },
        help: { type: 'boolean', short: 'h' },
        version: { type: 'boolean', short: 'v' },
      },
    });
  } catch (err) {
    throw new UsageError(err instanceof Error ? err.message : String(err));
  }
  const { values, positionals } = parsed;
  if (values.help) return { action: 'help', port: 0, open: true, output: null };
  if (values.version) return { action: 'version', port: 0, open: true, output: null };
  if (positionals.length > 0) {
    throw new UsageError(
      `Unexpected argument "${positionals[0]}". Run the command inside the repository instead (cd into it first).`,
    );
  }
  let port = 0;
  if (values.port !== undefined) {
    port = Number(values.port);
    if (!/^\d+$/.test(values.port) || port < 1 || port > 65535) {
      throw new UsageError(`--port must be a number from 1 to 65535, not "${values.port}".`);
    }
  }
  if (values.output !== undefined && values.output.trim() === '') {
    throw new UsageError('--output needs a file name, such as --output history.gitlog.');
  }
  return { action: 'run', port, open: !values['no-open'], output: values.output ?? null };
}
