// SPDX-License-Identifier: MIT

import { defineConfig } from '@playwright/test';

// Unit tests of the pure modules: Playwright's runner only transpiles and
// runs them in Node (as apps/player does for its unit tests).
export default defineConfig({
  testDir: 'test',
  testMatch: /.*\.test\.ts$/,
  fullyParallel: true,
  reporter: [['list']],
});
