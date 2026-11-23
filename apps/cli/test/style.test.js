// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { style, useColor } from '../src/style.js';

test('colour only in a terminal, and never against NO_COLOR, TERM=dumb or FORCE_COLOR=0', () => {
  const tty = { isTTY: true };
  const pipe = { isTTY: false };
  assert.equal(useColor(tty, {}), true);
  assert.equal(useColor(pipe, {}), false);
  assert.equal(useColor({}, {}), false);
  assert.equal(useColor(tty, { NO_COLOR: '1' }), false);
  assert.equal(useColor(tty, { NO_COLOR: '' }), true);
  assert.equal(useColor(tty, { NODE_DISABLE_COLORS: '1' }), false);
  assert.equal(useColor(tty, { TERM: 'dumb' }), false);
  assert.equal(useColor(tty, { FORCE_COLOR: '0' }), false);
  assert.equal(useColor(tty, { FORCE_COLOR: 'false' }), false);
  assert.equal(useColor(tty, { FORCE_COLOR: '1' }), true);
  // Piped output stays plain even when a CI service or a test runner sets FORCE_COLOR.
  assert.equal(useColor(pipe, { FORCE_COLOR: '1' }), false);
  assert.equal(useColor(pipe, { FORCE_COLOR: '' }), false);
});

test('plain style changes nothing; the coloured one wraps and resets', () => {
  const plain = style(false);
  for (const fn of Object.values(plain)) assert.equal(fn('text'), 'text');
  const color = style(true);
  assert.equal(color.bold('a'), '\x1b[1ma\x1b[22m');
  assert.equal(color.dim('a'), '\x1b[2ma\x1b[22m');
  assert.equal(color.red('a'), '\x1b[31ma\x1b[39m');
});
