// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

import assert from 'node:assert/strict';
import { test } from 'node:test';

import { isRemoteShell, openCommand } from '../src/open.js';

const URL = 'http://127.0.0.1:5000/#src=%2Fabc%2Fa%2526b.gitlog&title=a%26b';

test('each platform opens the link as one argument, without a shell', () => {
  assert.deepEqual(openCommand(URL, 'darwin', {}), ['open', [URL]]);
  assert.deepEqual(openCommand(URL, 'win32', {}), ['rundll32', ['url.dll,FileProtocolHandler', URL]]);
  assert.deepEqual(openCommand(URL, 'linux', {}), ['xdg-open', [URL]]);
  assert.deepEqual(openCommand(URL, 'linux', { WSL_DISTRO_NAME: 'Ubuntu' }), [
    'rundll32.exe',
    ['url.dll,FileProtocolHandler', URL],
  ]);
  assert.equal(openCommand(URL, 'aix', {}), null);
});

test('SSH sessions are recognised', () => {
  assert.equal(isRemoteShell({}), false);
  assert.equal(isRemoteShell({ SSH_TTY: '/dev/pts/1' }), true);
  assert.equal(isRemoteShell({ SSH_CONNECTION: '1 2 3 4' }), true);
});
