// SPDX-License-Identifier: GPL-3.0-or-later
// RPA presentation against the real WASM, existing WebGL passes and DOM host.
import { mkdirSync, writeFileSync } from 'node:fs';
import { expect, test, type Page } from '@playwright/test';
import { realLog } from './mock-engine';

test.skip(process.env.E2E_REAL !== '1', 'requires the real release WASM build');
const SHOTS = process.env.E2E_PR_SHOTS ?? 'test-results/pr';
const FIXED = '0.0166667';
const CODE = '1704067200|Author|A|/src/code.rs\n';
const NODE = '/src/.pull-requests/#7 Frozen proposal';
const SEED = `1704067200|Owner|A|${NODE}||ev=pr-opened;pr=7;pr-author=Owner;pr-created=1700000000;pr-state=open;pr-initial=1;pr-log=1,1699000000,1800000000,1704067200,c,p,u\n`;

async function open(page: Page, path: string, text: string, extra = '') {
  await page.route(`**${path}`, (route) => route.fulfill({ contentType: 'text/plain', body: text }));
  await page.goto(realLog(path, `&seconds-per-day=86400&auto-skip-seconds=-1${extra}`, `fixed-dt=${FIXED}`));
  await expect(page.locator('main[data-screen="viewer"]')).toBeVisible({ timeout: 60_000 });
  await expect.poll(() => page.evaluate(() => window.__gource!.status().seekable), { timeout: 60_000 }).toBe(true);
  await page.evaluate(() => window.__gource!.step(120));
}

async function png(page: Page): Promise<Buffer> {
  const data = await page.evaluate(() => {
    window.__gource!.step(0);
    return document.querySelector<HTMLCanvasElement>('.gource-canvas')!.toDataURL('image/png');
  });
  return Buffer.from(data.slice(data.indexOf(',') + 1), 'base64');
}

async function legendPixels(page: Page): Promise<number> {
  return page.evaluate(() => {
    window.__gource!.step(0);
    const canvas = document.querySelector<HTMLCanvasElement>('.gource-canvas')!;
    const copy = document.createElement('canvas');
    copy.width = canvas.width;
    copy.height = canvas.height;
    const context = copy.getContext('2d')!;
    context.drawImage(canvas, 0, 0);
    const sx = canvas.width / canvas.clientWidth;
    const sy = canvas.height / canvas.clientHeight;
    // Only the leftmost legend text is sampled. The fixed seed's code/PR
    // labels are centred, including at the short viewport, outside this strip.
    const pixels = context.getImageData(Math.round(10 * sx), Math.round(180 * sy),
      Math.round(Math.min(130, canvas.clientWidth - 20) * sx), Math.round(80 * sy)).data;
    let white = 0;
    for (let i = 0; i < pixels.length; i += 4) {
      if (pixels[i]! >= 180 && pixels[i + 1]! >= 180 && pixels[i + 2]! >= 180) white++;
    }
    return white;
  });
}

async function hoveredProposal(page: Page) {
  return page.evaluate(() => {
    const canvas = document.querySelector<HTMLCanvasElement>('.gource-canvas')!;
    const app = window.__gource!.app;
    for (let y = 8; y < canvas.clientHeight; y += 8) {
      for (let x = 8; x < canvas.clientWidth; x += 8) {
        app.pointer_move(x, y, 0);
        window.__gource!.step(0);
        const result = app.pr_hover?.();
        if (result && result !== 'null') return JSON.parse(result) as Record<string, string>;
      }
    }
    return null;
  });
}

for (const viewport of [{ width: 1280, height: 840 }, { width: 390, height: 800 }, { width: 390, height: 480 }]) {
  test(`recordable PR marker, replay age, key and honest coverage at ${viewport.width}x${viewport.height}px`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await open(page, '/pr-seed.log', CODE + SEED, '&pr-flow-hud=true');
    const status = await page.evaluate(() => window.__gource!.status());
    expect(status.draw_calls).toBeLessThanOrEqual(12);
    expect(status.files).toBe(2);
    expect(status.users).toBe(1);
    const canvas = page.locator('.gource-canvas');
    const bounds = await canvas.boundingBox();
    expect(bounds!.width).toBe(viewport.width);
    expect(bounds!.height).toBeGreaterThan(300);
    mkdirSync(SHOTS, { recursive: true });
    writeFileSync(`${SHOTS}/pr-seed-${viewport.width}x${viewport.height}.png`, await png(page));
    await page.screenshot({ path: `${SHOTS}/pr-page-${viewport.width}x${viewport.height}.png` });
    if (viewport.width === 390) {
      const hint = await page.locator('.canvas-keys').boundingBox();
      const player = await page.locator('.playback').boundingBox();
      expect(hint!.y + hint!.height).toBeLessThan(180);
      expect(player!.y).toBeGreaterThan(260);
      expect(await legendPixels(page)).toBeGreaterThan(50);
      await page.evaluate(() => {
        window.__gource!.app.set_option('hide-pull-requests', 'true');
        window.__gource!.step(0);
      });
      expect(await legendPixels(page)).toBe(0);
    }
    // Native extraction checks exact text/counts; this real pixel band proves
    // the legend is actually drawn clear of the persistent mobile overlays.
  });
}

test('hidden typed PRs equal the actual plain-code frame pixel for pixel', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 840 });
  await open(page, '/pr-hidden.log', CODE + SEED, '&hide-pull-requests=true&pr-flow-hud=true');
  const hidden = await png(page);
  const hiddenDraws = await page.evaluate(() => window.__gource!.status().draw_calls);
  await open(page, '/pr-plain.log', CODE, '&pr-flow-hud=true');
  expect(await png(page)).toEqual(hidden);
  expect(await page.evaluate(() => window.__gource!.status().draw_calls)).toBe(hiddenDraws);
});

test('full non-Latin markup-like hover stays text and does not attribute retained approval to requester', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 800 });
  const title = '#7 提案 "quoted" <img src=x onerror=alert(1)>';
  const node = `/src/.pull-requests/${title}`;
  const fields = 'pr=7;pr-author=Owner;pr-created=1700000000;pr-state=open';
  const log = CODE + `1704067200|Owner|A|${node}||ev=pr-opened;${fields};pr-initial=1\n`
    + `1704067200|Reviewer|M|${node}||ev=review-approved;${fields};pr-initial=0\n`
    + `1704067200|Requester|M|${node}||ev=pr-review-requested;${fields};pr-initial=0;pr-target=Target\n`;
  const origin = new URL(test.info().project.use.baseURL as string).origin;
  const unexpected: string[] = [];
  page.on('request', (request) => {
    const url = new URL(request.url());
    if (url.protocol === 'http:' || url.protocol === 'https:') {
      if (url.origin !== origin || url.pathname === '/x') unexpected.push(url.pathname);
    }
  });
  await open(page, '/pr-unicode.log', log, '&hide=users');
  // Exercise actual native picking through public pointer and redraw APIs. A
  // bounded scan avoids a copied layout model or fake hovered-node assignment.
  const picked = await hoveredProposal(page);
  expect(picked).toMatchObject({ title, owner: 'Owner', actor: 'Requester', action: 'Review approved', target: 'Target', age_days: '47' });
  const canvas = page.locator('.gource-canvas');
  await expect(canvas).toHaveAttribute('title', `${title}\nOwner: Owner\nState: Review approved\nLast actor: Requester\nAge: 47 days\nRequested: Target`);
  await expect(page.locator('img[src="x"]')).toHaveCount(0);
  expect(unexpected).toEqual([]);
  expect(await page.evaluate(() => window.__gource!.status().draw_calls)).toBeLessThanOrEqual(12);
  mkdirSync(SHOTS, { recursive: true });
  await page.screenshot({ path: `${SHOTS}/pr-unicode-hover-390.png` });
  await page.evaluate(() => {
    window.__gource!.app.set_option('hide-pull-requests', 'true');
    window.__gource!.step(0);
  });
  await expect(canvas).toHaveAttribute('title', '');
});

test('actual merger changes the distinct marker and retires it after two animation seconds', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 800 });
  const log = CODE + SEED
    + `1704067200|Reviewer|M|${NODE}||ev=review-approved;pr=7;pr-author=Owner;pr-created=1700000000;pr-state=open;pr-initial=0\n`
    + `1704067204|Actual merger|D|${NODE}||ev=pr-merged;pr=7;pr-author=Owner;pr-created=1700000000;pr-state=open;pr-initial=0\n`;
  await open(page, '/pr-terminal.log', log, '&hide=users&pr-flow-hud=true');
  expect(await hoveredProposal(page)).toMatchObject({ owner: 'Owner', actor: 'Reviewer', action: 'Review approved' });
  await page.evaluate(() => {
    for (let i = 0; i < 300 && window.__gource!.status().time < 1704067204; i++) window.__gource!.step(1);
    window.__gource!.step(1);
  });
  expect(await hoveredProposal(page)).toMatchObject({ owner: 'Owner', actor: 'Actual merger', action: 'Merged proposal' });
  expect(await page.evaluate(() => window.__gource!.status().files)).toBe(2);
  await page.evaluate(() => { window.__gource!.app.pointer_leave(); window.__gource!.step(0); });
  mkdirSync(SHOTS, { recursive: true });
  writeFileSync(`${SHOTS}/pr-merged-390.png`, await png(page));
  await page.evaluate(() => window.__gource!.step(150));
  expect(await page.evaluate(() => window.__gource!.status().files)).toBe(1);
  expect(await page.evaluate(() => window.__gource!.app.pr_hover?.())).toBe('null');
  expect(await page.evaluate(() => window.__gource!.status().draw_calls)).toBeLessThanOrEqual(12);
});
