// SPDX-License-Identifier: GPL-3.0-or-later

// The viewer screen on the real engine: what the controls do to the frame and the simulation, and WebGL context loss.
// Runs with E2E_REAL=1 against a test build (apps/player/README.md).

import { expect, test, type Page } from '@playwright/test';
import type { EngineStatus } from '@gource-online/viewer';
import { playing } from './mock-engine';

test.skip(process.env.E2E_REAL !== '1', 'set E2E_REAL=1 with a test build served by scripts/serve.sh');

const OUT = 'test-results/real';
const LOG = '/samples/ai-coauthors.log';
const FIXED = 'fixed-dt=0.0166667';

const status = (page: Page) => page.evaluate(() => window.__gource?.status() ?? null) as Promise<EngineStatus | null>;

function collectErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on('console', (m) => {
    if (m.type() === 'error') errors.push(m.text());
  });
  page.on('pageerror', (e) => errors.push(e.message));
  return errors;
}

/** Opens the viewer on `query`, waits for the whole log and a live WebGL context. */
async function openViewer(page: Page): Promise<void> {
  await page.goto('about:blank');
  await page.goto(playing(LOG, {}, FIXED));
  await expect(page.locator('main[data-screen="viewer"]')).toBeVisible({ timeout: 60_000 });
  await expect.poll(async () => (await status(page))?.seekable, { timeout: 60_000 }).toBe(true);
}

const step = (page: Page, ticks: number) => page.evaluate((n) => window.__gource!.step(n), ticks);
const option = (page: Page, name: string) => page.evaluate((n) => window.__gource?.app.get_option(n), name);

/** Hash of the canvas pixels after rendering the current state without advancing it. */
async function frameHash(page: Page): Promise<number> {
  return page.evaluate(async () => {
    const canvas = document.querySelector<HTMLCanvasElement>('.gource-canvas')!;
    window.__gource!.step(0);
    const img = new Image();
    img.src = canvas.toDataURL();
    await img.decode();
    const c = document.createElement('canvas');
    c.width = img.width;
    c.height = img.height;
    const ctx = c.getContext('2d')!;
    ctx.drawImage(img, 0, 0);
    const px = ctx.getImageData(0, 0, c.width, c.height).data;
    let hash = 0;
    for (let i = 0; i < px.length; i += 4) hash = (Math.imul(hash, 31) + px[i]! + px[i + 1]! * 3 + px[i + 2]! * 7) | 0;
    return hash;
  });
}

async function openPanel(page: Page): Promise<void> {
  await page.getByRole('button', { name: 'Settings' }).click();
  for (const s of ['look', 'playback', 'visibility', 'filters']) {
    await page.locator(`details[data-section="${s}"]`).evaluate((d) => ((d as HTMLDetailsElement).open = true));
  }
}


test('a log by link plays with no server behind it', async ({ page }) => {
  let api = 0;
  page.on('request', (r) => {
    if (new URL(r.url()).pathname.startsWith('/api/')) api++;
  });
  await page.goto(playing('/samples/file-removal.log', {}, ''));
  await expect(page.locator('main[data-screen="viewer"]')).toBeVisible({ timeout: 60_000 });
  await expect.poll(async () => (await status(page))?.commits_loaded ?? 0).toBeGreaterThan(0);
  await expect.poll(async () => (await status(page))?.seekable).toBe(true);
  await expect.poll(async () => (await status(page))?.fps ?? 0).toBeGreaterThan(0);
  await page.screenshot({ path: `${OUT}/local-log-viewer.png` });
  expect(api).toBe(0);
});

test('fixed-dt mode advances only through step()', async ({ page }) => {
  await page.goto(playing('/samples/file-removal.log', {}, FIXED));
  await expect(page.locator('main[data-screen="viewer"]')).toBeVisible({ timeout: 60_000 });
  await expect.poll(async () => (await status(page))?.commits_loaded ?? 0).toBeGreaterThan(0);
  await page.waitForTimeout(500);
  expect((await status(page))!.ticks).toBe(0);
  await page.evaluate(() => window.__gource!.step(31));
  const s = (await status(page))!;
  expect(s.ticks).toBe(31);
  expect(s.fixed_dt).toBeCloseTo(1 / 60, 5);
});

test('viewer controls visibly change the frame and the simulation', async ({ page }) => {
  const errors = collectErrors(page);
  await openViewer(page);
  await step(page, 600);

  // Pause: the button and the space key; a paused simulation does not advance.
  await page.locator('.play-btn').click();
  expect((await status(page))!.paused).toBe(true);
  const t = (await status(page))!.time;
  await step(page, 60);
  expect((await status(page))!.time).toBe(t);
  await page.locator('.gource-canvas').focus();
  await page.keyboard.press(' ');
  expect((await status(page))!.paused).toBe(false);

  // Seek: hovering shows the date there, clicking jumps.
  const seek = await page.locator('.seek').boundingBox();
  await page.mouse.move(seek!.x + seek!.width / 2, seek!.y + seek!.height / 2);
  await expect(page.locator('.seek-tip')).toHaveText(await page.evaluate(() => window.__gource!.app.date_at(0.5)));
  await page.mouse.click(seek!.x + seek!.width / 2, seek!.y + seek!.height / 2);
  await step(page, 1);
  expect((await status(page))!.fraction).toBeGreaterThan(0.45);
  await page.evaluate(() => window.__gource!.app.seek(0.02));
  await step(page, 1);

  // Speed: 60 ticks are one second, i.e. one day at 1 s/day.
  await page.selectOption('.speed', '1');
  const t0 = (await status(page))!.time;
  await step(page, 60);
  expect((await status(page))!.time - t0).toBe(86_400);
  await expect(page).toHaveURL(/o\.seconds-per-day=1(&|$)/);

  // Gource's keys change settings in the engine; the page follows.
  await page.locator('.gource-canvas').focus();
  await page.keyboard.press('+');
  await expect(page.locator('.speed')).toHaveValue('0.5');
  await page.keyboard.press('v');
  await expect(page).toHaveURL(/o\.camera-mode=track/);
  await page.keyboard.press('v');
  await page.selectOption('.speed', '10');
  await step(page, 300);

  // Wheel zoom and a right drag (rotation) move the camera.
  let before = await frameHash(page);
  const c = await page.locator('.gource-canvas').boundingBox();
  await page.mouse.move(c!.x + c!.width / 2, c!.y + c!.height / 2);
  for (let i = 0; i < 4; i++) await page.mouse.wheel(0, -100);
  await step(page, 30);
  expect(await frameHash(page)).not.toBe(before);
  before = await frameHash(page);
  await page.mouse.down({ button: 'right' });
  for (let i = 1; i <= 8; i++) await page.mouse.move(c!.x + c!.width / 2 + i * 20, c!.y + c!.height / 2);
  await page.mouse.up({ button: 'right' });
  await step(page, 1);
  expect(await frameHash(page)).not.toBe(before);

  // Every hide toggle and the look controls change the rendered frame.
  await openPanel(page);
  const changes = async (apply: () => Promise<unknown>, revert: () => Promise<unknown>) => {
    const a = await frameHash(page);
    await apply();
    await page.waitForTimeout(500);
    const b = await frameHash(page);
    await revert();
    await page.waitForTimeout(500);
    return a !== b;
  };
  for (const element of ['date', 'users', 'tree', 'files', 'dirnames', 'bloom', 'root']) {
    const chip = page.locator(`[data-hide="${element}"]`);
    const click = () => chip.evaluate((e) => (e as HTMLInputElement).click());
    expect(await changes(click, click), `hide ${element}`).toBe(true);
  }
  for (const [name, value, reset] of [
    ['background-colour', '#ff0000', '#1a1a1a'],
    ['font-colour', '#00ff00', '#ffffff'],
  ] as const) {
    const input = page.locator(`[data-option="${name}"]`);
    expect(await changes(() => input.fill(value), () => input.fill(reset)), name).toBe(true);
  }
  for (const [name, pos] of [['bloom-intensity', '2.5'], ['bloom-multiplier', '2.5'], ['user-scale', '3'], ['font-scale', '2']] as const) {
    const slider = page.locator(`[data-option="${name}"]`);
    const set = (v: string) => slider.evaluate((el, v) => {
      (el as HTMLInputElement).value = v;
      el.dispatchEvent(new Event('input', { bubbles: true }));
    }, v);
    const initial = await slider.inputValue();
    expect(await changes(() => set(pos), () => set(initial)), name).toBe(true);
  }
  const seed = page.locator('[data-option="hash-seed"]');
  expect(await changes(() => seed.fill('1234'), () => seed.fill('31')), 'hash-seed').toBe(true);

  // hide=progress hides the seek bar (Gource's slider).
  const progress = page.locator('[data-hide="progress"]');
  await progress.evaluate((e) => (e as HTMLInputElement).click());
  await expect(page.locator('.seek-wrap')).toHaveCSS('visibility', 'hidden');
  await progress.evaluate((e) => (e as HTMLInputElement).click());
  await expect(page.locator('.seek-wrap')).toHaveCSS('visibility', 'visible');

  // Every slider bound is accepted by the engine.
  await page.evaluate(() => {
    for (const el of document.querySelectorAll<HTMLInputElement>('input.range-input')) {
      for (const v of [el.min, el.max]) {
        el.value = v;
        el.dispatchEvent(new Event('input', { bubbles: true }));
      }
    }
  });
  await page.waitForTimeout(400);
  expect(await page.locator('.control.is-invalid').count()).toBe(0);
  // Start over with default settings for the filter checks.
  await openViewer(page);

  // Filters re-simulate with fewer files; an invalid regex is reported in place.
  await page.evaluate(() => window.__gource!.app.set_option('seconds-per-day', '0.5'));
  await page.evaluate(() => window.__gource!.app.restart());
  await step(page, 1200);
  const all = (await status(page))!.files;
  await openPanel(page);
  await page.locator('[data-option="file-filter"]').fill('\\.(ts|tsx)$');
  await page.locator('.apply-filters').click();
  await expect(page.locator('.apply-status')).toHaveText('Applied: simulation restarted');
  await step(page, 1200);
  expect((await status(page))!.files).toBeLessThan(all);
  await page.locator('[data-option="file-filter"]').fill('(unclosed');
  await page.locator('.apply-filters').click();
  await expect(page.locator('[data-control="file-filter"] .control-error')).toContainText('regular expression');

  // The URL reproduces the view after a reload.
  await page.locator('[data-option="file-filter"]').fill('\\.svg$');
  await page.locator('.apply-filters').click();
  await page.locator('[data-option="title"]').fill('Round trip');
  await page.waitForTimeout(600);
  await page.reload();
  await expect.poll(() => option(page, 'title'), { timeout: 60_000 }).toBe('Round trip');
  expect(await option(page, 'file-filter')).toBe('\\.svg$');
  expect(errors).toEqual([]);
});

test('HiDPI drawable and WebGL context loss recovery', async ({ browser }) => {
  const context = await browser.newContext({ viewport: { width: 900, height: 700 }, deviceScaleFactor: 2 });
  const page = await context.newPage();
  const errors = collectErrors(page);
  await openViewer(page);
  await step(page, 300);
  let s = (await status(page))!;
  expect([s.width, s.height]).toEqual([1800, 1400]);
  await page.setViewportSize({ width: 600, height: 500 });
  await expect.poll(async () => (await status(page))!.width).toBe(1200);

  const result = await page.evaluate(async () => {
    const canvas = document.querySelector<HTMLCanvasElement>('.gource-canvas')!;
    const ext = canvas.getContext('webgl2')!.getExtension('WEBGL_lose_context')!;
    ext.loseContext();
    await new Promise((r) => setTimeout(r, 300));
    window.__gource!.step(2);
    const during = window.__gource!.status().draw_calls;
    ext.restoreContext();
    await new Promise((r) => setTimeout(r, 500));
    window.__gource!.step(2);
    return { during, after: window.__gource!.status().draw_calls };
  });
  expect(result.during).toBe(0);
  expect(result.after).toBeGreaterThan(0);
  await expect(page.locator('.viewer-banner')).toBeHidden();
  s = (await status(page))!;
  expect(s.memory_bytes).toBeGreaterThan(0);
  expect(errors).toEqual([]);
  await context.close();
});
