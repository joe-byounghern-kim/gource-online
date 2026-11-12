// SPDX-License-Identifier: GPL-3.0-or-later

import { expect, test } from '@playwright/test';

import { REPO_URL, SOURCE_REF, sourceUrl } from '../../src/build-info';

test('without a build there is a usable default', () => {
  expect(SOURCE_REF).toBe('main');
  expect(REPO_URL).toBe('https://github.com/joe-byounghern-kim/gource-online');
  expect(sourceUrl()).toBe('https://github.com/joe-byounghern-kim/gource-online/tree/main');
});

test('tags and commits make exact links', () => {
  expect(sourceUrl('v0.2.0-beta.1', 'https://github.com/o/r')).toBe('https://github.com/o/r/tree/v0.2.0-beta.1');
  expect(sourceUrl('feat/x y', 'https://github.com/o/r')).toBe('https://github.com/o/r/tree/feat/x%20y');
});

test('an empty repository URL means no source link yet', () => {
  expect(sourceUrl('main', '')).toBeNull();
});
