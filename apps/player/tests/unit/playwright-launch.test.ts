// SPDX-License-Identifier: GPL-3.0-or-later

import { expect, test } from '@playwright/test';
import config, { launchArguments } from '../../playwright.config';

const softwareArgs = ['--enable-unsafe-swiftshader', '--use-angle=swiftshader'];

test('the configured launch honors the platform and explicit real/CLI flags', () => {
  const softwareWebgl = process.platform !== 'darwin'
    || process.env.E2E_REAL === '1'
    || process.env.E2E_CLI === '1';
  expect(config.use?.launchOptions?.args).toEqual(softwareWebgl ? softwareArgs : []);
});

for (const platform of ['darwin', 'linux', 'win32'] as const) {
  test(`${platform}: explicit real/CLI software WebGL remains available`, () => {
    expect(launchArguments(platform, true)).toEqual(softwareArgs);
  });
  test(`${platform}: default launch preserves the platform backend choice`, () => {
    expect(launchArguments(platform, false)).toEqual(platform === 'darwin' ? [] : softwareArgs);
  });
}
