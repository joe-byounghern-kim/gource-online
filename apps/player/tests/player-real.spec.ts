// SPDX-License-Identifier: GPL-3.0-or-later

// The real wasm engine in the built player, under its CSP: a log handed over by
// postMessage (the Blob path) and one by URL. It runs in the `real-webgl`
// project, one page at a time and after the others (playwright.config.ts), as
// the other real-engine specs do (#58).

import { expect, test, type Page } from '@playwright/test';
import { existsSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { embed, expectState, fragment, gotTypes, handOver, startOrigins, type Origins } from './servers';

const WASM = fileURLToPath(new URL('../../../packages/viewer/src/wasm/gource_web_bg.wasm', import.meta.url));

let o: Origins;
let stop: () => Promise<void>;
test.beforeAll(async () => ({ origins: o, stop } = await startOrigins()));
test.afterAll(async () => stop());
test.skip(!existsSync(WASM), 'needs scripts/build-web.sh');

test('the engine plays a log handed over by postMessage, and one by URL, with no console error', async ({ page }) => {
  const errors: string[] = [];
  page.on('console', (message) => { if (message.type() === 'error') errors.push(message.text()); });
  await page.goto(`${o.sender}/page.html`);
  await embed(page, `${o.player}/`);
  await expect.poll(() => gotTypes(page)).toEqual(['ready']);
  await handOver(page, o.player);
  await expect.poll(() => gotTypes(page)).toEqual(['ready', 'ack']);
  const frame = page.frameLocator('iframe.player');
  await expect(frame.locator('main.player')).toHaveAttribute('data-state', 'playing', { timeout: 20_000 });
  await expect(frame.locator('[data-stat="commits"]')).toContainText('/ 3 commits', { timeout: 20_000 });

  await page.goto(`${o.player}/${fragment({ src: `${o.logs}/real.log` })}`);
  await expectState(page, 'playing');
  await expect(page.locator('[data-stat="commits"]')).toContainText('/ 3 commits', { timeout: 20_000 });
  expect(errors).toEqual([]);
});

/** Waits for the whole log, in fixed-step mode (`?fixed-dt`, a test hook). */
async function openFixed(page: Page, fragmentText: string) {
  await page.goto(`${o.player}/?fixed-dt=0.0166667${fragmentText}`);
  await expect(page.locator('main[data-screen="viewer"]')).toBeVisible({ timeout: 30_000 });
  await expect.poll(() => page.evaluate(() => window.__gource!.status().seekable), { timeout: 30_000 }).toBe(true);
}

test('the engine draws the log within the expected draw calls', async ({ page }) => {
  await openFixed(page, fragment({ src: `${o.logs}/draw.log` }));
  const drawn = await page.evaluate(() => {
    window.__gource!.step(120);
    const { files, draw_calls: draws } = window.__gource!.status();
    return { mock: Boolean(window.__gourceMock), files, draws };
  });
  expect(drawn.mock).toBe(false);
  expect(drawn.files).toBe(2);
  expect(drawn.draws).toBeGreaterThan(0);
  expect(drawn.draws).toBeLessThanOrEqual(12);
});

test('the engine replays the log when the displayed-file cap is lowered and raised', async ({ page }) => {
  const log = ['one', 'two', 'three'].map((name, index) => `${1704067200 + index * 3600}|Self|A|/${name}/main.rs\n`).join('');
  await page.route('**/same-capped.log', (route) => route.fulfill({ contentType: 'text/plain', body: log }));
  await openFixed(page, fragment({ src: '/same-capped.log', 'o.max-files': '2' }));
  const result = await page.evaluate(() => {
    const engine = window.__gource!;
    engine.step(240);
    const initial = engine.status().files;
    const lowerEffect = engine.app.set_option('max-files', '1');
    engine.app.restart(); engine.step(240);
    const lowered = engine.status().files;
    const raiseEffect = engine.app.set_option('max-files', '3');
    engine.app.restart(); engine.step(240);
    return { initial, lowerEffect, lowered, raiseEffect, raised: engine.status().files };
  });
  expect(result).toEqual({ initial: 2, lowerEffect: 'restart', lowered: 1, raiseEffect: 'restart', raised: 3 });
});

test('the engine and the browser encoder record a log to an MP4', async ({ page }) => {
  test.setTimeout(120_000);
  await openFixed(page, fragment({ src: '/samples/ai-coauthors.log' }));
  await expect(page.locator('.record-btn')).toHaveAttribute('aria-disabled', 'false');
  await page.locator('.record-btn').click();
  const dialog = page.getByRole('dialog', { name: 'Record video' });
  await expect(dialog.locator('.rec-encoder')).toContainText('H.264', { timeout: 30_000 });
  await dialog.locator('input[name="rec-size"][value="1280x720"]').check();
  await dialog.locator('input[name="rec-fps"][value="30"]').check();
  await dialog.locator('#rec-in-max').fill('1');
  await page.evaluate(() => { window.__recordTest!.sink = 'blob'; });
  await page.locator('.rec-start').click();
  await page.waitForFunction(() => ['done', 'failed', 'cancelled'].includes(window.__record?.state() ?? ''), undefined, { timeout: 90_000 });
  const recorded = await page.evaluate(async () => {
    const data = new Uint8Array(await window.__record!.output()!.arrayBuffer());
    const view = new DataView(data.buffer);
    const boxes: string[] = [];
    for (let offset = 0; offset + 8 <= data.length;) {
      const size = view.getUint32(offset);
      boxes.push(String.fromCharCode(...data.slice(offset + 4, offset + 8)));
      if (size < 8) break;
      offset += size;
    }
    return { state: window.__record!.state(), error: window.__record!.error(), frames: window.__record!.frames(), bytes: data.length, boxes };
  });
  expect(recorded.error).toBeNull();
  expect(recorded.state).toBe('done');
  expect(recorded.frames).toBe(30);
  expect(recorded.bytes).toBeGreaterThan(1000);
  expect(recorded.boxes).toEqual(expect.arrayContaining(['ftyp', 'moov', 'mdat']));
});

test('a browser without WebGL2 is told the visualisation could not start', async ({ page }) => {
  await page.addInitScript(() => {
    const original = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function (this: HTMLCanvasElement, type: string, ...rest: unknown[]) {
      if (type === 'webgl2') return null;
      return (original as (...a: unknown[]) => unknown).call(this, type, ...rest);
    } as typeof original;
  });
  await page.goto(`${o.player}/${fragment({ src: `${o.logs}/no-webgl.log` })}`);
  await expectState(page, 'failed');
  await expect(page.locator('.player-banner')).toContainText('could not start');
  await expect(page.locator('.player-file-input')).toBeAttached();
});
