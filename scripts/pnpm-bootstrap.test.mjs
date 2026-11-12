// SPDX-License-Identifier: GPL-3.0-or-later
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import test from 'node:test';

for (const [file, job] of [['ci.yml', 'web'], ['security.yml', 'audit']]) {
  test(`${job} initializes Node before pnpm's npm bootstrap`, () => {
    const workflow = readFileSync(new URL(`../.github/workflows/${file}`, import.meta.url), 'utf8');
    const block = workflow.split(`\n  ${job}:\n`)[1].split(/\n  [a-z][a-z-]*:\n/)[0];
    const steps = block.split(/\n      - /).slice(1);
    const pnpm = steps.findIndex(step => step.includes('uses: pnpm/action-setup@'));
    const node = steps.findIndex(step => step.includes('uses: actions/setup-node@'));
    assert.ok(node >= 0 && node < pnpm, 'Node must be on PATH before pnpm invokes npm');
    assert.match(steps[node], /node-version: 22/);
    assert.doesNotMatch(steps[node], /cache: pnpm/, 'pnpm caching must wait until pnpm is installed');
    if (job === 'web') {
      assert.ok(steps.some((step, index) => index > pnpm && step.includes('uses: actions/setup-node@') && step.includes('cache: pnpm')));
    }
  });
}
