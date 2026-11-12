// SPDX-License-Identifier: GPL-3.0-or-later

// RPA-4: actual recording-dialog DOM with the existing mock recording backend.
// The mock's future-date empty range exercises UI recovery, not native seed detection.
import { expect, test, type Page } from '@playwright/test';

import type { RecordFactory } from 'gource-player/record/types';
import { engineCalls, playing } from './mock-engine';

/** The page's own recording mock module, as the Vite dev server serves it (the app imports the same one); a wrong path fails the import loudly. */
const MOCK_MODULE = '/src/record/mock.ts';

declare global {
  interface Window {
    __emptyRangePlans?: Array<Record<string, string>>;
  }
}

async function openDialog(page: Page, fault?: 'record_empty_run' | 'record_invalid_option') {
  await page.goto(playing('/samples/ai-coauthors.log', {}, 'mock=1&mock-record=1'));
  await expect(page.locator('[data-screen="viewer"]')).toBeVisible();
  await expect(page.locator('.record-btn')).toHaveAttribute('aria-disabled', 'false');
  // Test-local observation/fault injection at the existing factory seam. The real dialog,
  // form events and planner requests remain in use, with no production mock edits.
  await page.evaluate(async ({ code, path }) => {
    const modulePath = path;
    const { mockRecordFactory } = await import(modulePath) as { mockRecordFactory: RecordFactory };
    const plan = mockRecordFactory.plan;
    window.__emptyRangePlans = [];
    mockRecordFactory.plan = (app, json) => {
      const options = JSON.parse(json) as Record<string, string>;
      window.__emptyRangePlans!.push(options);
      if (code && options['start-date']?.startsWith('2150')) {
        // Identical prose under a different code must not turn a generic error into empty-range recovery.
        throw new Error(`${code}: No activity in this range`);
      }
      return plan(app, json);
    };
  }, { code: fault, path: MOCK_MODULE });
  await page.locator('.record-btn').click();
  const dialog = page.getByRole('dialog', { name: 'Record video' });
  await expect(dialog).toBeVisible();
  await expect(dialog.locator('.rec-encoder')).toContainText('H.264 High');
  return dialog;
}

async function emptyDates(page: Page) {
  const dialog = page.getByRole('dialog', { name: 'Record video' });
  await dialog.getByRole('radio', { name: 'Dates', exact: true }).check();
  await dialog.locator('#rec-in-startDate').fill('2150-01-01');
  await dialog.locator('#rec-in-stopDate').fill('2150-01-02');
  await expect.poll(() => page.evaluate(() => window.__emptyRangePlans?.at(-1)?.['stop-date'])).toBe('2150-01-02');
  await expect(dialog.locator('.rec-start')).toBeDisabled();
  return dialog;
}

const noExport = async (page: Page) => {
  expect(await page.evaluate(() => ({
    state: window.__record?.state(),
    frames: window.__record?.frames(),
    bytes: window.__record?.sinkBytes(),
    output: window.__record?.output() !== null,
  }))).toEqual({ state: 'configuring', frames: 0, bytes: 0, output: false });
};

test('RPA-4: empty estimate disables Record and offers exact activity-neutral copy', async ({ page }, testInfo) => {
  const dialog = await openDialog(page);
  await emptyDates(page);
  await expect(dialog.locator('.rec-estimate')).toHaveText('No activity in this range');
  await expect(dialog.locator('.rec-start')).toHaveAttribute('title', 'No activity in this range');
  await expect(dialog.getByRole('button', { name: 'Use entire history', exact: true })).toBeVisible();
  await dialog.locator('.rec-estimate').scrollIntoViewIfNeeded();
  await expect(dialog.getByRole('button', { name: 'Use entire history', exact: true })).toBeInViewport();
  await dialog.screenshot({ path: testInfo.outputPath('empty-range.png') });
  await noExport(page);
});

test('RPA-4: keyboard widening clears recording range options and replans without exporting', async ({ page }) => {
  const dialog = await openDialog(page);
  await dialog.getByRole('radio', { name: '25 fps', exact: true }).check();
  await dialog.locator('#rec-in-spd').fill('4');
  await dialog.locator('#rec-in-tail').fill('9');
  await emptyDates(page);
  const callsBefore = (await engineCalls(page, 'set_option')).length;
  const widen = dialog.getByRole('button', { name: 'Use entire history', exact: true });
  await widen.focus();
  await expect(widen).toBeFocused();
  await page.keyboard.press('Enter');
  await expect(dialog.getByRole('radio', { name: 'Whole history', exact: true })).toBeChecked();
  await expect(dialog.getByRole('radio', { name: 'Whole history', exact: true })).toBeFocused();
  await expect(dialog.locator('#rec-in-startDate')).toHaveValue('');
  await expect(dialog.locator('#rec-in-stopDate')).toHaveValue('');
  await expect(dialog.locator('.rec-dates')).toBeHidden();
  await expect(dialog.locator('.rec-start')).toBeEnabled();
  await expect(widen).toBeHidden();
  await expect(dialog.getByRole('radio', { name: '25 fps', exact: true })).toBeChecked();
  await expect(dialog.locator('#rec-in-spd')).toHaveValue('4');
  await expect(dialog.locator('#rec-in-tail')).toHaveValue('9');
  expect(await page.evaluate(() => window.__emptyRangePlans?.at(-1))).toMatchObject({
    'start-position': '', 'start-date': '', 'stop-date': '',
    'record-framerate': '25', 'seconds-per-day': '4', 'record-tail-seconds': '9',
  });
  expect((await engineCalls(page, 'set_option')).length).toBe(callsBefore);
  await noExport(page);
});

test('RPA-4: typed record_empty_run uses the same recovery branch despite diagnostic prose', async ({ page }) => {
  const dialog = await openDialog(page, 'record_empty_run');
  await emptyDates(page);
  await expect(dialog.locator('.rec-estimate')).toHaveText('No activity in this range');
  await dialog.getByRole('button', { name: 'Use entire history', exact: true }).click();
  await expect(dialog.getByRole('radio', { name: 'Whole history', exact: true })).toBeChecked();
  await expect(dialog.locator('.rec-start')).toBeEnabled();
  await noExport(page);
});

test('RPA-4: entire history clears a supplied recipe stop position while preserving unrelated extras', async ({ page }) => {
  const dialog = await openDialog(page);
  const recipe = {
    v: 1,
    source: { kind: 'url', path: '/samples/ai-coauthors.log' },
    options: {
      'start-date': '2150-01-01', 'stop-date': '2150-01-02',
      'stop-position': '0.25', 'seconds-per-day': '4',
      'hash-seed': '31', 'bloom-intensity': '0.5',
    },
    record: { 'record-framerate': '25', 'record-tail-seconds': '9' },
  };
  await dialog.locator('input[type=file]').setInputFiles({
    name: 'bounded.recipe.json', mimeType: 'application/json', buffer: Buffer.from(JSON.stringify(recipe)),
  });
  await expect(dialog).toContainText('Loaded the recipe bounded.recipe.json');
  await expect.poll(() => page.evaluate(() => window.__emptyRangePlans?.at(-1)?.['stop-position'])).toBe('0.25');
  await expect(dialog.locator('.rec-estimate')).toHaveText('No activity in this range');
  await expect(dialog.locator('.rec-start')).toBeDisabled();
  const callsBefore = (await engineCalls(page, 'set_option')).length;
  await noExport(page);

  await dialog.getByRole('button', { name: 'Use entire history', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__emptyRangePlans?.at(-1)?.['stop-position'])).toBe('');
  expect(await page.evaluate(() => window.__emptyRangePlans?.at(-1))).toMatchObject({
    'start-position': '', 'stop-position': '', 'start-date': '', 'stop-date': '',
    'hash-seed': '31', 'bloom-intensity': '0.5', 'seconds-per-day': '4',
    'record-framerate': '25', 'record-tail-seconds': '9',
  });
  await expect(dialog.getByRole('radio', { name: 'Whole history', exact: true })).toBeChecked();
  await expect(dialog.locator('.rec-start')).toBeEnabled();
  expect((await engineCalls(page, 'set_option')).length).toBe(callsBefore);
  await noExport(page);

  // Supplied recording settings still require confirmation, never automatic export.
  await dialog.locator('.rec-start').click();
  const confirm = page.getByRole('dialog', { name: 'Confirm recording' });
  await expect(confirm).toBeVisible();
  await expect(confirm.locator('.rec-supplied-list')).toContainText('record-framerate=25');
  await expect(confirm.locator('.rec-supplied-list')).not.toContainText('stop-position=');
  expect(await page.evaluate(() => ({
    state: window.__record?.state(), frames: window.__record?.frames(),
    bytes: window.__record?.sinkBytes(), output: window.__record?.output() !== null,
  }))).toEqual({ state: 'confirm', frames: 0, bytes: 0, output: false });
  await confirm.getByRole('button', { name: 'Back', exact: true }).click();
  await noExport(page);
});

test('RPA-4: identical free text under a nonempty error code does not offer range widening', async ({ page }) => {
  const dialog = await openDialog(page, 'record_invalid_option');
  await emptyDates(page);
  await expect(dialog.locator('.rec-estimate')).toHaveText('No activity in this range');
  await expect(dialog.getByRole('button', { name: 'Use entire history', exact: true })).toBeHidden();
  await noExport(page);
});

test('RPA-4: recording PR visibility has an accessible label and only changes recording options', async ({ page }) => {
  const dialog = await openDialog(page);
  const callsBefore = (await engineCalls(page, 'set_option')).length;
  const showPullRequests = dialog.getByRole('checkbox', { name: 'Pull requests', exact: true });
  await expect(showPullRequests).toBeChecked();
  await showPullRequests.uncheck();
  await expect.poll(() => page.evaluate(() => window.__emptyRangePlans?.at(-1)?.['hide']?.split(','))).toContain('pull-requests');
  expect((await engineCalls(page, 'set_option')).length).toBe(callsBefore);
  await noExport(page);
});
