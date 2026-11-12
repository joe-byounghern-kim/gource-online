// SPDX-License-Identifier: GPL-3.0-or-later

// F1 against the real engine (E2E_REAL=1, F1 section 8.3): the sample
// /samples/ai-coauthors.log in fixed-dt mode. Decorations cost exactly two
// draw calls, hide=decorations removes them and changes the pixels, and then
// equals the log with its attribution stripped; ai_stats() matches the log;
// a bad rule shows its code and line and leaves the scene alone. Screenshots
// go to test-results/ai/ (and to E2E_AI_SHOTS, if set, for review).

import { mkdirSync, readFileSync, writeFileSync } from 'node:fs';

import { realLog } from './mock-engine';
import { expect, test, type Page } from '@playwright/test';

import type { EngineStatus } from '@gource-online/viewer';
import { RULE_PRESETS } from 'gource-player/ai/presets';

test.skip(process.env.E2E_REAL !== '1', 'set E2E_REAL=1 with the wasm build served (scripts/serve.sh)');

const OUT = 'test-results/ai';
const SHOTS = process.env.E2E_AI_SHOTS ?? OUT;
const SAMPLE_TEXT = readFileSync(new URL('./samples/ai-coauthors.log', import.meta.url), 'utf8');
const FIXED_DT = '0.0166667';
/** Cape-wearing tom (co:cursor) and Cursor Agent (au:cursor) commit within minutes here. */
const BOTH_AT = 1790246851;

const status = (page: Page) => page.evaluate(() => window.__gource!.status()) as Promise<EngineStatus>;
const step = (page: Page, ticks: number) => page.evaluate((n) => window.__gource!.step(n), ticks);
const setOption = (page: Page, name: string, value: string) =>
  page.evaluate(([n, v]) => window.__gource!.app.set_option(n!, v!), [name, value]);

async function open(page: Page, log: string, extra = ''): Promise<void> {
  await page.goto(realLog(log, extra, `fixed-dt=${FIXED_DT}`));
  await expect(page.locator('main[data-screen="viewer"]')).toBeVisible({ timeout: 60_000 });
  await expect.poll(async () => (await status(page)).seekable, { timeout: 60_000 }).toBe(true);
}

/** Steps until the frame draws both decoration blocks (12 commands); returns the ticks run. */
async function stepToDecorated(page: Page, limit = 6000): Promise<number> {
  let ticks = 0;
  while (ticks < limit) {
    await step(page, 10);
    ticks += 10;
    if ((await status(page)).draw_calls === 12) return ticks;
  }
  throw new Error(`no decorated frame within ${limit} ticks`);
}

/** The canvas alone (no page overlays): one more render, read back in the same task. */
async function canvasPng(page: Page): Promise<Buffer> {
  const url = await page.evaluate(() => {
    window.__gource!.step(0);
    return document.querySelector<HTMLCanvasElement>('.gource-canvas')!.toDataURL('image/png');
  });
  return Buffer.from(url.slice(url.indexOf(',') + 1), 'base64');
}

/** Mean absolute difference per channel and the share of pixels differing by more than 24 levels. */
async function compare(page: Page, a: Buffer, b: Buffer): Promise<{ mad: number; share: number }> {
  return page.evaluate(
    async ([x, y]) => {
      const read = async (data: string) => {
        const img = new Image();
        img.src = `data:image/png;base64,${data}`;
        await img.decode();
        const c = document.createElement('canvas');
        c.width = img.width;
        c.height = img.height;
        const ctx = c.getContext('2d')!;
        ctx.drawImage(img, 0, 0);
        return ctx.getImageData(0, 0, c.width, c.height).data;
      };
      const [p, q] = [await read(x!), await read(y!)];
      let sum = 0;
      let big = 0;
      for (let i = 0; i < p.length; i += 4) {
        const d = Math.abs(p[i]! - q[i]!) + Math.abs(p[i + 1]! - q[i + 1]!) + Math.abs(p[i + 2]! - q[i + 2]!);
        sum += d;
        if (d > 24) big++;
      }
      return { mad: sum / (p.length * 0.75), share: big / (p.length / 4) };
    },
    [a.toString('base64'), b.toString('base64')],
  );
}

/** The sample's credits, counted per commit (the `ai=` field is on a commit's first line). */
function sampleCounts() {
  const agents = new Map<string, number>();
  const models = new Map<string, number>();
  let assisted = 0;
  let agent = 0;
  for (const line of SAMPLE_TEXT.split('\n')) {
    const meta = line.split('|')[5];
    if (!meta?.startsWith('ai=')) continue;
    const credits = meta.slice(3).split(',').map((c) => c.split(':'));
    if (credits.some((c) => c[0] === 'au')) agent++;
    else assisted++;
    for (const id of new Set(credits.map((c) => c[1]!))) agents.set(id, (agents.get(id) ?? 0) + 1);
    for (const c of credits) if (c[2]) models.set(c[2], (models.get(c[2]) ?? 0) + 1);
  }
  return { agents, models, assisted, agent };
}

test('decorations cost two draw calls and hide=decorations equals the stripped log', async ({ page }) => {
  await open(page, '/samples/ai-coauthors.log');
  const ticks = await stepToDecorated(page);
  expect((await status(page)).draw_calls).toBe(12);
  const decorated = await canvasPng(page);

  await setOption(page, 'hide', 'decorations');
  await step(page, 0);
  expect((await status(page)).draw_calls).toBe(10);
  const hidden = await canvasPng(page);
  const diff = await compare(page, decorated, hidden);
  expect(diff.share).toBeGreaterThan(0);

  // The same run with the attribution stripped from the log draws the hidden frame.
  const stripped = SAMPLE_TEXT.replace(/\|\|ai=[^\n]*/g, '');
  await page.route((url) => url.pathname === '/samples/ai-stripped.log', (route) =>
    route.fulfill({ status: 200, contentType: 'text/plain', body: stripped }),
  );
  await open(page, '/samples/ai-stripped.log');
  await step(page, ticks);
  expect((await status(page)).draw_calls).toBe(10);
  const plain = await canvasPng(page);
  const same = await compare(page, hidden, plain);
  expect(same.mad).toBe(0);
});

test('ai_stats() matches the sample and the panel lists every agent as text', async ({ page }) => {
  await open(page, '/samples/ai-coauthors.log');
  const stats = JSON.parse(await page.evaluate(() => window.__gource!.app.ai_stats())) as {
    loaded: { commits: number; assisted: number; agent: number; sets: number; agents: Array<{ id: string; commits: number; as_author: number; models: Array<{ model: string; commits: number }> }> };
    played: { commits: number };
  };
  const want = sampleCounts();
  expect(stats.loaded.commits).toBe((await status(page)).commits_loaded);
  expect(stats.loaded.sets).toBeGreaterThan(0);
  expect(stats.loaded.assisted).toBe(want.assisted);
  expect(stats.loaded.agent).toBe(want.agent);
  expect(new Map(stats.loaded.agents.map((a) => [a.id, a.commits]))).toEqual(want.agents);
  const models = new Map(stats.loaded.agents.flatMap((a) => a.models.map((m) => [m.model, m.commits] as const)));
  expect(models).toEqual(want.models);
  expect(stats.played.commits).toBe(0);

  await page.getByRole('button', { name: 'Settings' }).click();
  const section = page.locator('#control-panel [data-section="style"]');
  await section.locator('> summary').click();
  for (const [id, n] of want.agents) {
    await expect(section.locator(`tr[data-agent="${id}"] td[data-stat="loaded"]`)).toHaveText(String(n));
  }
  await expect(section.locator('.ai-no-data')).toBeHidden();
  await step(page, 600);
  await expect(section.locator('.ai-totals[data-stat="played"]')).toHaveText(/^Played[1-9]/);
});

test('a bad rule shows its code and line and keeps the scene; presets are valid rules', async ({ page }) => {
  await open(page, '/samples/ai-coauthors.log');
  for (const preset of RULE_PRESETS) {
    const result = await page.evaluate((text) => window.__gource!.app.check_style(text), preset.rules);
    expect(JSON.parse(result), preset.id).toEqual({ ok: true });
  }
  await stepToDecorated(page);

  await page.getByRole('button', { name: 'Settings' }).click();
  const section = page.locator('#control-panel [data-section="style"]');
  await section.locator('> summary').click();
  const rules = section.locator('#opt-user-style');
  await rules.fill('ai:codex => hat=agent; glow=agent');
  await expect.poll(() => page.evaluate(() => window.__gource!.app.get_option('user-style'))).toBe('ai:codex => hat=agent; glow=agent');
  const before = await canvasPng(page);

  await rules.fill('ai:codex => hat=agent; glow=agent\nai => capes=on');
  const error = section.locator('#opt-user-style-error');
  await expect(error).toHaveText(/^Line 2: style_bad_property: Unknown property "capes"/);
  await expect(error).toHaveAttribute('data-code', 'style_bad_property');
  await page.screenshot({ path: `${SHOTS}/f1-web-rule-error.png` });
  // The previous rules stay active and the frame is unchanged.
  expect(await page.evaluate(() => window.__gource!.app.get_option('user-style'))).toBe('ai:codex => hat=agent; glow=agent');
  expect((await compare(page, before, await canvasPng(page))).mad).toBe(0);
});

test('screenshot: capes and shades next to the agent halo and glow', async ({ page }) => {
  mkdirSync(SHOTS, { recursive: true });
  await open(page, '/samples/ai-coauthors.log', '&camera-mode=track');
  for (let i = 0; i < 2000 && (await status(page)).time < BOTH_AT; i++) await step(page, 30);
  // A few frames on, jane (Claude) and tom (Cursor) wear capes and shades while Cursor Agent glows.
  await step(page, 75);
  expect((await status(page)).draw_calls).toBe(12);
  writeFileSync(`${SHOTS}/f1-web-hero.png`, await canvasPng(page));
  await setOption(page, 'hide', 'decorations');
  writeFileSync(`${SHOTS}/f1-web-hero-hidden.png`, await canvasPng(page));
});
