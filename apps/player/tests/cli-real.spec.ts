// SPDX-License-Identifier: GPL-3.0-or-later

// `npx gource-online` end to end: the package built by
// scripts/build-cli.sh --pack, run with npx in a subfolder of a fresh
// repository, its link (the player's own, #src=…) opened in Chrome with the
// real wasm engine. Every request must stay on the command's own 127.0.0.1
// origin. Skipped unless
// E2E_CLI=1:
//
//   scripts/build-cli.sh --pack
//   E2E_CLI=1 pnpm -C apps/player exec playwright test tests/cli-real.spec.ts

import { execFileSync, spawn, type ChildProcessWithoutNullStreams } from 'node:child_process';
import { mkdirSync, mkdtempSync, readdirSync, realpathSync, statSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

import { expect, test } from '@playwright/test';

test.skip(process.env.E2E_CLI !== '1', 'set E2E_CLI=1 after scripts/build-cli.sh --pack');

const CLI_DIR = new URL('../../cli/', import.meta.url).pathname;

/** The newest packed package in apps/cli/. */
function packedPackage(): string {
  const tgz = readdirSync(CLI_DIR)
    .filter((f) => /^gource-online-.*\.tgz$/.test(f))
    .map((f) => join(CLI_DIR, f))
    .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0];
  if (!tgz) throw new Error('no apps/cli/gource-online-*.tgz: run scripts/build-cli.sh --pack');
  return tgz;
}

/** A repository with three commits by two authors, one co-authored by an agent. */
function makeRepo(): string {
  // The real path: on macOS the temporary folder is a link (/var → /private/var), and git prints the real one.
  const dir = join(realpathSync(mkdtempSync(join(tmpdir(), 'gource-cli-e2e-'))), 'acme widgets');
  mkdirSync(join(dir, 'src', 'ui'), { recursive: true });
  const git = (args: string[], name: string, time: number) =>
    execFileSync('git', args, {
      cwd: dir,
      env: {
        ...process.env,
        GIT_AUTHOR_NAME: name,
        GIT_AUTHOR_EMAIL: `${name.toLowerCase()}@example.com`,
        GIT_COMMITTER_NAME: name,
        GIT_COMMITTER_EMAIL: `${name.toLowerCase()}@example.com`,
        GIT_AUTHOR_DATE: `@${time} +0000`,
        GIT_COMMITTER_DATE: `@${time} +0000`,
      },
    });
  git(['init', '-q', '-b', 'main'], 'Ada', 0);
  writeFileSync(join(dir, 'README.md'), 'widgets\n');
  writeFileSync(join(dir, 'src', 'main.rs'), 'fn main() {}\n');
  git(['add', '-A'], 'Ada', 0);
  git(['commit', '-q', '-m', 'Start'], 'Ada', 1700000000);
  writeFileSync(join(dir, 'src', 'ui', 'button.rs'), 'pub struct Button;\n');
  git(['add', '-A'], 'Grace', 0);
  git(['commit', '-q', '-m', 'Add a button', '-m', 'Co-authored-by: Claude <noreply@anthropic.com>'], 'Grace', 1700086400);
  writeFileSync(join(dir, 'src', 'main.rs'), 'fn main() { println!("hi"); }\n');
  git(['commit', '-q', '-am', 'Say hi'], 'Ada', 1700172800);
  return dir;
}

/** Resolves with the command's link once it is printed. */
function waitForLink(child: ChildProcessWithoutNullStreams, output: { text: string }): Promise<string> {
  return new Promise((resolve, reject) => {
    const look = (chunk: Buffer) => {
      output.text += chunk.toString('utf8');
      const url = /http:\/\/127\.0\.0\.1:\d+\/#src=\S+/.exec(output.text)?.[0];
      if (url) resolve(url);
    };
    child.stdout.on('data', look);
    child.stderr.on('data', look);
    child.once('exit', (code) => reject(new Error(`gource-online exited (${code}) before printing a link:\n${output.text}`)));
  });
}

test('npx gource-online plays the checkout in the real engine, on its own origin only', async ({ page }) => {
  test.setTimeout(240_000);
  const repo = makeRepo();
  // Its own process group, so Ctrl+C reaches npx and the command together, as in a terminal.
  const child = spawn('npx', ['-y', `--package=${packedPackage()}`, 'gource-online', '--no-open'], {
    cwd: join(repo, 'src', 'ui'),
    detached: true,
  });
  const output = { text: '' };
  let origin = '';
  try {
    const link = await waitForLink(child, output);
    expect(output.text).toContain(`Reading the history of ${repo}`);
    expect(output.text).toMatch(/ {2}3 commits, /);
    origin = new URL(link).origin;

    const elsewhere: string[] = [];
    const loaded: string[] = [];
    page.on('request', (req) => {
      loaded.push(new URL(req.url()).pathname);
      if (new URL(req.url()).origin !== origin) elsewhere.push(req.url());
    });
    await page.goto(link);
    await expect(page.locator('[data-screen="viewer"]')).toBeVisible({ timeout: 60_000 });
    await expect(page.locator('.viewer-name')).toHaveText('acme widgets');
    await expect(page.locator('[data-stat="commits"]')).toContainText('/ 3 commits', { timeout: 60_000 });
    await expect(page.getByRole('button', { name: 'Copy link to this view' })).toBeHidden();
    // The player (a release build: it has no placeholder engine) ran the real wasm engine on every commit.
    await expect(page.locator('[data-stat="commits"]')).toContainText('3 / 3 commits', { timeout: 60_000 });
    expect(loaded.some((path) => path.endsWith('.wasm'))).toBe(true);
    expect(loaded.some((path) => /\/mock-[^/]*\.js$/.test(path))).toBe(false);
    expect(output.text).toContain('Public repositories and sign-in features: https://gource.online');
    await page.waitForTimeout(2000);
    await page.screenshot({ path: test.info().outputPath('cli-viewer.png') });
    expect(elsewhere).toEqual([]);

    // The history is not readable without the per-run token.
    const res = await page.request.get(`${origin}/acme%20widgets.gitlog`);
    expect(res.status()).toBe(404);
  } finally {
    if (child.pid) process.kill(-child.pid, 'SIGINT');
  }
  // npm re-raises Ctrl+C on itself once the command has stopped, as in a terminal.
  const ended = await new Promise<string>((done) => {
    if (child.exitCode !== null || child.signalCode !== null) done(String(child.exitCode ?? child.signalCode));
    else child.once('exit', (code, signal) => done(String(code ?? signal)));
  });
  expect(['0', 'SIGINT'], output.text).toContain(ended);
  // The server stopped with it.
  await expect(fetch(`${origin}/`)).rejects.toThrow();
});
