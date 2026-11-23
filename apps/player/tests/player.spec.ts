// SPDX-License-Identifier: GPL-3.0-or-later

// The built player on real HTTP origins (./servers.ts): the CSP, CORS and
// cross-origin postMessage are the browser's own. The placeholder engine
// (?mock=1, test builds only) keeps it fast; player-real.spec.ts runs the engine.

import { expect, test } from '@playwright/test';
import { readFileSync } from 'node:fs';
import { embed, expectState, fragment, got, gotEvents, gotTypes, handOver, mockCalls, openPlayer, startOrigins, type Origins, type Request } from './servers';

let o: Origins;
let requests: Request[];
let stop: () => Promise<void>;
test.beforeAll(async () => ({ origins: o, requests, stop } = await startOrigins()));
test.afterAll(async () => stop());

const reads = (path: string) => requests.filter((r) => r.path === path);

test.describe('standalone, by URL', () => {
  test('plays a public log with its title and options; the fetch sends no cookie and no referrer', async ({ page, context }) => {
    await context.addCookies([{ name: 'session', value: 'secret', domain: '127.0.0.1', path: '/' }]);
    // The same-origin read shows `credentials: 'omit'` (the default would send the cookie). `keep-referrer` drops the
    // page's Referrer-Policy header, so only the fetch's own `referrerPolicy` keeps the referrer out.
    for (const src of [`${o.logs}/url-a.log`, '/same-url.log']) {
      await page.goto(`${o.player}/?mock=1&keep-referrer=1${fragment({ src, title: 'Example repo', 'o.seconds-per-day': '2' })}`);
      await expectState(page, 'playing');
      await expect(page.locator('.viewer-name')).toHaveText('Example repo');
      await expect(page).toHaveTitle('Example repo · Gource player');
      await expect(page.locator('[data-stat="commits"]')).toContainText('/ 3 commits');
      expect(await mockCalls(page)).toContain('set_option(seconds-per-day,2)');
      await page.goto('about:blank');
    }
    const fetched = [...reads('logs:/url-a.log'), ...reads('player:/same-url.log')];
    expect(fetched).toHaveLength(2);
    for (const read of fetched) expect(read).toMatchObject({ cookie: undefined, referer: undefined });
  });

  test('a new fragment on the same page plays that log; a bad one does not stop the one playing', async ({ page }) => {
    await page.goto(`${o.player}/?mock=1${fragment({ src: `${o.logs}/hash-1.log`, title: 'First' })}`);
    await expectState(page, 'playing');
    await page.evaluate((hash) => { window.location.hash = hash; }, fragment({ src: `${o.logs}/hash-2.log`, title: 'Second' }));
    await expect(page.locator('.viewer-name')).toHaveText('Second');
    await expect.poll(() => reads('logs:/hash-2.log').length).toBe(1);
    await page.evaluate(() => { window.location.hash = '#nonsense=1'; });
    await expect(page.locator('.viewer-banner')).toContainText('This player link is not valid');
    await expectState(page, 'playing');
    // The address goes back to the log that plays, so a reload or a copied link still works.
    expect(new URLSearchParams(new URL(page.url()).hash.slice(1)).get('src')).toBe(`${o.logs}/hash-2.log`);
    await expect(page.locator('gource-viewer')).toHaveCount(1);
  });

  test('refuses a source that is not allowed, and any other key, before fetching anything', async ({ page }) => {
    const bads: Array<Record<string, string>> = [
      { src: `${o.hostile}/blocked.log` },
      { src: `${o.logs}/refused.log`, extra: '1' },
      { src: `${o.logs}/refused.log`, 'o.Bad': '1' },
      { title: 'no src' },
    ];
    for (const bad of bads) {
      await page.goto(`${o.player}/?mock=1${fragment(bad)}`);
      await expect(page.locator('.player-banner')).toContainText('This player link is not valid');
      await expectState(page, 'failed');
      await page.goto('about:blank');
    }
    expect(reads('hostile:/blocked.log')).toHaveLength(0);
    expect(reads('logs:/refused.log')).toHaveLength(0);
  });

  test('a log that cannot be fetched leaves no player controls, and offers a local file', async ({ page }) => {
    // The redirect goes to an allowed source with CORS, so only `redirect: 'error'` refuses it.
    for (const src of [`${o.logs}/redirect-1.log`, `${o.logs}/no-cors.log`]) {
      await page.goto(`${o.player}/?mock=1${fragment({ src })}`);
      await expect(page.locator('.player-banner')).toContainText('Could not download the log');
      await expectState(page, 'failed');
      await expect(page.locator('main[data-screen="viewer"]')).toHaveCount(0);
      await expect(page.locator('.player-status')).toHaveText('');
      await expect(page.locator('.player-empty')).toBeVisible();
      await expect(page.locator('gource-viewer')).toHaveCount(0);
      await page.goto('about:blank');
    }
    expect(reads('logs:/redirect-1.log')).toHaveLength(1);
    expect(reads('logs:/redirected-1.log')).toHaveLength(0);
  });
});

test.describe('by postMessage', () => {
  test('embedded: an allowed page hands over a log and gets ready, then ack; focus stays on the page', async ({ page }) => {
    await page.goto(`${o.sender}/page.html`);
    await embed(page, `${o.player}/?mock=1`);
    await expect.poll(() => gotTypes(page)).toEqual(['ready']);
    expect((await got(page))[0]).toMatchObject({ origin: o.player, fromPlayer: true });
    const frame = page.frameLocator('iframe.player');
    await expect(frame.locator('main.player')).toHaveAttribute('data-state', 'waiting');
    // While it waits, the start screen stays hidden (the log is on its way), but the person can still open a local file.
    await expect(frame.locator('.player-empty')).toBeHidden();
    await expect(frame.locator('.player-bar .player-open')).toBeVisible();
    await handOver(page, o.player);
    await expect.poll(() => gotTypes(page)).toEqual(['ready', 'ack']);
    await expect(frame.locator('main.player')).toHaveAttribute('data-state', 'playing');
    await expect(frame.locator('.viewer-name')).toHaveText('Handed over');
    await expect(frame.locator('[data-stat="commits"]')).toContainText('/ 3 commits');
    expect(await page.evaluate(() => document.activeElement?.tagName)).toBe('BODY');
  });

  test('new window: the page that opened the player, under the hosted sender\'s COOP, hands over a log', async ({ page }) => {
    await page.goto(`${o.sender}/page.html`);
    const opened = await openPlayer(page, `${o.player}/?mock=1`);
    await expect.poll(() => gotTypes(page)).toEqual(['ready']);
    await handOver(page, o.player);
    await expect.poll(() => gotTypes(page)).toEqual(['ready', 'ack']);
    await expectState(opened, 'playing');
    expect(await mockCalls(opened)).toContain('set_option(seconds-per-day,3)');
  });

  test('a handed-over log leaves no trace: its name is not in the tab title, the opener is cut, its settings stay out of storage', async ({ page }) => {
    await page.goto(`${o.sender}/page.html`);
    const opened = await openPlayer(page, `${o.player}/?mock=1`);
    await expect.poll(() => gotTypes(page)).toEqual(['ready']);
    expect(await opened.evaluate(() => window.opener !== null)).toBe(true);
    await handOver(page, o.player);
    await expectState(opened, 'playing');
    await expect(opened.locator('.viewer-name')).toHaveText('Handed over');
    await expect(opened).toHaveTitle('Gource player');
    expect(await opened.evaluate(() => window.opener)).toBeNull();

    // A setting changed now works for this log and is gone from storage, also after it.
    await opened.getByRole('button', { name: 'Settings' }).click();
    await opened.locator('#control-panel [data-section="style"] > summary').click();
    await opened.locator('#opt-user-style-preset').selectOption('glowing-hats');
    await expect.poll(() => mockCalls(opened)).toContainEqual(expect.stringContaining('set_option(user-style,assisted'));
    expect(await opened.evaluate(() => localStorage.getItem('gource.ai-options.v1'))).toBeNull();
  });

  test('a log by URL keeps its name in the tab title and its settings in storage', async ({ page }) => {
    await page.goto(`${o.player}/?mock=1${fragment({ src: `${o.logs}/stored.log`, title: 'Public repo' })}`);
    await expectState(page, 'playing');
    await expect(page).toHaveTitle('Public repo · Gource player');
    await page.getByRole('button', { name: 'Settings' }).click();
    await page.locator('#control-panel [data-section="style"] > summary').click();
    await page.locator('#opt-user-style-preset').selectOption('glowing-hats');
    await expect.poll(() => page.evaluate(() => localStorage.getItem('gource.ai-options.v1'))).toContain('assisted');
  });

  test('only the window that opened the player counts: a frame of the same page is ignored', async ({ page }) => {
    await page.goto(`${o.sender}/page.html`);
    const opened = await openPlayer(page, `${o.player}/?mock=1`);
    await expect.poll(() => gotTypes(page)).toEqual(['ready']);
    // Same origin, a valid handoff, the right target window, but not the opener as event.source.
    await page.evaluate(() => {
      const frame = document.createElement('iframe');
      frame.className = 'child';
      frame.src = '/plain.html';
      document.body.append(frame);
      return new Promise((done) => frame.addEventListener('load', done));
    });
    await page.evaluate(({ log, origin }) => {
      const child = document.querySelector<HTMLIFrameElement>('iframe.child')!.contentWindow as Window & { eval(code: string): unknown };
      child.eval(`(() => {
        const w = parent.player, bytes = new TextEncoder().encode(${JSON.stringify(log)}), e = { protocol: 'gource-player', version: 1 }, id = 'from-frame';
        w.postMessage({ ...e, type: 'begin', id, bytes: bytes.byteLength, chunk_bytes: 65536, chunks: 1, options: {} }, ${JSON.stringify(origin)});
        w.postMessage({ ...e, type: 'chunk', id, seq: 0, data: bytes.buffer }, ${JSON.stringify(origin)}, [bytes.buffer]);
        w.postMessage({ ...e, type: 'end', id }, ${JSON.stringify(origin)});
      })()`);
    }, { log: '1704067200|Frame|A|/frame\n', origin: o.player });
    await handOver(page, o.player, { id: 'after-frame' });
    await expect.poll(() => gotTypes(page)).toEqual(['ready', 'ack']);
    expect((await got(page))[1]).toMatchObject({ id: 'after-frame' });
    await expectState(opened, 'playing');
    await expect(opened.locator('[data-stat="commits"]')).toContainText('/ 3 commits');
  });

  test('the first sender to speak fixes the origin: the same window from another allowed origin is ignored', async ({ page }) => {
    await page.goto(`${o.sender}/plain.html`);
    const opened = await openPlayer(page, `${o.player}/?mock=1`);
    await expect.poll(() => gotTypes(page)).toEqual(['ready']);
    await handOver(page, o.player, { parts: ['begin'] });
    // The opener's window moves to the other allowed origin and finishes the handoff from there.
    await page.goto(`${o.sender2}/plain.html`);
    await page.evaluate(() => { (window as unknown as { player: Window | null }).player = window.open('', 'gplayer'); });
    await handOver(page, o.player, { parts: ['chunk', 'end'] });
    // A file opened afterwards proves the player was still waiting, not playing that log.
    await opened.locator('.player-file-input').setInputFiles({ name: 'mine.log', mimeType: 'text/plain', buffer: Buffer.from('1704067200|Me|A|/x\n') });
    await expect(opened.locator('.viewer-name')).toHaveText('mine.log');
    expect(await gotTypes(page)).toEqual([]);
  });

  test('a bad link while waiting for a handoff only shows a notice; the handoff still arrives', async ({ page }) => {
    await page.goto(`${o.sender}/page.html`);
    const opened = await openPlayer(page, `${o.player}/?mock=1`);
    await expect.poll(() => gotTypes(page)).toEqual(['ready']);
    await opened.evaluate(() => { window.location.hash = '#nonsense=1'; });
    await expect(opened.locator('.player-banner')).toContainText('This player link is not valid');
    await expectState(opened, 'waiting');
    expect(new URL(opened.url()).hash).toBe('');
    await handOver(page, o.player);
    await expect.poll(() => gotTypes(page)).toEqual(['ready', 'ack']);
    await expectState(opened, 'playing');
  });

  test('a message out of order fails the handoff and is answered with an error', async ({ page }) => {
    await page.goto(`${o.sender}/page.html`);
    await embed(page, `${o.player}/?mock=1`);
    await expect.poll(() => gotTypes(page)).toEqual(['ready']);
    await handOver(page, o.player, { parts: ['chunk', 'begin', 'end'] });
    await expect.poll(() => gotTypes(page)).toEqual(['ready', 'error:order']);
    const frame = page.frameLocator('iframe.player');
    await expect(frame.locator('.player-banner')).toContainText('did not arrive in order');
    await expect(frame.locator('main.player')).toHaveAttribute('data-state', 'failed');
  });

  test('a handed-over log records as possibly private, with a warning', async ({ page }) => {
    await page.goto(`${o.sender}/page.html`);
    const opened = await openPlayer(page, `${o.player}/?mock=1&mock-record=1`);
    await expect.poll(() => gotTypes(page)).toEqual(['ready']);
    await handOver(page, o.player);
    await expectState(opened, 'playing');
    await opened.locator('.record-btn').click();
    const dialog = opened.getByRole('dialog', { name: 'Record video' });
    await expect(dialog.getByText('This log may come from a private repository: the video shows its names and paths.')).toBeVisible();
  });

  test('a local file records as before: no private warning', async ({ page }) => {
    await page.goto(`${o.player}/?mock=1&mock-record=1`);
    await page.locator('.player-file-input').setInputFiles({ name: 'mine.log', mimeType: 'text/plain', buffer: Buffer.from('1704067200|Me|A|/x\n') });
    await expectState(page, 'playing');
    await page.locator('.record-btn').click();
    const dialog = page.getByRole('dialog', { name: 'Record video' });
    // The dialog is drawn first, so the absence below is not just an empty dialog.
    await expect(dialog.getByRole('button', { name: 'Save preset' })).toBeVisible();
    await expect(dialog.locator('.rec-warn')).toHaveCount(0);
  });

  test('a handoff that never finishes times out: the sender hears it and the opener is cut', async ({ page, context }) => {
    await context.clock.install();
    await page.goto(`${o.sender}/page.html`);
    const opened = await openPlayer(page, `${o.player}/?mock=1`);
    await expect.poll(() => gotTypes(page)).toEqual(['ready']);
    await handOver(page, o.player, { parts: ['begin'] });
    await opened.clock.runFor(61_000);
    await expect.poll(() => gotTypes(page)).toEqual(['ready', 'error:timeout']);
    expect(await opened.evaluate(() => window.opener)).toBeNull();
    await expectState(opened, 'failed');
  });

  test('a failed handoff cuts the opener too', async ({ page }) => {
    await page.goto(`${o.sender}/page.html`);
    const opened = await openPlayer(page, `${o.player}/?mock=1`);
    await expect.poll(() => gotTypes(page)).toEqual(['ready']);
    await handOver(page, o.player, { parts: ['chunk', 'begin', 'end'] });
    await expect.poll(() => gotTypes(page)).toEqual(['ready', 'error:order']);
    expect(await opened.evaluate(() => window.opener)).toBeNull();
  });

  test('opening a local file while a handoff is under way tells the sender it was cancelled', async ({ page }) => {
    await page.goto(`${o.sender}/page.html`);
    const opened = await openPlayer(page, `${o.player}/?mock=1`);
    await expect.poll(() => gotTypes(page)).toEqual(['ready']);
    await handOver(page, o.player, { parts: ['begin'] });
    await opened.locator('.player-file-input').setInputFiles({ name: 'mine.log', mimeType: 'text/plain', buffer: Buffer.from('1704067200|Me|A|/x\n') });
    await expect.poll(() => gotTypes(page)).toEqual(['ready', 'error:cancelled']);
    expect((await got(page))[1]).toMatchObject({ id: 'handoff-1' });
    await expect(opened.locator('.viewer-name')).toHaveText('mine.log');
    expect(await opened.evaluate(() => window.opener)).toBeNull();
    // The rest of the cancelled handoff changes nothing.
    await handOver(page, o.player, { parts: ['chunk', 'end'] });
    await expect(opened.locator('[data-stat="commits"]')).toContainText('/ 1 commits');
    expect(await gotTypes(page)).toEqual(['ready', 'error:cancelled']);
  });

  test('a page from another origin can neither embed the player nor hand it a log, and hears nothing', async ({ page }) => {
    await page.goto(`${o.hostile}/page.html`);
    // frame-ancestors lists only the senders.
    const blocked = page.waitForEvent('console', (m) => /frame-ancestors/.test(m.text()));
    await embed(page, `${o.player}/?mock=1`);
    await blocked;
    await page.locator('iframe.player').evaluate((frame) => frame.remove());
    // It can open one, but `ready` goes only to the senders, and its messages are ignored.
    const opened = await openPlayer(page, `${o.player}/?mock=1`);
    await expectState(opened, 'waiting');
    await handOver(page, o.player);
    await opened.locator('.player-file-input').setInputFiles({ name: 'mine.log', mimeType: 'text/plain', buffer: Buffer.from('1704067200|Me|A|/x\n') });
    await expect(opened.locator('.viewer-name')).toHaveText('mine.log');
    // Its events go to the senders' origins only, so this page hears none of them either.
    await opened.getByRole('button', { name: 'Open another log' }).click();
    await expectState(opened, 'idle');
    expect(await got(page)).toEqual([]);
  });
});

test.describe('events for the page that opened or embeds the player', () => {
  test('a handed-over log: the sender hears it open, start and end, with counts only, after the opener is cut', async ({ page, context }) => {
    await context.clock.install();
    await page.goto(`${o.sender}/page.html`);
    const opened = await openPlayer(page, `${o.player}/?mock=1`);
    await expect.poll(() => gotTypes(page)).toEqual(['ready']);
    await handOver(page, o.player);
    await expectState(opened, 'playing');
    expect(await opened.evaluate(() => window.opener)).toBeNull();
    await expect.poll(() => gotEvents(page)).toEqual([
      { event: 'log_opened', props: { source: 'message', log_size: '<10KB', commits: 3 }, origin: o.player, fromPlayer: true },
    ]);
    await opened.clock.runFor(5_500);
    await expect.poll(async () => (await gotEvents(page)).map((e) => e.event)).toEqual(['log_opened', 'playback_started']);
    const started = (await gotEvents(page))[1]!.props!;
    expect(started.first_frame_ms).toEqual(expect.any(Number));
    expect(Object.keys(started).filter((key) => key !== 'first_frame_ms' && key !== 'fps_p50_5s')).toEqual([]);
    await opened.clock.runFor(2_000);
    await opened.getByRole('button', { name: 'Open another log' }).click();
    await expect.poll(async () => (await gotEvents(page)).map((e) => e.event)).toEqual(['log_opened', 'playback_started', 'playback_ended']);
    expect((await gotEvents(page))[2]!.props).toEqual({ watched_s: expect.any(Number) });
    // The handoff's own messages are unchanged.
    expect(await gotTypes(page)).toEqual(['ready', 'ack']);
  });

  test('a log by URL from a sender\'s window: the opener is cut at once, and the sender still hears playback', async ({ page }) => {
    await page.goto(`${o.sender}/page.html`);
    const opened = await openPlayer(page, `${o.player}/?mock=1${fragment({ src: `${o.logs}/events.log`, title: 'Public repo' })}`);
    await expectState(opened, 'playing');
    expect(await opened.evaluate(() => window.opener)).toBeNull();
    await expect.poll(() => gotEvents(page)).toEqual([
      { event: 'log_opened', props: { source: 'url', log_size: '<10KB', commits: 3 }, origin: o.player, fromPlayer: true },
    ]);
    // A page closing ends playback too.
    await opened.goto('about:blank');
    await expect.poll(async () => (await gotEvents(page)).map((e) => e.event)).toContain('playback_ended');
    expect(await gotTypes(page)).toEqual([]);
  });

  test('embedded: the page around the player hears a local file open, never its name', async ({ page }) => {
    await page.goto(`${o.sender}/page.html`);
    await embed(page, `${o.player}/?mock=1`);
    await expect.poll(() => gotTypes(page)).toEqual(['ready']);
    const frame = page.frameLocator('iframe.player');
    await frame.locator('.player-file-input').setInputFiles({ name: 'secret-name.log', mimeType: 'text/plain', buffer: Buffer.from('1704067200|Me|A|/x\n') });
    await expect(frame.locator('.viewer-name')).toHaveText('secret-name.log');
    await expect.poll(() => gotEvents(page)).toEqual([
      { event: 'log_opened', props: { source: 'file', log_size: '<10KB', commits: 1 }, origin: o.player, fromPlayer: true },
    ]);
    expect(JSON.stringify(await got(page))).not.toContain('secret-name');
  });
});

test.describe('local file', () => {
  test('after a handed-over log nothing of it is stored, and a file stores its own settings again', async ({ page }) => {
    await page.goto(`${o.sender}/page.html`);
    const opened = await openPlayer(page, `${o.player}/?mock=1`);
    await expect.poll(() => gotTypes(page)).toEqual(['ready']);
    await handOver(page, o.player);
    await expectState(opened, 'playing');
    await opened.getByRole('button', { name: 'Settings' }).click();
    await opened.locator('#control-panel [data-section="style"] > summary').click();
    await opened.locator('#opt-user-style-preset').selectOption('glowing-hats');
    await opened.getByRole('button', { name: 'Open another log' }).click();
    await expectState(opened, 'idle');
    expect(await opened.evaluate(() => localStorage.getItem('gource.ai-options.v1'))).toBeNull();
    await expect(opened).toHaveTitle('Gource player');

    await opened.locator('.player-file-input').setInputFiles({ name: 'mine.log', mimeType: 'text/plain', buffer: Buffer.from('1704067200|Me|A|/x\n') });
    await expectState(opened, 'playing');
    await expect(opened).toHaveTitle('mine.log · Gource player');
    await opened.getByRole('button', { name: 'Settings' }).click();
    await opened.locator('#control-panel [data-section="style"] > summary').click();
    await opened.locator('#opt-user-style-preset').selectOption('glowing-hats');
    await expect.poll(() => opened.evaluate(() => localStorage.getItem('gource.ai-options.v1'))).toContain('assisted');
  });

  test('opens a log from this computer, and another after it', async ({ page }) => {
    await page.goto(`${o.player}/?mock=1`);
    await expectState(page, 'idle');
    await expect(page.locator('.player-empty')).toContainText('Nothing is uploaded');
    const input = page.locator('.player-file-input');
    await input.setInputFiles({ name: 'first.log', mimeType: 'text/plain', buffer: Buffer.from('1704067200|A|A|/a\n1704070800|B|A|/b\n1704074400|A|M|/a\n') });
    await expectState(page, 'playing');
    await expect(page.locator('.viewer-name')).toHaveText('first.log');
    await expect(page.locator('[data-stat="commits"]')).toContainText('/ 3 commits');
    await page.locator('.play-btn').click();
    await expect(page.locator('.play-btn')).toHaveAttribute('aria-label', 'Play');
    await input.setInputFiles({ name: 'second.log', mimeType: 'text/plain', buffer: Buffer.from('1704067200|A|A|/a\n') });
    await expect(page.locator('.viewer-name')).toHaveText('second.log');
    await expect(page.locator('[data-stat="commits"]')).toContainText('/ 1 commits');
    await expect(page.locator('gource-viewer')).toHaveCount(1);
  });

  test('the file picker offers history.gitlog', async ({ page }) => {
    await page.goto(`${o.player}/?mock=1`);
    expect((await page.locator('.player-file-input').getAttribute('accept'))?.split(',')).toContain('.gitlog');
  });

  test('says when a file has no commit, and offers another', async ({ page }) => {
    await page.goto(`${o.player}/?mock=1`);
    await page.locator('.player-file-input').setInputFiles({ name: 'empty.log', mimeType: 'text/plain', buffer: Buffer.from('not a log\n') });
    await expect(page.locator('.player-banner')).toContainText('No commit could be read');
    await expectState(page, 'failed');
    await expect(page.locator('main[data-screen="viewer"]')).toHaveCount(0);
    await expect(page.locator('.player-empty')).toBeVisible();
  });
});

test.describe('the start screen', () => {
  const CLI_VERSION = JSON.parse(readFileSync(new URL('../../cli/package.json', import.meta.url), 'utf8')).version as string;

  /** Records whether the start screen was ever on screen, from the first mutation of the page. */
  const watchStart = () => {
    const w = window as unknown as { startShown: boolean };
    w.startShown = false;
    new MutationObserver(() => {
      if (document.querySelector('.player-empty:not([hidden])')) w.startShown = true;
    }).observe(document, { subtree: true, childList: true, attributes: true });
  };
  const startShown = (page: import('@playwright/test').Page) => page.evaluate(() => (window as unknown as { startShown: boolean }).startShown);

  test('opens a file first, says how to make one, and links the hosted site, Gource and the source', async ({ page }) => {
    await page.goto(`${o.player}/?mock=1`);
    await expectState(page, 'idle');
    const start = page.locator('.player-empty');
    await expect(start.getByRole('heading', { level: 2 })).toHaveText('Play a Gource log');
    await expect(start.getByRole('button', { name: 'Open a log file' })).toBeVisible();
    // One way to open a file at a time: the bar's is for the states without the start screen.
    await expect(page.locator('.player-bar .player-open')).toBeHidden();
    await expect(start).toContainText('The file is read in this browser. Nothing is uploaded.');

    const git = start.getByLabel('git command', { exact: true });
    await expect(git).toHaveText(/^git -c core\.quotepath=off log --reverse --raw .* --output=history\.gitlog$/);
    await expect(start.getByLabel('npx command', { exact: true })).toHaveText(`npx -y gource-online@${CLI_VERSION}`);

    const link = (name: string | RegExp) => start.getByRole('link', { name });
    // The test build's senders are not gource.online, so the hosted site is gource.online.
    for (const [name, href] of [
      [/^Go to gource\.online/, 'https://gource.online'],
      [/^gource\.io/, 'https://gource.io'],
      [/^Gource on GitHub/, 'https://github.com/acaudwell/Gource'],
      [/^Star it on GitHub/, 'https://github.com/joe-byounghern-kim/gource-online'],
    ] as const) {
      await expect(link(name)).toHaveAttribute('href', href);
      await expect(link(name)).toHaveAttribute('target', '_blank');
      await expect(link(name)).toHaveAttribute('rel', 'noopener noreferrer');
      await expect(link(name)).toContainText('(opens in a new tab)');
    }
    await expect(start).toContainText('Andrew Caudwell');
    await expect(start).toContainText('not made or endorsed by Gource\'s author');

    // Gone while a log plays: nothing is left over the visualisation.
    await page.locator('.player-file-input').setInputFiles({ name: 'a.log', mimeType: 'text/plain', buffer: Buffer.from('1704067200|A|A|/a\n') });
    await expectState(page, 'playing');
    await expect(page.locator('main.player')).toBeHidden();
    await expect(start).toBeHidden();
  });

  test('the Copy buttons copy each command, and select it where the clipboard is not available', async ({ page, context }) => {
    // A clipboard that records what it is given: the real one also needs the page to have focus.
    await page.addInitScript(() => {
      const copied: string[] = [];
      Object.assign(window, { __copied: copied });
      Object.defineProperty(Navigator.prototype, 'clipboard', { get: () => ({ writeText: async (text: string) => { copied.push(text); } }) });
    });
    await page.goto(`${o.player}/?mock=1`);
    await expectState(page, 'idle');
    const start = page.locator('.player-empty');
    for (const name of ['git command', 'npx command']) {
      // Each command has its own status, and the first one's "Copied" stays for 4 s: check the one beside the button.
      const block = start.locator('.start-command', { has: page.getByRole('button', { name: `Copy the ${name}` }) });
      await block.getByRole('button').click();
      await expect(block.getByRole('status')).toHaveText('Copied');
      expect(await page.evaluate(() => (window as unknown as { __copied: string[] }).__copied.at(-1))).toBe(await start.getByLabel(name, { exact: true }).textContent());
    }

    const bare = await context.newPage();
    await bare.addInitScript(() => Object.defineProperty(Navigator.prototype, 'clipboard', { get: () => undefined }));
    await bare.goto(`${o.player}/?mock=1`);
    await expectState(bare, 'idle');
    await bare.getByRole('button', { name: 'Copy the npx command' }).click();
    await expect(bare.locator('.player-empty').getByRole('status').filter({ hasText: 'press Ctrl+C' })).toHaveCount(1);
    expect(await bare.evaluate(() => window.getSelection()?.toString())).toBe(`npx -y gource-online@${CLI_VERSION}`);
  });

  test('works with the keyboard, and fits a 390px-wide screen', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(`${o.player}/?mock=1`);
    await expectState(page, 'idle');
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= document.documentElement.clientWidth)).toBe(true);
    await page.keyboard.press('Tab');
    await expect(page.getByRole('button', { name: 'Open a log file' })).toBeFocused();
    await page.keyboard.press('Tab');
    await expect(page.getByRole('button', { name: 'Copy the git command' })).toBeFocused();
    await page.keyboard.press('Tab');
    await expect(page.getByLabel('git command', { exact: true })).toBeFocused();
  });

  test('never shows while a log by link loads or a page hands one over', async ({ page }) => {
    // On the context, so the window the sender opens below has it too.
    await page.context().addInitScript(watchStart);
    await page.goto(`${o.player}/?mock=1${fragment({ src: `${o.logs}/start-hidden.log` })}`);
    await expectState(page, 'playing');
    expect(await startShown(page)).toBe(false);

    await page.goto(`${o.sender}/page.html`);
    const opened = await openPlayer(page, `${o.player}/?mock=1`);
    await expect.poll(() => gotTypes(page)).toEqual(['ready']);
    await expectState(opened, 'waiting');
    await expect(opened.locator('.player-bar .player-open')).toBeVisible();
    await handOver(page, o.player);
    await expectState(opened, 'playing');
    expect(await startShown(opened)).toBe(false);
  });
});
