// SPDX-License-Identifier: GPL-3.0-or-later

import { defineConfig } from '@playwright/test';

// Browser tests of the player. Most run on the Vite dev server with the placeholder engine (?mock=1). The specs that
// use tests/servers.ts build the player themselves and serve it on real HTTP origins. E2E_REAL=1 with E2E_BASE_URL
// runs the real-engine specs against a test build (scripts/serve.sh); E2E_CLI=1 runs the packed npm package.
// E2E_CHANNEL picks the browser: `chrome` (default) or `chromium`, Playwright's bundled build. CI uses chromium: there
// is no Google Chrome for Linux arm64.
const PORT = Number(process.env.E2E_PORT ?? 5199);
const real = process.env.E2E_REAL === '1';
const cli = process.env.E2E_CLI === '1';
const baseURL = process.env.E2E_BASE_URL ?? `http://127.0.0.1:${PORT}`;

/** The log sources the dev server allows, for the specs that hold a log open: their own servers listen here. */
export const HELD_LOG_PORT = PORT - 1;
export const DROPPED_LOG_PORT = PORT - 2;

/** Software WebGL is needed on CI and by the explicit GPU suites, not by the default macOS suite: forcing it there
 * stalls even Canvas2D frames under load, including Playwright's stability checks. */
export function launchArguments(platform: NodeJS.Platform, softwareWebgl: boolean): string[] {
  return platform !== 'darwin' || softwareWebgl
    ? ['--enable-unsafe-swiftshader', '--use-angle=swiftshader']
    : [];
}

/** Specs whose pages run the real engine in the default suite. Two SwiftShader pages rendering at once made WebGL2
 * context creation alone take seconds on CI (#58), so at most one runs at a time, and CI runs this project only after
 * the default project (.github/workflows/ci.yml). */
export const REAL_WEBGL = /(?:^|\/)(?:player-real|log-csp|release-build)\.spec\.ts$/;

export default defineConfig({
  testDir: 'tests',
  testMatch: /.*\.spec\.ts$/,
  timeout: real ? 15 * 60_000 : 30_000,
  expect: { timeout: real ? 60_000 : 5_000 },
  fullyParallel: true,
  forbidOnly: Boolean(process.env.CI),
  retries: 0,
  reporter: [['list']],
  use: {
    baseURL,
    channel: process.env.E2E_CHANNEL || 'chrome',
    headless: true,
    viewport: { width: 1280, height: 800 },
    launchOptions: { args: launchArguments(process.platform, real || cli) },
    trace: 'retain-on-failure',
  },
  projects: [
    { name: 'real-webgl', testMatch: REAL_WEBGL, workers: 1 },
    { name: 'default', testIgnore: REAL_WEBGL },
  ],
  webServer: process.env.E2E_BASE_URL || cli
    ? undefined
    : {
        command: `pnpm exec vite --host 127.0.0.1 --port ${PORT} --strictPort`,
        // The recording's and the engine's test hooks (src/record/hooks.ts, src/test-hooks.ts).
        env: { VITE_E2E_HOOKS: '1', GOURCE_PLAYER_SOURCES: `http://127.0.0.1:${HELD_LOG_PORT} http://127.0.0.1:${DROPPED_LOG_PORT}` },
        url: baseURL,
        reuseExistingServer: !process.env.CI,
        timeout: 60_000,
      },
});
