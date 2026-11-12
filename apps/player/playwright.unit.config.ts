// SPDX-License-Identifier: GPL-3.0-or-later

import { defineConfig } from '@playwright/test';

// Unit tests of pure modules: Playwright's runner only transpiles and runs them in Node.
export default defineConfig({
  testDir: 'tests/unit',
  testMatch: /.*\.test\.ts$/,
  fullyParallel: true,
  reporter: [['list']],
});
