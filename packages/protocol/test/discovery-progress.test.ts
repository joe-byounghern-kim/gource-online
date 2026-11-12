// SPDX-License-Identifier: MIT
import { test, expect } from '@playwright/test';
import * as protocol from '../src/index.js';

const api = protocol as unknown as {
  readActivityBusy(value: unknown): unknown;
  readActivityDiscoveryEvent(value: unknown): unknown;
};
const repo = { id: 'repo_1', url: 'https://github.com/example/project', name: 'example/project', default_branch: 'main', head: 'a'.repeat(40), fork: false, visibility: 'public' };
const result = { discovery: 'discovery_1', repositories: [repo], coverage: 'incomplete', expires_at: '2026-10-19T12:00:00Z' };
const busy = { reason: 'slot_held', retry_after: 600, expires_at: result.expires_at };

test('busy details copy only bounded reasons and server deadlines', () => {
  expect(api.readActivityBusy).toBeDefined();
  expect(api.readActivityBusy({ ...busy, account: 'PRIVATE', message: 'PRIVATE' })).toEqual(busy);
  for (const change of [{ reason: 'PRIVATE' }, { retry_after: 0 }, { retry_after: 601 }, { retry_after: 1.1 }, { retry_after: '3' }, { expires_at: '2026-02-31T00:00:00Z' }])
    expect(api.readActivityBusy({ ...busy, ...change })).toBeNull();
  expect(Object.isFrozen(api.readActivityBusy(busy))).toBe(true);
});

test('discovery stream frames are closed, immutable and strip provider objects', () => {
  expect(api.readActivityDiscoveryEvent).toBeDefined();
  const progress = { type: 'progress', stage: 'metadata', completed: 3, total: 50 };
  expect(api.readActivityDiscoveryEvent({ ...progress, email: 'PRIVATE' })).toEqual(progress);
  expect(api.readActivityDiscoveryEvent({ type: 'result', result: { ...result, token: 'PRIVATE' } })).toEqual({ type: 'result', result });
  expect(api.readActivityDiscoveryEvent({ type: 'error', code: 'provider_unavailable', message: 'PRIVATE' })).toEqual({ type: 'error', code: 'provider_unavailable' });
  for (const raw of [{ ...progress, stage: 'PRIVATE' }, { ...progress, completed: 51 }, { ...progress, completed: -1 }, { ...progress, total: 51 }, { ...progress, completed: 0.5 }, { type: 'error', code: 'PRIVATE' }, { type: 'result', result: {} }, { type: 'PRIVATE' }])
    expect(api.readActivityDiscoveryEvent(raw)).toBeNull();
  expect(Object.isFrozen(api.readActivityDiscoveryEvent(progress))).toBe(true);
});

test('discovery statistics are optional bounded display hints, never identity authority', () => {
  const repositories = [{ ...repo, commit_count: 42, last_activity_at: '2025-12-31T00:00:00Z' }];
  expect(protocol.readActivityDiscovery({ ...result, repositories })?.repositories).toEqual(repositories);
  expect(protocol.readActivityDiscovery(result)?.repositories).toEqual([repo]);
  expect(protocol.readActivityDiscovery({ ...result, repositories: [{ ...repo, commit_count: null, last_activity_at: null }] })?.repositories[0]?.commit_count).toBeNull();
  for (const change of [{ commit_count: -1 }, { commit_count: 1.5 }, { commit_count: '42' }, { last_activity_at: 'PRIVATE' }])
    expect(protocol.readActivityDiscovery({ ...result, repositories: [{ ...repo, ...change }] })).toBeNull();
});
