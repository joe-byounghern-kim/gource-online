// SPDX-License-Identifier: GPL-3.0-or-later
import { test, expect, type Page } from '@playwright/test';

// <gource-viewer> on its own, with the mock engine: what any page gets from the element.

async function viewer(page: Page, attributes: Record<string, string>) {
  await page.goto('/?mock=1');
  return page.evaluate(async (attrs) => {
    const events: Array<{ type: string; detail: unknown }> = [];
    const el = document.createElement('gource-viewer');
    for (const type of ['gource-progress', 'gource-done', 'gource-error', 'gource-render-error', 'gource-key'])
      el.addEventListener(type, (e) => events.push({ type, detail: (e as CustomEvent).detail }));
    for (const [name, value] of Object.entries(attrs)) el.setAttribute(name, value);
    Object.assign(window, { __viewer: el, __events: events });
    el.style.cssText = 'display:block;width:320px;height:240px';
    el.canvas.style.cssText = 'width:100%;height:100%';
    document.body.append(el);
    return { canvas: el.canvas.className, child: el.firstElementChild === el.canvas, tabIndex: el.canvas.tabIndex };
  }, attributes);
}
const events = (page: Page) => page.evaluate(() => (window as unknown as { __events: Array<{ type: string; detail: Record<string, unknown> }> }).__events);

test('with src, it creates the engine, loads the log, plays from the first commit and reports the whole log', async ({ page }) => {
  expect(await viewer(page, { mock: '', src: '/samples/file-removal.log' })).toEqual({ canvas: 'gource-canvas', child: true, tabIndex: 0 });
  await expect.poll(async () => (await events(page)).some((e) => e.type === 'gource-done')).toBe(true);
  const seen = await events(page);
  const progress = seen.filter((e) => e.type === 'gource-progress');
  expect(progress.length).toBeGreaterThan(0);
  expect(progress.at(-1)!.detail).toMatchObject({ playable: true });
  expect(seen.find((e) => e.type === 'gource-done')!.detail).toMatchObject({ format: 'custom', skipped: 0 });
  expect(seen.some((e) => e.type === 'gource-error')).toBe(false);
  const state = await page.evaluate(() => {
    const el = (window as unknown as { __viewer: HTMLElementTagNameMap['gource-viewer'] }).__viewer;
    return { playing: el.playing, focused: document.activeElement === el.canvas, seekable: el.status()?.seekable, resized: window.__gourceMock!.calls.some((c) => c.method === 'resize') };
  });
  expect(state).toEqual({ playing: true, focused: false, seekable: true, resized: true });
});

test('an empty or missing log is reported as an error event, and nothing plays', async ({ page }) => {
  await page.route('**/empty.log', (route) => route.fulfill({ body: '' }));
  await viewer(page, { mock: '', src: '/empty.log' });
  await expect.poll(async () => (await events(page)).find((e) => e.type === 'gource-error')?.detail.code).toBe('empty_log');
  expect(await page.evaluate(() => (window as unknown as { __viewer: HTMLElementTagNameMap['gource-viewer'] }).__viewer.playing)).toBe(false);
  await page.route('**/missing.log', (route) => route.fulfill({ status: 404, body: 'nope' }));
  await viewer(page, { mock: '', src: '/missing.log' });
  await expect.poll(async () => (await events(page)).find((e) => e.type === 'gource-error')?.detail.code).toBe('log_failed');
});

test('keys reach the engine from the canvas, Escape never does, and a handled key is announced', async ({ page }) => {
  await viewer(page, { mock: '', src: '/samples/file-removal.log' });
  await expect.poll(async () => (await events(page)).some((e) => e.type === 'gource-done')).toBe(true);
  await page.evaluate(() => {
    (window as unknown as { __viewer: HTMLElementTagNameMap['gource-viewer'] }).__viewer.canvas.focus();
    window.__gourceMock!.calls.length = 0;
  });
  await page.keyboard.press('Escape');
  await page.keyboard.press('Space');
  const keys = await page.evaluate(() => window.__gourceMock!.calls.filter((c) => c.method === 'key').map((c) => c.args[0]));
  expect(keys).not.toContain('Escape');
  expect(keys).toContain(' ');
  expect((await events(page)).some((e) => e.type === 'gource-key' && e.detail.key === ' ')).toBe(true);
});

test('one log per element, one engine per element, and removing it frees the engine', async ({ page }) => {
  await viewer(page, { mock: '', src: '/samples/file-removal.log' });
  await expect.poll(async () => (await events(page)).some((e) => e.type === 'gource-done')).toBe(true);
  const result = await page.evaluate(async () => {
    const el = (window as unknown as { __viewer: HTMLElementTagNameMap['gource-viewer'] }).__viewer;
    const errors: string[] = [];
    try { el.attach(el.engine!); } catch (e) { errors.push((e as Error).message); }
    try { el.load({ kind: 'url', url: '/other.log' }); } catch (e) { errors.push((e as Error).message); }
    try { await el.play({ kind: 'url', url: '/other.log' }); } catch (e) { errors.push((e as Error).message); }
    const warnings: string[] = [];
    const warn = console.warn;
    console.warn = (message: string) => warnings.push(message);
    el.setAttribute('src', '/other.log');
    console.warn = warn;
    const instances = window.__gourceMock!.instances;
    el.remove();
    await new Promise((resolve) => setTimeout(resolve, 0));
    // Disposed for good: putting it back neither restarts it nor binds its input again.
    document.body.append(el);
    await new Promise((resolve) => setTimeout(resolve, 50));
    return { errors, warnings, instances, after: window.__gourceMock!.instances, engine: el.engine, playing: el.playing,
      freed: window.__gourceMock!.calls.some((c) => c.method === 'free') };
  });
  expect(result.errors).toEqual(['<gource-viewer> already has an engine.', '<gource-viewer> needs an engine and loads one log.', '<gource-viewer> already has an engine.']);
  expect(result.warnings).toEqual([expect.stringContaining('plays one log')]);
  expect(result).toMatchObject({ engine: null, freed: true, instances: 1, after: 1 });
});

test('src set after the element is on the page plays, without taking focus from the page', async ({ page }) => {
  await viewer(page, { mock: '' });
  await page.evaluate(() => {
    const input = document.createElement('input');
    document.body.append(input);
    input.focus();
    (window as unknown as { __viewer: HTMLElement }).__viewer.setAttribute('src', '/samples/file-removal.log');
  });
  await expect.poll(async () => (await events(page)).some((e) => e.type === 'gource-done')).toBe(true);
  expect(await page.evaluate(() => document.activeElement?.tagName)).toBe('INPUT');
});

test('moving the element while its engine loads starts one engine, not two', async ({ page }) => {
  await page.goto('/?mock=1');
  const result = await page.evaluate(async () => {
    const errors: string[] = [];
    const el = document.createElement('gource-viewer');
    el.setAttribute('mock', '');
    el.setAttribute('src', '/samples/file-removal.log');
    el.addEventListener('gource-error', (e) => errors.push(e.detail.code));
    document.body.append(el);
    const box = document.createElement('div');
    document.body.append(box);
    box.append(el);
    await new Promise<void>((resolve) => el.addEventListener('gource-done', () => resolve()));
    return { errors, instances: window.__gourceMock!.instances };
  });
  expect(result).toEqual({ errors: [], instances: 1 });
});

test('moving the element within the page keeps its engine', async ({ page }) => {
  await viewer(page, { mock: '', src: '/samples/file-removal.log' });
  await expect.poll(async () => (await events(page)).some((e) => e.type === 'gource-done')).toBe(true);
  const kept = await page.evaluate(async () => {
    const el = (window as unknown as { __viewer: HTMLElementTagNameMap['gource-viewer'] }).__viewer;
    const box = document.createElement('div');
    document.body.append(box);
    box.append(el);
    await new Promise((resolve) => setTimeout(resolve, 0));
    return el.engine !== null && el.playing;
  });
  expect(kept).toBe(true);
});
