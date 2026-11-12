// SPDX-License-Identifier: GPL-3.0-or-later

// What a release build leaves out and keeps: built the way release.yml builds it, with no test setting, then opened
// with every test switch in its address.

import { expect, test } from '@playwright/test';
import { execFileSync } from 'node:child_process';
import { createServer, type Server } from 'node:http';
import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { extname, join, normalize } from 'node:path';
import { fileURLToPath } from 'node:url';

const WASM = fileURLToPath(new URL('../../../packages/viewer/src/wasm/gource_web_bg.wasm', import.meta.url));
const LOG = '1704067200|Alice|A|/src/a.rs\n';
const MIME: Record<string, string> = { '.html': 'text/html', '.js': 'text/javascript', '.wasm': 'application/wasm', '.css': 'text/css', '.ttf': 'font/ttf' };

test.skip(!existsSync(WASM), 'needs scripts/build-web.sh');

test('a release build has no test hooks or sample logs, ignores the test switches, and links the source it names', async ({ page }) => {
  test.setTimeout(120_000);
  const dir = mkdtempSync(join(tmpdir(), 'gource-player-release-'));
  const { GOURCE_PLAYER_SENDERS: _s, GOURCE_PLAYER_SOURCES: _l, GOURCE_PLAYER_TEST_BUILD: _t, VITE_E2E_HOOKS: _h, ...env } = process.env;
  let server: Server | undefined;
  try {
    execFileSync('pnpm', ['exec', 'vite', 'build', '--outDir', dir, '--emptyOutDir'], {
      cwd: fileURLToPath(new URL('../', import.meta.url)),
      env: { ...env, GOURCE_SOURCE_REF: 'v9.9.9-test', GOURCE_REPO_URL: 'https://example.org/acme/gource' },
      stdio: 'pipe',
    });
    expect(existsSync(join(dir, 'samples'))).toBe(false);
    for (const file of ['_headers', 'index.html', 'COPYING.txt', 'THIRD_PARTY_LICENSES.txt', 'fonts/FreeSans.ttf']) expect(existsSync(join(dir, file)), file).toBe(true);
    const code = readdirSync(join(dir, 'assets')).filter((name) => name.endsWith('.js')).map((name) => readFileSync(join(dir, 'assets', name), 'utf8')).join('\n');
    expect(code).toContain('https://example.org/acme/gource');
    expect(code).toContain('v9.9.9-test');
    // The switches are gone. What they would turn on stays as dead code: the recording's hooks (src/record/hooks.ts) and the
    // placeholder engine's chunk, which `?mock` reaches only in a test build (src/main.ts).
    for (const hook of [/__gource\b/, /mock-record/, /fixed-dt/]) expect(code, String(hook)).not.toMatch(hook);
    const headers = readFileSync(join(dir, '_headers'), 'utf8');
    expect(headers).not.toContain('127.0.0.1');

    const policy = /^ {2}Content-Security-Policy: (.+)$/m.exec(headers)![1]!;
    const requested: string[] = [];
    server = createServer((req, res) => {
      const path = new URL(req.url!, 'http://x').pathname;
      requested.push(path);
      res.setHeader('Content-Security-Policy', policy);
      if (path === '/same.log') { res.setHeader('Content-Type', 'text/plain'); res.end(LOG); return; }
      try {
        const body = readFileSync(join(dir, normalize(path === '/' ? 'index.html' : path)));
        res.setHeader('Content-Type', MIME[extname(path === '/' ? 'index.html' : path)] ?? 'application/octet-stream');
        res.end(body);
      } catch {
        res.writeHead(404);
        res.end();
      }
    });
    await new Promise<void>((done) => server!.listen(0, '127.0.0.1', done));
    const origin = `http://127.0.0.1:${(server.address() as { port: number }).port}`;
    await page.goto(`${origin}/?mock=1&mock-record=1&fixed-dt=0.0166667#src=${encodeURIComponent('/same.log')}`);
    await expect(page.locator('main[data-screen="viewer"]')).toBeVisible({ timeout: 30_000 });
    await expect(page.locator('.seek')).toBeEnabled({ timeout: 30_000 });
    const globals = await page.evaluate(() => {
      const w = window as unknown as Record<string, unknown>;
      return ['__gource', '__gourceMock', '__record', '__recordTest', '__recordSink'].filter((name) => w[name] !== undefined);
    });
    expect(globals).toEqual([]);
    expect(requested.filter((path) => /mock/.test(path))).toEqual([]);
    expect((await page.request.get(`${origin}/samples/file-removal.log`)).status()).toBe(404);
  } finally {
    if (server) { server.closeAllConnections(); await new Promise((done) => server!.close(done)); }
    rmSync(dir, { recursive: true, force: true });
  }
});
