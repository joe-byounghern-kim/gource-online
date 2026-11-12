// SPDX-License-Identifier: MIT
import { test, expect } from '@playwright/test';
import * as protocol from '../src/hosted-activity.js';

test('account quota reader copies only bounded numeric display hints for both features', () => {
  const read = (protocol as unknown as { readActivityQuotas: (raw: unknown) => unknown }).readActivityQuotas;
  const quotas = [
    { feature: 'user-activity', used: 3, limit: 10, remaining: 7, email: 'private@example.invalid' },
    { feature: 'pull-request-reviews', used: 8, limit: 5, remaining: 0 },
  ];
  expect(read(quotas)).toEqual([
    { feature: 'user-activity', used: 3, limit: 10, remaining: 7 },
    { feature: 'pull-request-reviews', used: 8, limit: 5, remaining: 0 },
  ]);
  expect(JSON.stringify(read(quotas))).not.toContain('@');
  for (const raw of [null, {}, [], [quotas[0]], [quotas[0], quotas[0]],
    [{ ...quotas[0], used: -1 }, quotas[1]], [{ ...quotas[0], limit: 1.5 }, quotas[1]],
    [{ ...quotas[0], remaining: 8 }, quotas[1]], [{ ...quotas[0], used: Number.MAX_SAFE_INTEGER + 1 }, quotas[1]],
    [{ ...quotas[0], feature: 'unknown' }, quotas[1]]]) expect(read(raw)).toBeNull();
});
