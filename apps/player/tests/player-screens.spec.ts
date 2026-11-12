// SPDX-License-Identifier: GPL-3.0-or-later

// The player's screens on its own build: options, the AI statistics and recording.
// The dev-server specs next to it (ai-mock, record-mock, viewer-controls) cover them in depth.

import { expect, test, type Locator, type Page } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { expectState, fragment, mockCalls, startOrigins, type Origins } from './servers';

const AI_SAMPLE = readFileSync(new URL('./samples/ai-coauthors.log', import.meta.url));

let o: Origins;
let stop: () => Promise<void>;
test.beforeAll(async () => ({ origins: o, stop } = await startOrigins()));
test.afterAll(async () => stop());

async function openFile(page: Page, query = '?mock=1') {
  await page.goto(`${o.player}/${query}`);
  await page.locator('.player-file-input').setInputFiles({ name: 'ai-coauthors.log', mimeType: 'text/plain', buffer: AI_SAMPLE });
  await expectState(page, 'playing');
  // The whole log is in: seeking, the AI summary and the recording depend on it.
  await expect(page.locator('.seek')).toBeEnabled();
}

test('the options panel changes the engine, and a log by URL keeps its link equal to the view', async ({ page }) => {
  await page.goto(`${o.player}/?mock=1${fragment({ src: `${o.logs}/screens.log`, title: 'Linked' })}`);
  await expectState(page, 'playing');
  await expect(page.getByRole('button', { name: 'Copy link to this view' })).toBeVisible();
  await page.locator('select.speed').selectOption('5');
  await expect.poll(() => mockCalls(page)).toContain('set_option(seconds-per-day,5)');
  await expect.poll(() => new URL(page.url()).hash).toContain('o.seconds-per-day=5');
  expect(new URLSearchParams(new URL(page.url()).hash.slice(1)).get('title')).toBe('Linked');
  // The link still plays the same log with the same option.
  await page.reload();
  await expectState(page, 'playing');
  await expect(page.locator('select.speed')).toHaveValue('5');
});

test('a log the player\'s own server on this computer serves (npx gource-online) has no link to copy', async ({ page }) => {
  await page.goto(`${o.player}/?mock=1${fragment({ src: '/same-cli.log', title: 'my checkout' })}`);
  await expectState(page, 'playing');
  await expect(page.locator('.viewer-name')).toHaveText('my checkout');
  await expect(page.getByRole('button', { name: 'Copy link to this view' })).toBeHidden();
});

test('a local file has no link to copy, and the back button returns to the start screen', async ({ page }) => {
  await openFile(page);
  await expect(page.getByRole('button', { name: 'Copy link to this view' })).toBeHidden();
  await page.getByRole('button', { name: 'Open another log' }).click();
  await expectState(page, 'idle');
  await expect(page.locator('main[data-screen="viewer"]')).toHaveCount(0);
  await expect(page.locator('.player-file-input')).toBeFocused();
});

test('the licence, the exact source and the third-party notices are linked on the start screen and beside a log', async ({ page, request }) => {
  const check = async (note: Locator) => {
    const href = (name: string) => note.getByRole('link', { name }).evaluate((a) => (a as HTMLAnchorElement).href);
    expect(await href('Source code')).toMatch(/^https:\/\/github\.com\/.+\/tree\/[^/]+$/);
    expect(await (await request.get(await href('GPL-3.0-or-later'))).text()).toContain('GNU GENERAL PUBLIC LICENSE');
    expect((await request.get(await href('Third-party licences'))).ok()).toBe(true);
  };
  await page.goto(`${o.player}/?mock=1`);
  await check(page.locator('.player-empty .player-licence'));
  await openFile(page);
  await page.getByRole('button', { name: 'Settings' }).click();
  await check(page.locator('#control-panel .player-licence'));
});

test('the AI statistics of a log, in the panel', async ({ page }) => {
  await openFile(page);
  await page.getByRole('button', { name: 'Settings' }).click();
  const section = page.locator('#control-panel [data-section="style"]');
  await section.locator('> summary').click();
  await expect(section.locator('summary')).toHaveText('Contributors & AI');
  await expect(section.locator('tr[data-agent="claude"] .ai-agent-name')).toHaveText('Claude');
  await expect(section.locator('tr[data-agent="claude"] .ai-models').first()).toContainText('Claude Opus 5.5 (21)');
  await expect(section.locator('.ai-caveat')).toHaveText('From commit trailers; not proof, may be undercounted.');
});

test('the recording dialog opens for a log and closes again', async ({ page }) => {
  await openFile(page, '?mock=1&mock-record=1');
  const record = page.getByRole('button', { name: 'Record video' });
  await expect(record).toHaveAttribute('aria-disabled', 'false');
  await record.click();
  const dialog = page.getByRole('dialog', { name: 'Record video' });
  await expect(dialog).toBeVisible();
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
});

test('the player sends nothing: no request leaves the player origin besides the log', async ({ page }) => {
  const outside: string[] = [];
  page.on('request', (request) => {
    if (!request.url().startsWith(o.player) && !request.url().startsWith('data:') && !request.url().startsWith('blob:')) outside.push(request.url());
  });
  await openFile(page, '?mock=1&mock-record=1');
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.locator('#control-panel [data-section="style"] > summary').click();
  await page.locator('select.speed').selectOption('2');
  await page.getByRole('button', { name: 'Record video' }).click();
  await page.keyboard.press('Escape');
  // A log by URL: the only other request is the log itself.
  await page.goto(`${o.player}/?mock=1${fragment({ src: `${o.logs}/quiet.log` })}`);
  await expectState(page, 'playing');
  await page.getByRole('button', { name: 'Settings' }).click();
  await page.locator('#control-panel [data-section="style"] > summary').click();
  expect(outside).toEqual([`${o.logs}/quiet.log`]);
});

test('a view too large for a link keeps the last link that fits, so a reload still plays', async ({ page }) => {
  // Long stored rules (about 1300 characters) are over the protocol's 1024 per option value.
  const rules = Array.from({ length: 60 }, (_, i) => `ai:codex => hat=agent # ${String(i).padStart(2, '0')}`).join('\n');
  await page.addInitScript((value) => localStorage.setItem('gource.ai-options.v1', JSON.stringify({ options: { 'user-style': value } })), rules);
  const link = `${o.player}/?mock=1${fragment({ src: `${o.logs}/too-long.log`, title: 'Long rules' })}`;
  await page.goto(link);
  await expectState(page, 'playing');
  await expect(page.locator('.viewer-banner')).toContainText('does not fit in a link');
  expect(page.url()).toBe(link);
  await page.reload();
  await expectState(page, 'playing');
});

test('back from a log by URL leaves no link to it, and its recording offers no reproduce link', async ({ page }) => {
  await page.goto(`${o.player}/?mock=1&mock-record=1${fragment({ src: `${o.logs}/back.log` })}`);
  await expectState(page, 'playing');
  await expect(page.locator('.viewer-host')).toHaveText('Log');
  await expect(page.locator('.seek')).toBeEnabled();
  await page.evaluate(() => Object.assign(((window as unknown as { __recordTest?: object }).__recordTest ??= {}), { sink: 'blob' }));
  await page.getByRole('button', { name: 'Record video' }).click();
  const dialog = page.locator('.rec-dialog');
  await dialog.locator('input[name="rec-size"][value="1280x720"]').check();
  await dialog.locator('#rec-in-max').fill('2');
  await dialog.locator('.rec-start').click();
  await expect.poll(() => page.evaluate(() => (window as unknown as { __record?: { state(): string } }).__record?.state())).toBe('done');
  await expect(dialog.getByRole('button', { name: 'Download recipe (.json)' })).toBeVisible();
  await expect(dialog.getByRole('button', { name: 'Copy reproduce link' })).toBeHidden();
  await dialog.getByRole('button', { name: 'Close', exact: true }).first().click();
  await page.getByRole('button', { name: 'Open another log' }).click();
  await expectState(page, 'idle');
  expect(new URL(page.url()).hash).toBe('');
});
