// SPDX-License-Identifier: GPL-3.0-or-later

// F3 recording in the mock suite (REC-93): `?mock=1&mock-record=1` runs the
// dialog, the export loop, cancel, errors and the finished screen with the
// mock recorder, a fake encoder, the byte-counting muxer and the in-memory
// sink. Needs the test hooks (VITE_E2E_HOOKS=1, set by playwright.config.ts).

import { readFileSync } from 'node:fs';
import { createServer, type ServerResponse } from 'node:http';

import { expect, test, type Page } from '@playwright/test';

import { HELD_LOG_PORT } from '../playwright.config';
import type { RecordTestConfig } from '../src/record/hooks';
import { SMALL_LOG, engineCalls, playing } from './mock-engine';

const LOG = '/samples/ai-coauthors.log';
const RECORDING = 'mock=1&mock-record=1';

async function openViewer(page: Page): Promise<void> {
  await page.goto(playing(LOG, {}, RECORDING));
  await expect(page.locator('[data-screen="viewer"]')).toBeVisible({ timeout: 15_000 });
  await expect(page.locator('.record-btn')).toHaveAttribute('aria-disabled', 'false', { timeout: 15_000 });
}

async function setTest(page: Page, config: RecordTestConfig): Promise<void> {
  await page.evaluate((c) => Object.assign((window.__recordTest ??= {}), c), config);
}

/** Opens the dialog and picks a small, short recording. */
async function openDialog(page: Page, opts: { size?: string; fps?: string; max?: string } = {}) {
  await page.locator('.record-btn').click();
  await expect(page.getByRole('dialog', { name: 'Record video' })).toBeVisible();
  // The accessible name follows the screen ("Recording", "Saved"), so keep a stable locator.
  const dialog = page.locator('.rec-dialog');
  await dialog.locator(`input[name="rec-size"][value="${opts.size ?? '1280x720'}"]`).check();
  await dialog.locator(`input[name="rec-fps"][value="${opts.fps ?? '30'}"]`).check();
  if (opts.max !== undefined) await dialog.locator('#rec-in-max').fill(opts.max);
  await expect(dialog.locator('.rec-encoder')).toContainText('H.264 High');
  return dialog;
}

const state = (page: Page) => page.evaluate(() => window.__record?.state());

async function downloadedJson(page: Page, click: () => Promise<void>): Promise<Record<string, any>> {
  const download = page.waitForEvent('download');
  await click();
  return JSON.parse(readFileSync(await (await download).path(), 'utf8')) as Record<string, any>;
}

test('T-1: Record waits for the full history and needs the real engine', async ({ page }) => {
  // Hold EOF instead of racing the fabricated log's 40-ms chunk schedule.
  // The real parser worker still receives and applies actual HTTP bytes.
  const split = Math.floor(SMALL_LOG.length / 2);
  let pending: ServerResponse | undefined;
  const log = createServer((_request, response) => {
    response.writeHead(200, { 'content-type': 'text/plain', 'access-control-allow-origin': '*' });
    response.write(SMALL_LOG.slice(0, split));
    pending = response;
  });
  await new Promise<void>((resolve, reject) => {
    log.once('error', reject);
    log.listen(HELD_LOG_PORT, '127.0.0.1', resolve);
  });
  try {
    await page.goto(playing(`http://127.0.0.1:${HELD_LOG_PORT}/held.log`, {}, RECORDING));
    await expect(page.locator('[data-screen="viewer"]')).toBeVisible({ timeout: 15_000 });
    const btn = page.locator('.record-btn');
    await expect(btn).toHaveAttribute('aria-disabled', 'true');
    await expect(btn).toHaveAttribute('title', 'Waiting for the full history');
    // locator.click() intentionally waits for aria-disabled to become false.
    // Here we need a genuine user attempt *while* it is disabled, not a
    // forced locator action or an untrusted dispatched DOM event.
    await btn.scrollIntoViewIfNeeded();
    await btn.evaluate((button) => button.addEventListener('click', (event) => {
      button.setAttribute('data-test-trusted-click', String(event.isTrusted));
    }, { once: true }));
    const box = await btn.boundingBox();
    if (!box) throw new Error('the recording button must be visible for the pointer attempt');
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    await expect(btn).toHaveAttribute('data-test-trusted-click', 'true');
    await expect(btn).toHaveAttribute('aria-disabled', 'true');
    await expect(page.getByRole('dialog')).toHaveCount(0);
    if (!pending) throw new Error('the partial log must be established before EOF is released');
    pending.end(SMALL_LOG.slice(split));
    await expect(btn).toHaveAttribute('aria-disabled', 'false', { timeout: 15_000 });
    await expect(btn).toHaveAttribute('title', 'Record video (Alt+R)');
    await btn.click();
    await expect(page.getByRole('dialog', { name: 'Record video' })).toBeVisible();

    // The mock engine alone (no test mode) cannot record.
    await page.goto(playing(LOG));
    await expect(page.locator('[data-screen="viewer"]')).toBeVisible();
    await expect(btn).toHaveAttribute('title', /needs the real engine/);
    await expect(btn).toHaveAttribute('aria-disabled', 'true');
  } finally {
    log.closeAllConnections();
    await new Promise<void>((resolve) => log.close(() => resolve()));
  }
});

test('T-1: no VideoEncoder disables Record by feature detection', async ({ page }) => {
  await page.addInitScript(() => {
    delete (window as unknown as Record<string, unknown>)['VideoEncoder'];
  });
  await page.goto(playing(LOG, {}, RECORDING));
  await expect(page.locator('[data-screen="viewer"]')).toBeVisible();
  await expect(page.locator('.record-btn')).toHaveAttribute('aria-disabled', 'true');
  await expect(page.locator('.record-btn')).toHaveAttribute('title', /cannot encode video here/);
});

test('T-24 and T-31: Alt+R, labels, focus trap, Esc, camera note; the engine never sees the key', async ({ page }) => {
  await openViewer(page);
  await page.locator('.gource-canvas').focus();
  await page.mouse.move(400, 400);
  await page.mouse.wheel(0, -300);
  await page.keyboard.press('Alt+r');
  const dialog = page.getByRole('dialog', { name: 'Record video' });
  await expect(dialog).toBeVisible();
  await expect(dialog).toHaveAttribute('aria-modal', 'true');
  const keys = (await engineCalls(page, 'key')).map((c) => String(c.args[0]).toLowerCase());
  expect(keys).not.toContain('r');
  await expect(dialog).toContainText('Camera: automatic. Your zoom and rotation are not recorded.');
  await expect(dialog).toContainText('can contain flashing');

  // Exercise each actual trap boundary instead of spending the test's
  // budget on 90 arbitrary browser round trips and checking only at the end.
  const first = dialog.getByRole('button', { name: 'Close', exact: true });
  const last = dialog.getByRole('button', { name: 'Record…', exact: true });
  await expect(last).toBeEnabled();
  await last.focus();
  await expect(last).toBeFocused();
  await page.keyboard.press('Tab');
  await expect(first).toBeFocused();
  expect(await page.evaluate(() => document.activeElement?.closest('.rec-dialog') !== null)).toBe(true);
  await page.keyboard.press('Shift+Tab');
  await expect(last).toBeFocused();
  expect(await page.evaluate(() => document.activeElement?.closest('.rec-dialog') !== null)).toBe(true);
  await dialog.focus();
  await expect(dialog).toBeFocused();
  await page.keyboard.press('Shift+Tab');
  await expect(last).toBeFocused();
  await dialog.focus();
  await page.keyboard.press('Tab');
  await expect(first).toBeFocused();

  // Every control has a label.
  const unlabelled = await dialog.locator('input:not([type=hidden]):not([type=file]), select').evaluateAll((els) =>
    els.filter((el) => !(el as HTMLInputElement).labels?.length && !el.getAttribute('aria-label')).map((el) => el.outerHTML),
  );
  expect(unlabelled).toEqual([]);

  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  // Focus returns to where it was (the canvas, where Alt+R was pressed).
  await expect(page.locator('.gource-canvas')).toBeFocused();
  await page.locator('.record-btn').click();
  await expect(dialog).toBeVisible();
  await page.getByRole('button', { name: 'Close' }).click();
  await expect(page.locator('.record-btn')).toBeFocused();
});

test('T-24, REC-20, REC-21: progress, live region, pause of the live view, cancel choice, stop and save', async ({ page }) => {
  await openViewer(page);
  await setTest(page, { sink: 'blob', encoderDelayMs: 4 });
  const dialog = await openDialog(page, { max: '60' });
  await dialog.locator('.rec-start').click();
  await expect(dialog.getByRole('progressbar')).toBeVisible();
  await expect.poll(() => state(page)).toBe('running');
  await expect(dialog.locator('.rec-live')).toContainText(/Recording/);

  // REC-20: banner, live view paused, seek and play locked, beforeunload asks.
  await expect(page.locator('.record-banner')).toBeVisible();
  await expect(page.locator('.play-btn')).toBeDisabled();
  await expect(page.locator('.seek')).toBeDisabled();
  // Dispatch the real panel event while its modal is open. A rebuild must not
  // reach either live or recording engine, even through programmatic input.
  await page.locator('[data-option="max-files"]').evaluate((element) => {
    const input = element as HTMLInputElement;
    input.value = '1'; input.dispatchEvent(new Event('input', { bubbles: true }));
  });
  await expect(page.getByText('Recording in progress: the live view is paused; this changes nothing in the video.', { exact: true })).toHaveCount(1);
  expect(await page.evaluate(() => window.__gource!.app.get_option('max-files'))).toBe('0');
  expect((await engineCalls(page, 'set_paused')).at(-1)?.args).toEqual([true]);
  const unload = await page.evaluate(() => {
    const e = new Event('beforeunload', { cancelable: true });
    window.dispatchEvent(e);
    return e.defaultPrevented;
  });
  expect(unload).toBe(true);

  await expect.poll(async () => Number(await dialog.getByRole('progressbar').getAttribute('aria-valuenow'))).toBeGreaterThan(0);
  await expect(dialog.locator('.rec-frames')).toContainText(/frame [\d,]+ of about [\d,]+/);

  // Esc opens the cancel choice, Esc again keeps recording.
  await dialog.focus();
  await page.keyboard.press('Escape');
  await expect(dialog.locator('.rec-cancel-choice')).toBeVisible();
  await expect(dialog.locator('.rec-cancel-choice')).toContainText(/is recorded/);
  await expect.poll(() => state(page)).toBe('paused');
  await page.keyboard.press('Escape');
  await expect(dialog.locator('.rec-cancel-choice')).toHaveCount(0);
  await expect.poll(() => state(page)).toBe('running');

  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await dialog.getByRole('button', { name: 'Stop and save' }).click();
  await expect.poll(() => state(page)).toBe('done');
  await expect(dialog).toContainText('Stopped early');
  await expect(dialog).toContainText('Nothing was uploaded');
  const out = await page.evaluate(() => ({ size: window.__record!.output()?.size ?? 0, frames: window.__record!.frames() }));
  expect(out.size).toBeGreaterThan(0);
  expect(out.frames).toBeGreaterThan(0);
  expect(out.frames).toBeLessThan(1800);
  // The live view is back as it was.
  await expect(page.locator('.record-banner')).toBeHidden();
  await expect(page.locator('.play-btn')).toBeEnabled();
  expect((await engineCalls(page, 'set_paused')).at(-1)?.args).toEqual([false]);
});

test('REC-22: Discard leaves nothing', async ({ page }) => {
  await openViewer(page);
  await setTest(page, { sink: 'blob', encoderDelayMs: 4 });
  const dialog = await openDialog(page, { max: '60' });
  await dialog.locator('.rec-start').click();
  await expect.poll(() => state(page)).toBe('running');
  await dialog.getByRole('button', { name: 'Cancel' }).click();
  await dialog.getByRole('button', { name: 'Discard' }).click();
  await expect.poll(() => state(page)).toBe('cancelled');
  await expect(dialog).toContainText('Nothing was saved');
  expect(await page.evaluate(() => window.__record!.output())).toBeNull();
});

test('a complete run ends by itself; the finished screen offers the recipe and no reproduce link', async ({ page }) => {
  await openViewer(page);
  await setTest(page, { sink: 'blob' });
  const dialog = await openDialog(page, { max: '4' });
  await dialog.locator('.rec-start').click();
  await expect.poll(() => state(page), { timeout: 15_000 }).toBe('done');
  expect(await page.evaluate(() => window.__record!.frames())).toBe(120);
  await expect(dialog).toContainText('maximum length');

  const recipe = await downloadedJson(page, () => dialog.getByRole('button', { name: 'Download recipe (.json)' }).click());
  expect(recipe['v']).toBe(1);
  expect(recipe['record']['record-framerate']).toBe('30');
  await expect(dialog.getByRole('button', { name: 'Copy reproduce link' })).toHaveCount(0);
});

test('T-17: a log by URL gets no title, the stem gource and a redacted sidecar', async ({ page }) => {
  await openViewer(page);
  await setTest(page, { sink: 'blob' });
  const dialog = await openDialog(page, { max: '2' });
  await expect(dialog.locator('#rec-in-title')).toHaveValue('');
  await expect(dialog.locator('#rec-sel-metadata')).toHaveValue('none');
  await dialog.locator('.rec-start').click();
  await expect.poll(() => state(page)).toBe('done');
  await expect(dialog.locator('.rec-saved-name')).toHaveText(/^gource_1280x720_30fps_/);
  let recipe = await downloadedJson(page, () => dialog.getByRole('button', { name: 'Download recipe (.json)' }).click());
  expect(recipe['source']['path']).toBe('');
  expect(recipe['source']['kind']).toBe('url');
  // Confirming includes the source.
  await dialog.getByLabel(/Include the source/).check();
  recipe = await downloadedJson(page, () => dialog.getByRole('button', { name: 'Download recipe (.json)' }).click());
  expect(recipe['source']['path']).toBe(new URL(LOG, page.url()).href);
});

test('T-28: an encoder error offers Stop and save, which keeps the frames so far', async ({ page }) => {
  await openViewer(page);
  await setTest(page, { sink: 'blob', encoderErrorAt: 30 });
  const dialog = await openDialog(page, { max: '30' });
  await dialog.locator('.rec-start').click();
  await expect(dialog.locator('.rec-error-box')).toHaveAttribute('data-code', 'record_encoder_failed');
  await expect(dialog.locator('.rec-error-box')).toContainText('EncodingError');
  expect(await state(page)).toBe('failed');
  expect(await page.evaluate(() => window.__record!.error()?.code)).toBe('record_encoder_failed');
  const partial = await page.evaluate(() => ({ timing: window.__record!.timing(), frames: window.__record!.frames() }));
  expect(partial.frames).toBeGreaterThan(0);
  expect(partial.timing.frames).toBe(partial.frames);
  await dialog.getByRole('button', { name: 'Stop and save' }).click();
  await expect.poll(() => state(page)).toBe('done');
  expect(await page.evaluate(() => window.__record!.frames())).toBe(30);
});

test('T-28: a reordering encoder gives record_mux_reorder', async ({ page }) => {
  await openViewer(page);
  await setTest(page, { sink: 'blob', reorderAt: 20 });
  const dialog = await openDialog(page, { max: '30' });
  await dialog.locator('.rec-start').click();
  await expect(dialog.locator('.rec-error-box')).toHaveAttribute('data-code', 'record_mux_reorder');
  expect(await state(page)).toBe('failed');
  await dialog.getByRole('button', { name: 'Discard' }).click();
  await expect.poll(() => state(page)).toBe('cancelled');
});

test('T-28: a full disk (QuotaExceededError) gives record_disk_full and an attempted close', async ({ page }) => {
  await openViewer(page);
  await page.evaluate(() => {
    const log: string[] = [];
    (window as unknown as { __sinkLog: string[] }).__sinkLog = log;
    let written = 0;
    window.__recordSink = async () => ({
      name: 'video.mp4',
      createWritable: async () => ({
        write: async (data: unknown) => {
          const n = data instanceof Uint8Array ? data.byteLength : 0;
          written += n;
          if (written > 4000) throw new DOMException('disk full', 'QuotaExceededError');
          log.push(`write ${n}`);
        },
        close: async () => {
          log.push('close');
        },
        abort: async () => {
          log.push('abort');
        },
      }),
    });
  });
  // The mock's small chunks reach the sink in one batch at the end, so the write fails while finishing.
  const dialog = await openDialog(page, { max: '10' });
  await dialog.locator('.rec-start').click();
  await expect(dialog.locator('.rec-error-box')).toHaveAttribute('data-code', 'record_disk_full', { timeout: 15_000 });
  await expect(dialog.locator('.rec-error-box')).toContainText('The disk is full');
  await expect(dialog.locator('.rec-error-box')).toContainText('What was written so far was kept');
  expect(await state(page)).toBe('failed');
  const log = await page.evaluate(() => (window as unknown as { __sinkLog: string[] }).__sinkLog);
  expect(log).toContain('close');
  // The live view is released.
  await expect(page.locator('.record-banner')).toBeHidden();
});

test('T-28: the in-memory cap stops and saves with a notice', async ({ page }) => {
  await openViewer(page);
  await setTest(page, { sink: 'blob', blobCap: 20_000 });
  const dialog = await openDialog(page, { max: '60' });
  await dialog.locator('.rec-start').click();
  await expect.poll(() => state(page)).toBe('done');
  await expect(dialog).toContainText('in-memory limit');
  const size = await page.evaluate(() => window.__record!.output()!.size);
  expect(size).toBeGreaterThanOrEqual(20_000);
  expect(size).toBeLessThan(2 << 20);
});

test('a File System Access sink gets the bytes and is closed', async ({ page }) => {
  await openViewer(page);
  await page.evaluate(() => {
    const st = { bytes: 0, closed: false, options: null as unknown };
    (window as unknown as { __fsa: typeof st }).__fsa = st;
    window.__recordSink = async (options) => {
      st.options = options;
      return {
        name: 'picked.mp4',
        createWritable: async () => ({
          write: async (data: unknown) => {
            if (data instanceof Uint8Array) st.bytes += data.byteLength;
          },
          close: async () => {
            st.closed = true;
          },
          abort: async () => undefined,
        }),
      };
    };
  });
  const dialog = await openDialog(page, { max: '2' });
  await dialog.locator('.rec-start').click();
  await expect.poll(() => state(page)).toBe('done');
  const st = await page.evaluate(() => (window as unknown as { __fsa: { bytes: number; closed: boolean; options: { suggestedName: string } } }).__fsa);
  expect(st.closed).toBe(true);
  expect(st.bytes).toBeGreaterThan(0);
  expect(st.options.suggestedName).toMatch(/^gource_1280x720_30fps_.*\.mp4$/);
  await expect(dialog).toContainText('picked.mp4');
  await expect(dialog).toContainText('where you chose');
});

test('T-10: the ladder note shows a downgrade, and an empty range blocks Record', async ({ page }) => {
  await openViewer(page);
  await page.evaluate(() => {
    window.__recordTest = { rejectConfig: (c) => c.hardwareAcceleration === 'no-preference' && c.width === 1280 };
  });
  await page.locator('.record-btn').click();
  const dialog = page.getByRole('dialog', { name: 'Record video' });
  await dialog.locator('input[name="rec-size"][value="1280x720"]').check();
  await expect(dialog.locator('.rec-encoder')).toContainText('software preferred');
  await expect(dialog.locator('.rec-encoder')).toContainText('using software encoding');

  await dialog.locator('input[name="rec-range"][value="dates"]').check();
  await dialog.locator('#rec-in-startDate').fill('2150-01-01');
  await expect(dialog.locator('.rec-estimate')).toContainText('No activity in this range');
  await expect(dialog.locator('.rec-start')).toBeDisabled();
});

test('dry run, PNG still, remembered values, presets', async ({ page }) => {
  await openViewer(page);
  let dialog = await openDialog(page, { size: '1080x1920', fps: '25' });
  await dialog.getByRole('button', { name: 'Calculate exact length' }).click();
  await expect(dialog.locator('.rec-estimate')).toContainText('Exactly');

  const png = page.waitForEvent('download');
  await dialog.getByRole('button', { name: 'Save frame as PNG' }).click();
  expect((await png).suggestedFilename()).toMatch(/^gource_1080x1920_25fps_.*\.png$/);

  await dialog.getByLabel('Preset name').fill('vertical');
  await dialog.getByRole('button', { name: 'Save preset' }).click();
  await expect(dialog.locator('.rec-preset-state')).toContainText('Saved "vertical"');

  // Record so the values are remembered.
  await setTest(page, { sink: 'blob' });
  await dialog.locator('#rec-in-max').fill('1');
  await dialog.locator('.rec-start').click();
  await expect.poll(() => state(page)).toBe('done');
  await dialog.locator('.rec-foot').getByRole('button', { name: 'Close' }).click();

  await openViewer(page);
  await page.locator('.record-btn').click();
  dialog = page.getByRole('dialog', { name: 'Record video' });
  await expect(dialog.locator('input[name="rec-size"][value="1080x1920"]')).toBeChecked();
  await expect(dialog.locator('input[name="rec-fps"][value="25"]')).toBeChecked();
  await dialog.locator('input[name="rec-size"][value="1280x720"]').check();
  await dialog.locator('#rec-presets').selectOption('vertical');
  await expect(dialog.locator('input[name="rec-size"][value="1080x1920"]')).toBeChecked();
  // The chosen preset can be deleted, and is gone from the list and from storage.
  await expect(dialog.locator('#rec-presets')).toHaveValue('vertical');
  await dialog.locator('.rec-presets').getByRole('button', { name: 'Delete' }).click();
  await expect(dialog.locator('#rec-presets option[value="vertical"]')).toHaveCount(0);
  expect(await page.evaluate(() => localStorage.getItem('gource.record.presets'))).not.toContain('vertical');
});

test('T-25: no Record on touch-first narrow screens; storage that throws leaves the dialog working', async ({ browser }) => {
  const touch = await browser.newContext({ hasTouch: true, isMobile: true, viewport: { width: 400, height: 800 } });
  const p = await touch.newPage();
  await p.goto(playing(LOG, {}, RECORDING));
  await expect(p.locator('[data-screen="viewer"]')).toBeVisible();
  await expect(p.locator('.record-btn')).toBeHidden();
  await touch.close();

  const ctx = await browser.newContext();
  const page = await ctx.newPage();
  await page.addInitScript(() => {
    Object.defineProperty(window, 'localStorage', {
      get() {
        throw new DOMException('denied', 'SecurityError');
      },
    });
  });
  await openViewer(page);
  const dialog = await openDialog(page);
  await expect(dialog.locator('input[name="rec-fps"][value="30"]')).toBeChecked();
  await dialog.getByLabel('Preset name').fill('x');
  await dialog.getByRole('button', { name: 'Save preset' }).click();
  await expect(dialog.locator('.rec-preset-state')).toContainText('does not allow');
  await ctx.close();
});

test('REC-3, REC-124: record=1 opens the prefilled dialog, never starts, and confirms link values', async ({ page }) => {
  await page.goto(playing(LOG, { loop: 'true' }, `${RECORDING}&record=1&record-framerate=24&record-viewport=1080x1080&record-max-seconds=14400`));
  const dialog = page.getByRole('dialog', { name: 'Record video' });
  await expect(dialog).toBeVisible({ timeout: 15_000 });
  await expect(dialog.locator('input[name="rec-size"][value="1080x1080"]')).toBeChecked();
  await expect(dialog.locator('input[name="rec-fps"][value="24"]')).toBeChecked();
  await expect(dialog.locator('input[name="rec-ends"][value="loop"]')).toBeChecked();
  expect(await state(page)).toBe('configuring');
  await expect(dialog.locator('.rec-supplied')).toBeVisible();
  await setTest(page, { sink: 'blob' });
  await dialog.locator('.rec-start').click();
  const confirm = page.getByRole('dialog', { name: 'Confirm recording' });
  await expect(confirm).toBeVisible();
  await expect(confirm.locator('.rec-supplied-list')).toContainText('record-max-seconds=14400');
  await expect(confirm.locator('.rec-supplied-list')).toContainText('record-viewport=1080x1080');
  expect(await state(page)).toBe('confirm');
  await confirm.getByRole('button', { name: 'Back' }).click();
  await expect(page.getByRole('dialog', { name: 'Record video' })).toBeVisible();
});

test('US-15: a recipe file prefills the dialog and asks for confirmation', async ({ page }) => {
  await openViewer(page);
  await page.locator('.record-btn').click();
  const dialog = page.getByRole('dialog', { name: 'Record video' });
  const recipe = {
    v: 1,
    source: { kind: 'url', path: LOG },
    options: { 'seconds-per-day': '2', hide: 'filenames', 'hash-seed': '31', 'bloom-intensity': '0.5' },
    record: { 'record-framerate': '25', 'record-viewport': '2560x1440', 'record-quality': 'max' },
  };
  await dialog.locator('input[type=file]').setInputFiles({ name: 'my.recipe.json', mimeType: 'application/json', buffer: Buffer.from(JSON.stringify(recipe)) });
  await expect(dialog).toContainText('Loaded the recipe my.recipe.json');
  await expect(dialog.locator('input[name="rec-size"][value="2560x1440"]')).toBeChecked();
  await expect(dialog.locator('input[name="rec-fps"][value="25"]')).toBeChecked();
  await expect(dialog.locator('input[name="rec-quality"][value="max"]')).toBeChecked();
  await expect(dialog.locator('#rec-in-spd')).toHaveValue('2');
  await expect(dialog.getByLabel('File names', { exact: true })).not.toBeChecked();
  await expect(dialog.locator('.rec-supplied')).toBeVisible();

  // Values that came from a file are confirmed before anything starts.
  await setTest(page, { sink: 'blob' });
  await dialog.locator('.rec-start').click();
  const confirm = page.getByRole('dialog', { name: 'Confirm recording' });
  await expect(confirm).toBeVisible();
  await expect(confirm.locator('.rec-supplied-list')).toContainText('record-viewport=2560x1440');
  expect(await state(page)).toBe('confirm');
  await confirm.getByRole('button', { name: 'Back' }).click();
  await expect(page.getByRole('dialog', { name: 'Record video' })).toBeVisible();

  await dialog.locator('input[type=file]').setInputFiles({ name: 'bad.json', mimeType: 'application/json', buffer: Buffer.from('{"x":1}') });
  await expect(dialog).toContainText('bad.json is not a Gource recording recipe');
});
