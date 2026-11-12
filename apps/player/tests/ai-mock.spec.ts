// SPDX-License-Identifier: GPL-3.0-or-later

// F1 "Contributors & AI" in mock mode (?mock=1): the panel section, its
// options in the URL and in storage, the rules box, the AI summary from the
// mock's ai_stats(), the URL notices and the disabled state (F1 section 8.3).

import { readFileSync } from 'node:fs';

import { expect, test, type Page } from '@playwright/test';

import { SMALL_LOG, engineCalls, playing, setOptionCalls } from './mock-engine';

const SAMPLE = '/samples/ai-coauthors.log';
const SAMPLE_TEXT = readFileSync(new URL('./samples/ai-coauthors.log', import.meta.url), 'utf8');
const OUT = 'test-results/ai';

/** Commits per agent in the sample: the `ai=` field sits on the first line of each credited commit. */
function sampleAgentCounts(): Map<string, number> {
  const counts = new Map<string, number>();
  for (const line of SAMPLE_TEXT.split('\n')) {
    const meta = line.split('|')[5];
    if (!meta?.startsWith('ai=')) continue;
    const agents = new Set(meta.slice(3).split(',').map((c) => c.split(':')[1]!));
    for (const a of agents) counts.set(a, (counts.get(a) ?? 0) + 1);
  }
  return counts;
}

async function openSection(page: Page) {
  await page.getByRole('button', { name: 'Settings' }).click();
  const section = page.locator('#control-panel [data-section="style"]');
  await section.locator('> summary').click();
  await expect(section).toHaveAttribute('open', '');
  return section;
}

async function openSample(page: Page) {
  await page.goto(playing(SAMPLE));
  await expect(page.locator('[data-screen="viewer"]')).toBeVisible();
  // The whole log is loaded (the summary and the disabled state depend on it).
  await expect(page.locator('.seek')).toBeEnabled();
}

test.describe('Contributors & AI (mock)', () => {
  test('section, AI summary and the caveat', async ({ page }) => {
    await openSample(page);
    const section = await openSection(page);
    await expect(section.locator('summary')).toHaveText('Contributors & AI');
    for (const label of ['Show decorations', 'Agent key', 'Built-in look', 'Reduce motion', 'My rules']) {
      await expect(section.getByLabel(label, { exact: true })).toBeVisible();
    }
    await expect(section.getByRole('group', { name: 'AI commits' })).toBeVisible();
    await expect(section.locator('#opt-ai-style option')).toHaveText([
      'Cape + shades (default)',
      'Cape only',
      'Shades only',
      'Glowing hat',
      'None (only my rules)',
    ]);
    await expect(section.locator('#opt-show-decorations')).toBeChecked();
    await expect(section.locator('.ai-no-data')).toBeHidden();

    // Every agent of the log, as text, with its loaded count (AI-61d).
    const counts = sampleAgentCounts();
    const rows = section.locator('table.ai-agents tbody tr[data-agent]');
    await expect(rows).toHaveCount(counts.size);
    for (const [agent, n] of counts) {
      await expect(section.locator(`tr[data-agent="${agent}"] td[data-stat="loaded"]`)).toHaveText(String(n));
    }
    await expect(section.locator('tr[data-agent="claude"] .ai-agent-name')).toHaveText('Claude');
    await expect(section.locator('tr[data-agent="claude"] .ai-models').first()).toContainText('Claude Opus 5.5 (21)');
    await expect(section.locator('tr[data-agent="claude"] .ai-swatch')).toHaveCSS('background-color', 'rgb(230, 159, 0)');
    await expect(section.locator('.ai-caveat')).toHaveText('From commit trailers; not proof, may be undercounted.');

    // Played numbers refresh while the section is open.
    const played = () => section.locator('.ai-totals[data-stat="played"]').textContent();
    const first = await played();
    await page.evaluate(() => window.__gource!.app.seek(0.9));
    await expect.poll(played).not.toBe(first);
    for (const id of ['look', 'playback']) await page.locator(`#control-panel [data-section="${id}"] > summary`).click();
    await page.setViewportSize({ width: 1280, height: 1300 });
    await page.locator('#control-panel').screenshot({ path: `${OUT}/mock-section.png` });
  });

  test('controls call set_option; ai-filter restarts; the URL round-trips through a reload', async ({ page }) => {
    await openSample(page);
    const section = await openSection(page);
    const before = (await setOptionCalls(page)).length;
    const restarts = (await engineCalls(page, 'restart')).length;

    await section.locator('[data-control="decorations"] .switch').click();
    await section.locator('#opt-ai-style').selectOption('hat');
    await section.locator('[data-control="agent-key"] .switch').click();
    await section.locator('label.ai-radio', { hasText: 'Only assisted' }).click();
    await section.locator('[data-control="decor-motion"] .switch').click();

    await expect
      .poll(async () => (await setOptionCalls(page)).slice(before))
      .toEqual([
        ['hide', 'decorations'],
        ['ai-style', 'hat'],
        ['agent-key', 'true'],
        ['ai-filter', 'only-assisted'],
        ['decor-motion', 'off'],
      ]);
    // ai-filter re-simulates at once.
    expect((await engineCalls(page, 'restart')).length).toBe(restarts + 1);
    // The Hide chips of "Visibility & labels" show the same state.
    await expect(page.locator('#control-panel input[data-hide="decorations"]')).toBeChecked();

    await expect(page).toHaveURL(/decor-motion=off/);
    const url = page.url();
    expect(url).toContain('hide=decorations');
    for (const part of ['ai-style=hat', 'agent-key=true', 'ai-filter=only-assisted', 'decor-motion=off']) expect(url).toContain(part);

    await page.reload();
    await expect(page.locator('[data-screen="viewer"]')).toBeVisible();
    const applied = await setOptionCalls(page);
    for (const call of [
      ['hide', 'decorations'],
      ['ai-style', 'hat'],
      ['agent-key', 'true'],
      ['ai-filter', 'only-assisted'],
      ['decor-motion', 'off'],
    ]) {
      expect(applied).toContainEqual(call);
    }
    const again = await openSection(page);
    await expect(again.locator('#opt-show-decorations')).not.toBeChecked();
    await expect(again.locator('#opt-ai-style')).toHaveValue('hat');
    await expect(again.locator('#opt-agent-key')).toBeChecked();
    await expect(again.locator('input[name="ai-filter"][value="only-assisted"]')).toBeChecked();
    await expect(again.locator('#opt-decor-motion')).toBeChecked();
  });

  test('the AI filter radios work from the keyboard', async ({ page }) => {
    await openSample(page);
    const section = await openSection(page);
    const all = section.locator('input[name="ai-filter"][value="all"]');
    await all.focus();
    await page.keyboard.press('ArrowRight');
    await expect(section.locator('input[name="ai-filter"][value="only-assisted"]')).toBeChecked();
    await expect.poll(async () => (await setOptionCalls(page)).at(-1)).toEqual(['ai-filter', 'only-assisted']);
  });

  test('rules: presets, inline errors that keep the previous rules, the long-link hint', async ({ page }) => {
    await openSample(page);
    const section = await openSection(page);
    const rules = section.locator('#opt-user-style');
    const error = section.locator('#opt-user-style-error');

    await section.locator('#opt-user-style-preset').selectOption('glowing-hats');
    await expect(rules).toHaveValue('assisted => cape=off; shades=off; hat=agent; glow=agent');
    await expect.poll(async () => (await setOptionCalls(page)).at(-1)).toEqual(['user-style', 'assisted => cape=off; shades=off; hat=agent; glow=agent']);
    await expect(page).toHaveURL(/user-style=assisted/);

    // A broken second line: coded error with its line, nothing applied (US-6).
    const applied = (await setOptionCalls(page)).length;
    await rules.fill('ai:codex => hat=agent\nthis has no arrow');
    await expect(error).toHaveText('Line 2: style_missing_arrow: Rule needs a selector, => and properties.');
    await expect(error).toHaveAttribute('data-code', 'style_missing_arrow');
    await expect(rules).toHaveAttribute('aria-invalid', 'true');
    await expect(section.locator('#opt-user-style-preset')).toHaveValue('');
    await page.waitForTimeout(400);
    expect((await setOptionCalls(page)).length).toBe(applied);
    expect((await engineCalls(page, 'check_style')).at(-1)?.args).toEqual(['ai:codex => hat=agent\nthis has no arrow']);

    // Fixed: applied, error gone.
    await rules.fill('ai:codex => hat=agent\n# comment\nhuman => shades=on');
    await expect(error).toBeEmpty();
    await expect(rules).not.toHaveAttribute('aria-invalid', 'true');
    await expect.poll(async () => (await setOptionCalls(page)).at(-1)).toEqual(['user-style', 'ai:codex => hat=agent\n# comment\nhuman => shades=on']);

    // Over SHARE_URL_SOFT_LIMIT: a hint, the link itself unchanged.
    const hint = section.locator('.ai-length-hint');
    await expect(hint).toBeHidden();
    const long = Array.from({ length: 24 }, (_, i) => `ai:agent${i} => hat=agent; glow=agent; cape=off; shades=off; halo=off; badge=on`).join('\n');
    await rules.fill(long);
    await expect(hint).toBeVisible();
    await expect(hint).toContainText('2,000');
    await page.screenshot({ path: `${OUT}/mock-rules.png` });
    await section.locator('#opt-user-style-preset').selectOption('hero');
    await expect(hint).toBeHidden();
  });

  test('stored rules are restored on any log, and a link overrides them with a notice (US-15, US-16)', async ({ page }) => {
    await openSample(page);
    const section = await openSection(page);
    await section.locator('#opt-user-style').fill('human => hat=on');
    await expect.poll(async () => (await setOptionCalls(page)).at(-1)).toEqual(['user-style', 'human => hat=on']);
    await section.locator('[data-control="agent-key"] .switch').click();
    await expect.poll(async () => (await setOptionCalls(page)).at(-1)).toEqual(['agent-key', 'true']);

    // A fresh link without AI options: the stored ones apply, no notice.
    await page.route((url) => url.pathname === '/samples/other.log', (route) => route.fulfill({ status: 200, contentType: 'text/plain', body: SMALL_LOG }));
    await page.goto(playing('/samples/other.log'));
    await expect(page.locator('[data-screen="viewer"]')).toBeVisible();
    expect(await setOptionCalls(page)).toEqual(expect.arrayContaining([['user-style', 'human => hat=on'], ['agent-key', 'true']]));
    await expect(page).toHaveURL(/user-style=human/);
    await expect(page.locator('.ai-notice')).toHaveCount(0);

    // A link with its own rules and filter wins, and says so.
    // A new page load: on this page only the fragment would change, and the engine's call log would carry over.
    await page.goto('about:blank');
    await page.goto(playing(SAMPLE, { 'user-style': 'ai => glow=agent', 'ai-filter': 'exclude-assisted' }));
    await expect(page.locator('[data-screen="viewer"]')).toBeVisible();
    const calls = await setOptionCalls(page);
    expect(calls).toContainEqual(['user-style', 'ai => glow=agent']);
    expect(calls).not.toContainEqual(['user-style', 'human => hat=on']);
    expect(calls).toContainEqual(['agent-key', 'true']);
    const notices = page.locator('.ai-notice');
    await expect(notices).toHaveCount(2);
    await expect(notices.nth(0)).toHaveText('This link applies custom styling.');
    await expect(notices.nth(1)).toHaveText('This link filters AI commits.');
    await page.screenshot({ path: `${OUT}/mock-notices.png` });
    await notices.nth(0).getByRole('button', { name: 'Dismiss notice' }).click();
    await expect(notices).toHaveCount(1);

    // The link did not overwrite the viewer's own stored rules.
    const stored = await page.evaluate(() => window.localStorage.getItem('gource.ai-options.v1'));
    expect(JSON.parse(stored!).options['user-style']).toBe('human => hat=on');
  });

  test('decor-motion is set once from prefers-reduced-motion', async ({ page }) => {
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await openSample(page);
    expect(await setOptionCalls(page)).toContainEqual(['decor-motion', 'off']);
    await expect(page).toHaveURL(/decor-motion=off/);
    // A URL value wins over the OS preference.
    await page.goto('about:blank');
    await page.goto(playing(SAMPLE, { 'decor-motion': 'full' }));
    await expect(page.locator('[data-screen="viewer"]')).toBeVisible();
    expect(await setOptionCalls(page)).not.toContainEqual(['decor-motion', 'off']);
  });

  test('a log without attribution data disables the AI controls (AI-29)', async ({ page }) => {
    await page.route((url) => url.pathname === '/samples/plain.log', (route) => route.fulfill({ status: 200, contentType: 'text/plain', body: SMALL_LOG }));
    await page.goto(playing('/samples/plain.log'));
    await expect(page.locator('[data-screen="viewer"]')).toBeVisible();
    await expect(page.locator('.seek')).toBeEnabled();
    const section = await openSection(page);
    await expect(section.locator('.ai-no-data')).toHaveText('This log has no AI attribution data (detection off or log predates it).');
    await expect(section.locator('fieldset.ai-group')).toHaveAttribute('disabled', '');
    await expect(section.locator('input[name="ai-filter"][value="only-assisted"]')).toBeDisabled();
    await expect(section.locator('table.ai-agents')).toHaveCount(0);
    // Decorations, motion and the rules box stay usable (rules can match `human`).
    await expect(section.locator('#opt-user-style')).toBeEnabled();
    await expect(section.locator('#opt-show-decorations')).toBeEnabled();
    await page.screenshot({ path: `${OUT}/mock-no-data.png` });
  });
});
