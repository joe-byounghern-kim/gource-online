// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

// Command-line options. The command takes no arguments: it always reads the
// repository of the current folder.

import { parseArgs } from 'node:util';

/** Where the command's source can be read: the public repository (package.json `repository`). */
export const SOURCE = 'https://github.com/joe-byounghern-kim/gource-online';

export const USAGE = `Usage: npx gource-online [options]

Plays your git repository's history in the browser. Run it anywhere inside
the repository: it reads the history with git, serves Gource from 127.0.0.1
and opens it in your browser. Nothing is uploaded.

Options:
  --port <n>         Port on 127.0.0.1 (default: any free port)
  --no-open          Print the link instead of opening the browser
  --output <file>    Save the history to <file> and exit, to open it in the
                     Gource player later
  -h, --help         Show this help
  -v, --version      Show the version

Source (GPL-3.0-or-later): ${SOURCE}
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
    throw new UsageError(parseError(err));
  }
  const { values, positionals } = parsed;
  if (values.help) return { action: 'help', port: 0, open: true, output: null };
  if (values.version) return { action: 'version', port: 0, open: true, output: null };
  if (positionals.length > 0) {
    throw new UsageError(
      `Unexpected argument "${positionals[0]}".\nThe command takes no folder: cd into the repository and run it there.`,
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

/**
 * Node's message for a mistake parseArgs found, in this command's words.
 * @param {unknown} err
 */
function parseError(err) {
  const message = err instanceof Error ? err.message : String(err);
  const code = /** @type {{ code?: string }} */ (err).code;
  const unknown = /^Unknown option '([^']+)'/.exec(message)?.[1];
  if (code === 'ERR_PARSE_ARGS_UNKNOWN_OPTION' && unknown) return `Unknown option "${unknown}".`;
  const option = /^Option '(?:-\w, )?(--[^' ]+)/.exec(message)?.[1];
  if (code === 'ERR_PARSE_ARGS_INVALID_OPTION_VALUE' && option) {
    if (/argument missing/.test(message)) {
      if (option === '--output') return '--output needs a file name, such as --output history.gitlog.';
      if (option === '--port') return '--port needs a number from 1 to 65535, such as --port 8080.';
      return `${option} needs a value.`;
    }
    if (/does not take an argument/.test(message)) return `${option} does not take a value.`;
  }
  return message;
}
