// SPDX-License-Identifier: GPL-3.0-or-later

// Real HTTP origins for the player's browser specs, with no request interception:
// - player: the test build of apps/player, served with its own _headers;
// - sender, sender2: pages allowed to hand it a log;
// - logs: an allowed log source, which answers with CORS;
// - hostile: an origin that is neither.
// The build fixes the origins, so the servers listen before it runs.

import { expect, type Page } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { createServer, type IncomingMessage, type Server, type ServerResponse } from 'node:http';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { extname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

export const LOG = '1704067200|Alice|A|/src/a.rs\n1704070800|Bob|A|/src/b.rs\n1704074400|Alice|M|/src/a.rs\n';
const MIME: Record<string, string> = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.ttf': 'font/ttf' };

/** A page that records every message it receives, with whether it came from the player window. */
const RECORDER = `<!doctype html><meta charset="utf-8"><link rel="icon" href="data:,"><title>sender</title><script>
window.got = [];
addEventListener('message', (e) => {
  const player = document.querySelector('iframe.player')?.contentWindow ?? window.player;
  window.got.push({ origin: e.origin, type: e.data?.type, code: e.data?.code, id: e.data?.id, fromPlayer: e.source === player });
});
</script>`;

export interface Request { path: string; cookie?: string; referer?: string }
export interface Origins { player: string; sender: string; sender2: string; logs: string; hostile: string }

type Handler = (req: IncomingMessage, res: ServerResponse) => void;

/** Recorder pages: /page.html sends the hosted sender's COOP (same-origin-allow-popups), /plain.html none. */
const pages = (name: string, extra?: Handler): Handler => (req, res) => {
  const path = new URL(req.url!, 'http://x').pathname;
  if (path === '/page.html' || path === '/plain.html') {
    if (path === '/page.html') res.setHeader('Cross-Origin-Opener-Policy', 'same-origin-allow-popups');
    res.setHeader('Content-Type', 'text/html');
    res.end(RECORDER.replace('<title>sender</title>', `<title>${name}</title>`));
    return;
  }
  if (extra) extra(req, res);
  else { res.writeHead(404); res.end(); }
};

export async function startOrigins(): Promise<{ origins: Origins; requests: Request[]; stop(): Promise<void> }> {
  const servers: Server[] = [];
  const requests: Request[] = [];
  const serve = async (name: string, handler: Handler) => {
    const server = createServer((req, res) => {
      requests.push({ path: `${name}:${new URL(req.url!, 'http://x').pathname}`, cookie: req.headers.cookie, referer: req.headers.referer });
      handler(req, res);
    });
    servers.push(server);
    await new Promise<void>((done) => server.listen(0, '127.0.0.1', done));
    return `http://127.0.0.1:${(server.address() as { port: number }).port}`;
  };
  const o = {} as Origins;
  o.sender = await serve('sender', pages('sender'));
  o.sender2 = await serve('sender2', pages('sender2'));
  o.hostile = await serve('hostile', pages('hostile', (_req, res) => {
    res.setHeader('Access-Control-Allow-Origin', '*');
    res.end(LOG);
  }));
  o.logs = await serve('logs', (req, res) => {
    const path = new URL(req.url!, 'http://x').pathname;
    if (!path.startsWith('/no-cors')) res.setHeader('Access-Control-Allow-Origin', o.player);
    // A redirect to a log the player may fetch: only `redirect: 'error'` stops it.
    if (path.startsWith('/redirect')) { res.writeHead(302, { location: `${o.logs}/redirected${path.slice('/redirect'.length)}` }); res.end(); return; }
    res.setHeader('Content-Type', 'text/plain');
    res.end(LOG);
  });
  let dir = '';
  const headers: Record<string, string> = {};
  o.player = await serve('player', (req, res) => {
    const url = new URL(req.url!, 'http://x');
    for (const [key, value] of Object.entries(headers)) {
      // `?keep-referrer` drops Referrer-Policy from the page and from what it loads (the parser worker has its own
      // policy), so only the fetch settings can keep the referrer out.
      const keep = url.searchParams.has('keep-referrer') || (req.headers.referer ?? '').includes('keep-referrer');
      if (!(key === 'Referrer-Policy' && keep)) res.setHeader(key, value);
    }
    if (url.pathname.startsWith('/same')) { res.setHeader('Content-Type', 'text/plain'); res.end(LOG); return; }
    try {
      const file = join(dir, url.pathname === '/' ? 'index.html' : url.pathname);
      res.setHeader('Content-Type', MIME[extname(file)] ?? 'application/octet-stream');
      res.end(readFileSync(file));
    } catch {
      res.writeHead(404);
      res.end('Not found');
    }
  });
  dir = mkdtempSync(join(tmpdir(), 'gource-player-'));
  execFileSync('pnpm', ['exec', 'vite', 'build', '--outDir', dir, '--emptyOutDir'], {
    cwd: fileURLToPath(new URL('../', import.meta.url)),
    // VITE_E2E_HOOKS: the recording's test hooks (?mock-record=1).
    env: { ...process.env, GOURCE_PLAYER_SENDERS: `${o.sender} ${o.sender2}`, GOURCE_PLAYER_SOURCES: o.logs, GOURCE_PLAYER_TEST_BUILD: '1', VITE_E2E_HOOKS: '1' },
    stdio: 'pipe',
  });
  for (const line of readFileSync(join(dir, '_headers'), 'utf8').split('\n')) {
    const match = /^ {2}([^:]+): (.+)$/.exec(line);
    if (match) headers[match[1]!] = match[2]!;
  }
  return {
    origins: o,
    requests,
    stop: async () => {
      await Promise.all(servers.map((server) => new Promise<void>((done) => server.close(() => done()))));
      rmSync(dir, { recursive: true, force: true });
    },
  };
}

export const fragment = (params: Record<string, string>) => `#${new URLSearchParams(params)}`;

export interface Got { origin: string; type: string; code?: string; id?: string; fromPlayer: boolean }
export const got = (page: Page): Promise<Got[]> => page.evaluate(() => (window as unknown as { got: Got[] }).got);
export const gotTypes = async (page: Page) => (await got(page)).map((m) => (m.code ? `${m.type}:${m.code}` : m.type));
export const mockCalls = (page: Page) => page.evaluate(() => window.__gourceMock?.calls.map((c) => `${c.method}(${c.args.join(',')})`) ?? []);

/** Embeds the player in the current page, as `iframe.player`. */
export async function embed(page: Page, url: string): Promise<void> {
  await page.evaluate((src) => {
    const frame = document.createElement('iframe');
    frame.className = 'player';
    frame.src = src;
    frame.style.cssText = 'width: 800px; height: 600px';
    document.body.append(frame);
  }, url);
}

/** Opens the player in a new window named `gplayer`, kept as `window.player`. */
export async function openPlayer(page: Page, url: string): Promise<Page> {
  const popup = page.waitForEvent('popup');
  await page.evaluate((src) => { (window as unknown as { player: Window | null }).player = window.open(src, 'gplayer'); }, url);
  return popup;
}

export type Part = 'begin' | 'chunk' | 'end';

/**
 * Sends a handoff's messages the way a sender does (the chunk transferred), from `window.player` or the
 * embedded player's window, in the order given. `via` names another window expression to post to.
 */
export async function handOver(page: Page, player: string, { id = 'handoff-1', parts = ['begin', 'chunk', 'end'] as Part[], via = '' } = {}): Promise<void> {
  await page.evaluate(({ log, origin, id, parts, via }) => {
    const w: Window = via
      ? (new Function(`return ${via}`)() as Window)
      : document.querySelector<HTMLIFrameElement>('iframe.player')?.contentWindow ?? (window as unknown as { player: Window }).player;
    const bytes = new TextEncoder().encode(log);
    const envelope = { protocol: 'gource-player', version: 1 };
    const messages = {
      begin: { ...envelope, type: 'begin', id, bytes: bytes.byteLength, chunk_bytes: 65536, chunks: 1, title: 'Handed over', options: { 'seconds-per-day': '3' } },
      chunk: { ...envelope, type: 'chunk', id, seq: 0, data: bytes.buffer },
      end: { ...envelope, type: 'end', id },
    };
    for (const part of parts) w.postMessage(messages[part], origin, part === 'chunk' ? [bytes.buffer] : []);
  }, { log: LOG, origin: player, id, parts, via });
}

export async function expectState(page: Page, state: string): Promise<void> {
  await expect(page.locator('main.player')).toHaveAttribute('data-state', state);
}
