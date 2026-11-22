// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { UsageError, parseOptions } from '../src/args.js';

test('defaults: any port, open the browser, no file', () => {
  assert.deepEqual(parseOptions([]), { action: 'run', port: 0, open: true, output: null });
});

test('options', () => {
  assert.deepEqual(parseOptions(['--port', '8080', '--no-open']), { action: 'run', port: 8080, open: false, output: null });
  assert.deepEqual(parseOptions(['--port=9000']), { action: 'run', port: 9000, open: true, output: null });
  assert.deepEqual(parseOptions(['--output', 'history.gitlog']), {
    action: 'run',
    port: 0,
    open: true,
    output: 'history.gitlog',
  });
  assert.equal(parseOptions(['-h']).action, 'help');
  assert.equal(parseOptions(['--version']).action, 'version');
});

test('mistakes are usage errors', () => {
  for (const argv of [
    ['--port', 'abc'],
    ['--port', '0'],
    ['--port', '70000'],
    ['--port', '80.5'],
    ['--output', ''],
    ['--output'],
    ['--frobnicate'],
    ['some/folder'],
  ]) {
    assert.throws(() => parseOptions(argv), UsageError, argv.join(' '));
  }
});

test('usage errors are in the command\'s words, not Node\'s', () => {
  /** @param {string[]} argv */
  const message = (argv) => {
    try {
      parseOptions(argv);
    } catch (err) {
      return err instanceof Error ? err.message : '';
    }
    return '';
  };
  assert.equal(message(['--frobnicate']), 'Unknown option "--frobnicate".');
  assert.equal(message(['--port 80']), 'Unknown option "--port 80".');
  assert.equal(message(['-x']), 'Unknown option "-x".');
  assert.equal(message(['--port']), '--port needs a number from 1 to 65535, such as --port 8080.');
  assert.equal(message(['--output']), '--output needs a file name, such as --output history.gitlog.');
  assert.equal(message(['--help=1']), '--help does not take a value.');
  assert.equal(message(['--no-open=yes']), '--no-open does not take a value.');
  assert.equal(message(['--port', 'abc']), '--port must be a number from 1 to 65535, not "abc".');
  for (const argv of [['--frobnicate'], ['--port'], ['dir'], ['--help=1']]) {
    // Printed as `gource-online: <first line>`, then the rest as it is.
    for (const line of `gource-online: ${message(argv)}`.split('\n')) assert.ok(line.length <= 80, line);
  }
});
