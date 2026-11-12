// SPDX-License-Identifier: MIT

import { expect, test } from '@playwright/test';

import { LOG_ARGS, PRETTY, PRETTY_AI, shellLogCommand } from '../src/git-log.js';

test('the attribution format extends Gource\'s own', () => {
  expect(PRETTY).toBe('--pretty=format:user:%aN%n%ct');
  expect(PRETTY_AI.startsWith(`${PRETTY}%nm1:`)).toBe(true);
  expect(PRETTY_AI.endsWith('%x1e%s')).toBe(true);
});

test('the argument list is Gource\'s git command with UTF-8 paths unquoted', () => {
  expect(LOG_ARGS).toEqual([
    '-c', 'core.quotepath=off', 'log', '--reverse', '--raw', '--encoding=UTF-8',
    '--no-renames', '--no-show-signature', PRETTY_AI,
  ]);
});

test('the shell command writes the same output to a file', () => {
  expect(shellLogCommand('history.gitlog')).toBe(
    'git -c core.quotepath=off log --reverse --raw --encoding=UTF-8 --no-renames --no-show-signature ' +
      '--pretty=format:"user:%aN%n%ct%nm1:%an%x1f%ae%x1f%cn%x1f%ce%x1f%(trailers:key=Co-authored-by,key=Made-with,unfold,separator=%x1f)%x1e%s" ' +
      '--output=history.gitlog',
  );
});
