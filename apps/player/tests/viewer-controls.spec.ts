// SPDX-License-Identifier: GPL-3.0-or-later

// The viewer screen's controls on the placeholder engine: the panel, the filters, a link's options, playback, and
// keyboard and touch input.

import { expect, test, type Page } from '@playwright/test';
import { createServer, type ServerResponse } from 'node:http';
import { DROPPED_LOG_PORT } from '../playwright.config';
import { SMALL_LOG, engineCalls, playing, setOptionCalls } from './mock-engine';

async function openViewer(page: Page, options: Record<string, string> = {}) {
  await page.route('**/small.log', (route) => route.fulfill({ status: 200, contentType: 'text/plain', body: SMALL_LOG }));
  await page.goto(playing('/small.log', options));
  await expect(page.locator('[data-screen="viewer"]')).toBeVisible();
  await expect(page.locator('.seek')).toBeEnabled();
}

/** The options in the page's link. */
const link = (page: Page) => new URLSearchParams(new URL(page.url()).hash.slice(1));

async function keyCalls(page: Page): Promise<Array<[string, boolean]>> {
  return (await engineCalls(page, 'key')).map((c) => [c.args[0] as string, c.args[1] as boolean]);
}

test('panel controls call set_option with Gource option names', async ({ page }) => {
  await openViewer(page);
  const before = (await setOptionCalls(page)).length;
  await page.getByRole('button', { name: 'Settings' }).click();
  const panel = page.locator('#control-panel');
  await expect(panel).toBeVisible();

  // Colours & bloom.
  await panel.locator('[data-option="background-colour"]').fill('#203040');
  await panel.locator('[data-option="bloom-intensity"]').fill('1.5');
  await panel.locator('[data-option="bloom-multiplier"]').fill('2');

  // Playback & camera.
  await panel.locator('[data-option="camera-mode"]').selectOption('track');
  await panel.locator('[data-control="disable-auto-rotate"] .switch').click();
  await panel.locator('[data-option="time-scale"]').fill('2');

  // Visibility & labels.
  await panel.locator('[data-section="visibility"] > summary').click();
  await panel.locator('label.toggle-chip', { hasText: 'File names' }).click();
  await panel.locator('label.toggle-chip', { hasText: 'Dir names' }).click();
  await panel.locator('[data-control="key"] .switch').click();
  await panel.locator('[data-option="title"]').fill('Gource history');
  await panel.locator('[data-option="font-scale"]').fill('1.5');

  await expect
    .poll(async () => (await setOptionCalls(page)).slice(before))
    .toEqual(
      expect.arrayContaining([
        ['background-colour', '203040'],
        ['bloom-intensity', '1.5'],
        ['bloom-multiplier', '2'],
        ['camera-mode', 'track'],
        ['disable-auto-rotate', 'true'],
        ['time-scale', '2'],
        ['hide', 'filenames'],
        ['hide', 'filenames,dirnames'],
        ['key', 'true'],
        ['title', 'Gource history'],
        ['font-scale', '1.5'],
      ]),
    );

  // Colour reseed sets hash-seed, which needs a restart.
  const restartsBefore = (await engineCalls(page, 'restart')).length;
  await panel.locator('[data-action="reseed"]').click();
  const seed = (await setOptionCalls(page)).filter(([n]) => n === 'hash-seed').at(-1);
  expect(seed?.[1]).toMatch(/^\d+$/);
  expect((await engineCalls(page, 'restart')).length).toBe(restartsBefore + 1);

  // Playback bar speed control.
  await page.locator('select.speed').selectOption('1');
  expect((await setOptionCalls(page)).at(-1)).toEqual(['seconds-per-day', '1']);

  // The link mirrors the settings.
  await expect.poll(() => link(page).get('o.seconds-per-day')).toBe('1');
  expect(link(page).get('o.background-colour')).toBe('203040');
  expect(link(page).get('o.hide')).toBe('filenames,dirnames');
  expect(link(page).get('o.camera-mode')).toBe('track');
  expect(link(page).get('o.disable-auto-rotate')).toBe('true');
  expect(link(page).get('o.title')).toBe('Gource history');
});

test('filters apply all filter options and restart the simulation', async ({ page }) => {
  await openViewer(page);
  await page.getByRole('button', { name: 'Settings' }).click();
  const panel = page.locator('#control-panel');
  await panel.locator('[data-section="filters"] > summary').click();
  const apply = panel.getByRole('button', { name: 'Apply filters' });
  await expect(apply).toBeDisabled();

  const before = (await setOptionCalls(page)).length;
  await panel.locator('[data-option="file-filter"]').fill('\\.md$\n^/docs/');
  await panel.locator('[data-option="user-show-filter"]').fill('^Alice$');
  await panel.locator('[data-option="start-date"]').fill('2009-01-01');
  // Nothing is sent until Apply.
  expect((await setOptionCalls(page)).length).toBe(before);
  await expect(apply).toBeEnabled();

  const restartsBefore = (await engineCalls(page, 'restart')).length;
  await apply.click();
  expect((await setOptionCalls(page)).slice(before)).toEqual([
    ['file-filter', '\\.md$\n^/docs/'],
    ['file-show-filter', ''],
    ['user-filter', ''],
    ['user-show-filter', '^Alice$'],
    ['start-date', '2009-01-01'],
    ['stop-date', ''],
  ]);
  const calls = await engineCalls(page);
  expect(calls.map((c) => c.method).lastIndexOf('restart')).toBeGreaterThan(calls.map((c) => c.method).lastIndexOf('set_option'));
  expect((await engineCalls(page, 'restart')).length).toBe(restartsBefore + 1);
  await expect(panel.locator('.apply-status')).toContainText('Applied');
  await expect(apply).toBeDisabled();

  await expect.poll(() => link(page).get('o.file-filter')).toBe('\\.md$\n^/docs/');
  expect(link(page).get('o.user-show-filter')).toBe('^Alice$');
  expect(link(page).get('o.start-date')).toBe('2009-01-01');

  // An invalid regular expression is reported and nothing restarts.
  const restarts = (await engineCalls(page, 'restart')).length;
  await panel.locator('[data-option="user-filter"]').fill('([');
  await apply.click();
  await expect(panel.locator('[data-control="user-filter"] .control-error')).toContainText('invalid');
  expect((await engineCalls(page, 'restart')).length).toBe(restarts);
});

test('a failed filter Apply leaves the engine, the panel and the link as they were', async ({ page }) => {
  await openViewer(page);
  await page.getByRole('button', { name: 'Settings' }).click();
  const panel = page.locator('#control-panel');
  await panel.locator('[data-section="filters"] > summary').click();
  await panel.locator('[data-option="file-filter"]').fill('mod1/');
  await panel.locator('[data-option="user-filter"]').fill('(');
  const restarts = (await engineCalls(page, 'restart')).length;
  await panel.getByRole('button', { name: 'Apply filters' }).click();
  await expect(panel.locator('.apply-status')).toHaveText('Not applied');
  await expect(panel.locator('[data-option="user-filter"]')).toHaveAttribute('aria-invalid', 'true');
  await expect(panel.locator('[data-option="user-filter"]')).toHaveAttribute('aria-describedby', 'opt-user-filter-error');
  expect(await page.evaluate(() => window.__gource!.app.get_option('file-filter'))).toBe('');
  expect((await engineCalls(page, 'restart')).length).toBe(restarts);

  // A later live change does not carry the rejected filters into the link.
  await panel.locator('[data-option="bloom-intensity"]').fill('1.5');
  await expect.poll(() => link(page).get('o.bloom-intensity')).toBe('1.5');
  expect(link(page).has('o.file-filter')).toBe(false);

  // Fixing the filter applies both.
  await panel.locator('[data-option="user-filter"]').fill('^Bob$');
  await panel.getByRole('button', { name: 'Apply filters' }).click();
  await expect(panel.locator('.apply-status')).toContainText('Applied');
  await expect(panel.locator('[data-option="user-filter"]')).not.toHaveAttribute('aria-invalid');
  expect(await page.evaluate(() => window.__gource!.app.get_option('file-filter'))).toBe('mod1/');
  await expect.poll(() => link(page).get('o.file-filter')).toBe('mod1/');
});

test('a link restores settings into the engine and the panel', async ({ page }) => {
  await openViewer(page, {
    'seconds-per-day': '1',
    hide: 'filenames,usernames',
    'bloom-intensity': '1.25',
    'camera-mode': 'track',
    'file-filter': 'vendor/\n\\.min\\.js$',
  });
  expect(await setOptionCalls(page)).toEqual(
    expect.arrayContaining([
      ['seconds-per-day', '1'],
      ['hide', 'filenames,usernames'],
      ['bloom-intensity', '1.25'],
      ['camera-mode', 'track'],
      ['file-filter', 'vendor/\n\\.min\\.js$'],
    ]),
  );
  // Options are applied before the log starts.
  const methods = (await engineCalls(page)).map((c) => c.method);
  expect(methods.lastIndexOf('set_option')).toBeLessThan(methods.indexOf('begin_log'));

  await page.getByRole('button', { name: 'Settings' }).click();
  const panel = page.locator('#control-panel');
  await expect(panel.locator('[data-option="bloom-intensity"]')).toHaveValue('1.25');
  await expect(panel.locator('[data-option="camera-mode"]')).toHaveValue('track');
  await expect(panel.locator('[data-hide="filenames"]')).toBeChecked();
  await expect(panel.locator('[data-hide="usernames"]')).toBeChecked();
  await expect(panel.locator('[data-hide="date"]')).not.toBeChecked();
  await expect(page.locator('select.speed')).toHaveValue('1');
  await panel.locator('[data-section="filters"] > summary').click();
  await expect(panel.locator('[data-option="file-filter"]')).toHaveValue('vendor/\n\\.min\\.js$');

  // Round trip: change something, reload, and it is still there.
  await panel.locator('[data-option="bloom-multiplier"]').fill('2.5');
  await expect.poll(() => link(page).get('o.bloom-multiplier')).toBe('2.5');
  await page.reload();
  await expect(page.locator('[data-screen="viewer"]')).toBeVisible();
  expect(await setOptionCalls(page)).toEqual(
    expect.arrayContaining([
      ['bloom-multiplier', '2.5'],
      ['bloom-intensity', '1.25'],
      ['hide', 'filenames,usernames'],
    ]),
  );
});

test('the pull request controls of a log never ask a server for anything, and their hide aliases agree', async ({ page }) => {
  let logReads = 0;
  let serverCalls = 0;
  await page.route('**/import.log', (route) => {
    logReads++;
    return route.fulfill({ body: SMALL_LOG, contentType: 'text/plain' });
  });
  await page.route('**/api/**', (route) => { serverCalls++; return route.abort(); });
  await page.goto(playing('/import.log', { hide: 'pull-requests' }));
  await expect(page.locator('[data-screen="viewer"]')).toBeVisible();
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.locator('#control-panel [data-section="visibility"] > summary').click();
  await expect(page.locator('#opt-hide-pull-requests')).not.toBeChecked();
  await page.locator('#opt-pull-request-mode').selectOption('full');
  await expect.poll(() => link(page).get('o.pull-request-mode')).toBe('full');
  expect(logReads).toBe(1);
  expect(serverCalls).toBe(0);
  await page.locator('#opt-hide-pull-requests').check();
  await expect.poll(() => link(page).get('o.hide')).toBeNull();
  expect(await page.evaluate(() => window.__gource?.app.get_option('hide'))).toBe('');
});

test('playback controls, seek tooltip and input forwarding', async ({ page }) => {
  await openViewer(page);
  const play = page.locator('.play-btn');
  await expect(play).toHaveAttribute('aria-label', 'Pause');
  await play.click();
  expect((await engineCalls(page, 'set_paused')).at(-1)?.args).toEqual([true]);
  await expect(play).toHaveAttribute('aria-label', 'Play');
  await play.click();
  expect((await engineCalls(page, 'set_paused')).at(-1)?.args).toEqual([false]);

  // Seek with a date tooltip from date_at.
  const seek = page.locator('.seek');
  const box = (await seek.boundingBox())!;
  await page.mouse.move(box.x + box.width * 0.5, box.y + box.height / 2);
  await expect(page.locator('.seek-tip')).toBeVisible();
  await expect(page.locator('.seek-tip')).toContainText(/\d{4}/);
  await seek.fill('5000');
  expect((await engineCalls(page, 'seek')).at(-1)?.args).toEqual([0.5]);

  // Pointer, wheel and keys reach the engine in canvas CSS pixels.
  const canvas = page.locator('.gource-canvas');
  const cbox = (await canvas.boundingBox())!;
  await page.mouse.move(cbox.x + 200, cbox.y + 150);
  await page.mouse.down();
  await page.mouse.up();
  await page.mouse.wheel(0, 120);
  expect((await engineCalls(page, 'pointer_move')).at(-1)?.args).toEqual([200, 150, 0]);
  expect((await engineCalls(page, 'pointer_down')).at(-1)?.args).toEqual([200, 150, 0]);
  expect((await engineCalls(page, 'pointer_up')).at(-1)?.args).toEqual([200, 150, 0]);
  await expect.poll(async () => (await engineCalls(page, 'wheel')).at(-1)?.args).toEqual([120]);
  await expect(canvas).toBeFocused();
  await page.keyboard.press('Space');
  expect((await engineCalls(page, 'key')).filter((c) => c.args[0] === ' ').map((c) => c.args[1])).toEqual([true, false]);
  await expect(play).toHaveAttribute('aria-label', 'Play');

  // Keys typed into the panel are not forwarded.
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.locator('[data-section="visibility"] > summary').click();
  const keysBefore = (await engineCalls(page, 'key')).length;
  await page.locator('[data-option="title"]').fill('');
  await page.locator('[data-option="title"]').press('a');
  expect((await engineCalls(page, 'key')).length).toBe(keysBefore);

  // Debug overlay shows engine timings.
  await page.getByRole('button', { name: 'Toggle performance overlay' }).click();
  await expect(page.locator('.debug-overlay')).toBeVisible();
  await expect(page.locator('.debug-overlay')).toContainText('fps');
  await expect(page.locator('.debug-overlay')).toContainText('draws      12');

  // Resizing the viewport resizes the engine.
  await page.setViewportSize({ width: 900, height: 600 });
  await expect.poll(async () => (await engineCalls(page, 'resize')).at(-1)?.args).toEqual([900, 600, 1]);
});

test('back from the viewer frees the engine', async ({ page }) => {
  await openViewer(page);
  await page.getByRole('button', { name: 'Open another log' }).click();
  await expect(page.locator('[data-screen="viewer"]')).toHaveCount(0);
  expect((await engineCalls(page, 'free')).length).toBe(1);
});

test('a history that stops loading leaves a clear state and a retry', async ({ page }) => {
  const lines: string[] = [];
  for (let i = 0; i < 4000; i++) lines.push(`${1_230_000_000 + i * 3600}|User ${i % 7}|A|/dir${i % 13}/file${i}.c`);
  const body = `${lines.join('\n')}\n`;
  const open = new Set<ServerResponse>();
  let requests = 0;
  let complete = false;
  // Sends part of a large log, then drops the connection on request.
  const server = createServer((_req, res) => {
    requests++;
    res.writeHead(200, { 'content-type': 'text/plain', 'access-control-allow-origin': '*' });
    if (complete) {
      res.end(body);
      return;
    }
    res.write(body.slice(0, body.length / 2));
    open.add(res);
  });
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(DROPPED_LOG_PORT, '127.0.0.1', resolve);
  });
  try {
    await page.goto(playing(`http://127.0.0.1:${DROPPED_LOG_PORT}/big.log`));
    await expect(page.locator('[data-screen="viewer"]')).toBeVisible();
    await expect(page.locator('.mini-progress')).toBeVisible();

    for (const res of open) res.socket?.destroy();
    open.clear();
    const banner = page.locator('.viewer-banner');
    await expect(banner).toBeVisible();
    await expect(banner).toContainText('stopped loading');
    await expect(page.locator('.mini-progress')).toBeHidden();
    await expect(page.locator('.history-incomplete')).toContainText('History incomplete');
    await expect(page.locator('.seek')).toBeDisabled();
    await expect(page.locator('.seek')).toHaveAttribute('title', /did not load completely/);
    // Dismissing the banner keeps the note.
    await banner.getByRole('button', { name: 'Dismiss' }).click();
    await expect(page.locator('.history-incomplete')).toBeVisible();

    // Retry loads the history again, completely this time.
    complete = true;
    await page.locator('.history-incomplete').getByRole('button', { name: 'Retry' }).click();
    await expect(page.locator('[data-screen="viewer"]')).toBeVisible();
    await expect(page.locator('.seek')).toBeEnabled();
    await expect(page.locator('.history-incomplete')).toHaveCount(1);
    await expect(page.locator('.history-incomplete')).toBeHidden();
    expect(requests).toBe(2);
  } finally {
    server.closeAllConnections();
    await new Promise((resolve) => server.close(resolve));
  }
});

test('a log file dropped on the page plays under its name', async ({ page }) => {
  await page.goto('/?mock=1');
  const transfer = await page.evaluateHandle((text) => {
    const dt = new DataTransfer();
    dt.items.add(new File([text], 'dropped.log', { type: 'text/plain' }));
    return dt;
  }, SMALL_LOG);
  await page.locator('main.player').dispatchEvent('drop', { dataTransfer: transfer });
  await expect(page.locator('[data-screen="viewer"]')).toBeVisible();
  await expect(page.locator('.viewer-name')).toHaveText('dropped.log');
});

test('a stalled font does not hold the visualisation back', async ({ page }) => {
  await page.route('**/fonts/FreeSans.ttf', () => {
    // Never answered.
  });
  await page.route('**/small.log', (route) => route.fulfill({ status: 200, contentType: 'text/plain', body: SMALL_LOG }));
  // The page's load event waits for the font too.
  await page.goto(playing('/small.log'), { waitUntil: 'commit' });
  await expect(page.locator('[data-screen="viewer"]')).toBeVisible({ timeout: 10_000 });
});

test.describe('keyboard', () => {
  test('Tab is Gource\'s next-user key only on the canvas; Shift+Tab and Esc leave it', async ({ page }) => {
    await openViewer(page);
    const canvas = page.locator('.gource-canvas');
    await expect(canvas).toBeFocused();
    await expect(canvas).toHaveAttribute('aria-describedby', 'canvas-keys');
    await expect(page.locator('#canvas-keys')).toContainText('next user');

    // Tab on the canvas goes to the engine and keeps the focus there.
    await page.keyboard.press('Tab');
    await expect(canvas).toBeFocused();
    expect(await keyCalls(page)).toContainEqual(['Tab', true]);
    // Keyboard focus on the canvas shows a ring and the key hint.
    await expect(page.locator('#canvas-keys')).toHaveCSS('opacity', '1');
    expect(await canvas.evaluate((el) => getComputedStyle(el).outlineStyle)).toBe('solid');

    // Shift+Tab moves focus to the toolbar before the canvas, and is not sent.
    const before = (await keyCalls(page)).length;
    await page.keyboard.press('Shift+Tab');
    await expect(page.getByRole('button', { name: 'Settings' })).toBeFocused();
    expect((await keyCalls(page)).slice(before).filter(([k]) => k === 'Tab')).toEqual([]);

    // From the toolbar, Tab reaches the canvas again; Esc then moves on to the playback bar.
    await page.keyboard.press('Tab');
    await expect(canvas).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(page.locator('.play-btn')).toBeFocused();
    await page.keyboard.press('Tab');
    await expect(page.locator('.seek')).toBeFocused();

    // Tab with nothing focused is the browser's.
    await page.evaluate(() => (document.activeElement as HTMLElement | null)?.blur());
    const calls = (await keyCalls(page)).length;
    await page.keyboard.press('Tab');
    expect((await keyCalls(page)).slice(calls).filter(([k]) => k === 'Tab')).toEqual([]);
    expect(await page.evaluate(() => document.activeElement?.tagName)).not.toBe('BODY');
  });

  test('held keys are released when focus or the page moves away', async ({ page }) => {
    await openViewer(page);
    const canvas = page.locator('.gource-canvas');
    await expect(canvas).toBeFocused();

    // Focus moves to a toolbar button while ArrowRight is down.
    await page.keyboard.down('ArrowRight');
    await page.getByRole('button', { name: 'Settings' }).focus();
    expect((await keyCalls(page)).filter(([k]) => k === 'ArrowRight')).toEqual([
      ['ArrowRight', true],
      ['ArrowRight', false],
    ]);
    await page.keyboard.up('ArrowRight');
    expect((await keyCalls(page)).filter(([k]) => k === 'ArrowRight')).toHaveLength(2);

    // A keyup that lands on another element (or with Alt held) still releases the key.
    await canvas.focus();
    await page.keyboard.down('ArrowLeft');
    await page.evaluate(() => {
      const btn = document.querySelector('.play-btn')!;
      btn.dispatchEvent(new KeyboardEvent('keyup', { key: 'ArrowLeft', code: 'ArrowLeft', altKey: true, bubbles: true }));
    });
    expect((await keyCalls(page)).filter(([k]) => k === 'ArrowLeft').at(-1)).toEqual(['ArrowLeft', false]);
    await page.keyboard.up('ArrowLeft');

    // The window loses focus (Alt+Tab) or the tab is hidden.
    await page.keyboard.down('ArrowUp');
    await page.evaluate(() => window.dispatchEvent(new Event('blur')));
    expect((await keyCalls(page)).filter(([k]) => k === 'ArrowUp').at(-1)).toEqual(['ArrowUp', false]);
    await page.keyboard.up('ArrowUp');
    await page.keyboard.down('ArrowDown');
    await page.evaluate(() => {
      Object.defineProperty(document, 'visibilityState', { value: 'hidden', configurable: true });
      document.dispatchEvent(new Event('visibilitychange'));
    });
    expect((await keyCalls(page)).filter(([k]) => k === 'ArrowDown').at(-1)).toEqual(['ArrowDown', false]);
  });
});

test.describe('touch', () => {
  test.use({ hasTouch: true, viewport: { width: 390, height: 780 } });

  type Point = { x: number; y: number; id: number };
  type TouchType = 'touchStart' | 'touchMove' | 'touchEnd' | 'touchCancel';
  /** Touch events through one DevTools session (it tracks the active touches). */
  async function touchscreen(page: Page) {
    const cdp = await page.context().newCDPSession(page);
    return (type: TouchType, points: Point[]) => cdp.send('Input.dispatchTouchEvent', { type, touchPoints: points });
  }

  test('a pinch zooms in wheel notches without dragging the camera', async ({ page }) => {
    await openViewer(page);
    const box = (await page.locator('.gource-canvas').boundingBox())!;
    const cx = box.x + 195;
    const cy = box.y + 300;
    const a = (d: number): Point => ({ x: cx - d, y: cy, id: 1 });
    const b = (d: number): Point => ({ x: cx + d, y: cy, id: 2 });
    const start = (await engineCalls(page)).length;
    const touch = await touchscreen(page);

    await touch('touchStart', [a(20)]);
    await touch('touchStart', [a(20), b(20)]);
    // Spread from 40 px to 120 px: ln(3) / ln(1.1) = 11.5 notches in.
    for (let d = 25; d <= 60; d += 5) await touch('touchMove', [a(d), b(d)]);
    await touch('touchEnd', [b(60)]);
    await touch('touchEnd', []);

    const calls = (await engineCalls(page)).slice(start);
    const wheel = calls.filter((c) => c.method === 'wheel').map((c) => c.args[0]);
    expect(wheel.length).toBe(11);
    expect(new Set(wheel)).toEqual(new Set([-100]));
    // One pointer drove the engine: down at the first finger, up when the second arrived, no jumps.
    expect(calls.filter((c) => c.method === 'pointer_down')).toHaveLength(1);
    expect(calls.filter((c) => c.method === 'pointer_up')).toHaveLength(1);
    expect(calls.filter((c) => c.method === 'pointer_move').filter((c) => (c.args[2] as number) !== 0)).toEqual([]);

    // Pinching in zooms out.
    const mark = (await engineCalls(page)).length;
    await touch('touchStart', [a(60)]);
    await touch('touchStart', [a(60), b(60)]);
    await touch('touchMove', [a(40), b(40)]);
    await touch('touchEnd', []);
    const out = (await engineCalls(page)).slice(mark).filter((c) => c.method === 'wheel').map((c) => c.args[0]);
    expect(out.length).toBe(4); // ln(120 / 80) / ln(1.1) = 4.25
    expect(new Set(out)).toEqual(new Set([100]));
  });

  test('a cancelled touch ends the drag', async ({ page }) => {
    await openViewer(page);
    const box = (await page.locator('.gource-canvas').boundingBox())!;
    const start = (await engineCalls(page)).length;
    const touch = await touchscreen(page);
    await touch('touchStart', [{ x: box.x + 100, y: box.y + 200, id: 1 }]);
    await touch('touchMove', [{ x: box.x + 120, y: box.y + 210, id: 1 }]);
    await touch('touchCancel', []);
    const methods = (await engineCalls(page)).slice(start).map((c) => c.method);
    expect(methods).toContain('pointer_down');
    expect(methods.lastIndexOf('pointer_up')).toBeGreaterThan(methods.indexOf('pointer_down'));
    // A later hover is a plain move, not a drag.
    await page.mouse.move(box.x + 50, box.y + 50);
    expect((await engineCalls(page, 'pointer_move')).at(-1)?.args[2]).toBe(0);
  });
});
