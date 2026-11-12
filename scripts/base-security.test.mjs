// SPDX-License-Identifier: GPL-3.0-or-later
import assert from 'node:assert/strict';
import { readdir, readFile } from 'node:fs/promises';
import { test } from 'node:test';

for (const file of ['ci.yml', 'security.yml']) {
  test(`${file} runs on the actual web-mvp merge base`, async () => {
    const source = await readFile(new URL(`../.github/workflows/${file}`, import.meta.url), 'utf8');
    assert.match(source, /push:\n\s+branches: \[main, web-mvp\]/);
  });
}

test('no workflow deploys a preview and CodeQL is not counted when private', async () => {
  // The Pages preview was removed before the first release: a push to main must not publish a site.
  const workflows = await readdir(new URL('../.github/workflows/', import.meta.url));
  for (const file of workflows) {
    const source = await readFile(new URL(`../.github/workflows/${file}`, import.meta.url), 'utf8');
    assert.doesNotMatch(source, /pages: write|deploy-pages/, `${file} deploys to GitHub Pages`);
  }
  const security = await readFile(new URL('../.github/workflows/security.yml', import.meta.url), 'utf8');
  assert.match(security, /if: \$\{\{ !github.event.repository.private \}\}/);
  assert.match(security, /skipped.*not.*coverage/i);
});

test('the server TLS lock excludes versions below the reviewed rustls security fix', async (t) => {
  const lock = await readFile(new URL('../Cargo.lock', import.meta.url), 'utf8');
  const version = lock.match(/name = "rustls"\nversion = "([^"]+)"/)?.[1];
  // Only gource-server uses rustls, and the published tree does not have it.
  if (!version) { assert.doesNotMatch(lock, /name = "gource-server"/); return t.skip('no TLS in this workspace'); }
  const [major, minor, patch] = version.split('.').map(Number);
  assert.ok(major > 0 || minor > 23 || (minor === 23 && patch >= 46), `rustls ${version} predates 0.23.46 security remediation`);
});

test('scheduled fuzz cannot be cancelled by a push or manual smoke while superseded PRs still cancel', async () => {
  const source = await readFile(new URL('../.github/workflows/security.yml', import.meta.url), 'utf8');
  const concurrency = source.match(/^concurrency:\n([\s\S]*?)(?=^\S)/m)?.[1];
  assert.ok(concurrency, 'workflow concurrency must be explicit');
  assert.match(concurrency, /group: \$\{\{ github.workflow \}\}-\$\{\{ github.event_name \}\}-\$\{\{ github.event.pull_request.number \|\| github.ref \}\}/);
  assert.match(concurrency, /cancel-in-progress: true/);
  // The event dimension separates schedule, push, PR and manual smoke, while
  // the PR number keeps successive commits to the same PR in one group.
  const template = concurrency.match(/group: (.+)/)[1];
  const group = (event, pr, ref = 'refs/heads/web-mvp') => template
    .replace('${{ github.workflow }}', 'Security')
    .replace('${{ github.event_name }}', event)
    .replace('${{ github.event.pull_request.number || github.ref }}', String(pr || ref));
  const scheduled = group('schedule');
  for (const event of ['push', 'pull_request', 'workflow_dispatch']) {
    assert.notEqual(scheduled, group(event, event === 'pull_request' ? 30 : undefined));
  }
  assert.equal(group('pull_request', 30), group('pull_request', 30));
  assert.notEqual(group('pull_request', 30), group('pull_request', 31));
});

test('full scheduled fuzz retains at least 30 minutes for setup, instrumented build and cleanup', async () => {
  const source = await readFile(new URL('../.github/workflows/security.yml', import.meta.url), 'utf8');
  const fuzz = source.match(/^  fuzz:\n([\s\S]*?)(?=^  \w+:)/m)?.[1];
  assert.ok(fuzz);
  assert.match(fuzz, /FUZZ_SECONDS:.*github.event_name == 'schedule' && '3600' \|\| '15'/);
  const minutes = Number(fuzz.match(/timeout-minutes: (\d+)/)?.[1]);
  assert.ok(minutes >= 90, '60-minute fuzz budget needs 30 minutes of non-fuzz headroom');
  assert.match(fuzz, /-max_total_time="\$FUZZ_SECONDS"/);
  assert.match(fuzz, /fail-fast: false/);
});
