// SPDX-License-Identifier: GPL-3.0-or-later

// The player's release policy against real HTTP origins, with no route interception. Links to other origins are refused
// before any fetch (player.spec.ts); this holds behind that check, in the page and in its parser worker.

import { expect, test, type Page } from '@playwright/test';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { startOrigins, type Origins, type Request } from './servers';

const WASM = fileURLToPath(new URL('../../../packages/viewer/src/wasm/gource_web_bg.wasm', import.meta.url));

let o: Origins;
let requests: Request[];
let stop: () => Promise<void>;
test.beforeAll(async () => ({ origins: o, requests, stop } = await startOrigins()));
test.afterAll(async () => stop());
test.skip(!existsSync(WASM), 'needs scripts/build-web.sh');

const reads = (path: string) => requests.filter((r) => r.path === path).length;

/** Starts an element on `src` directly, past the player's link check, and waits for how it ends. */
const elementResult = (page: Page, src: string) => page.evaluate((url) => new Promise<string>((resolve) => {
  const el = document.createElement('gource-viewer');
  el.addEventListener('gource-done', () => resolve('done'));
  el.addEventListener('gource-error', (e) => resolve((e as CustomEvent<{ code: string }>).detail.code));
  el.setAttribute('src', url);
  document.body.append(el);
}), src);

test('the page and its parser worker read logs from the allowed origins only', async ({ page }) => {
  await page.goto(`${o.player}/`);

  // The control: an allowed origin is read, by the worker.
  expect(await elementResult(page, `${o.logs}/csp-allowed.log`)).toBe('done');
  expect(reads('logs:/csp-allowed.log')).toBe(1);

  expect(await elementResult(page, `${o.hostile}/csp-worker.log`)).toBe('log_failed');
  expect(reads('hostile:/csp-worker.log')).toBe(0);

  const fetched = await page.evaluate((url) => fetch(url).then(() => 'read', () => 'refused'), `${o.hostile}/csp-page.log`);
  expect(fetched).toBe('refused');
  expect(reads('hostile:/csp-page.log')).toBe(0);
});
