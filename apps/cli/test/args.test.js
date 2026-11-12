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
