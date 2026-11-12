// SPDX-License-Identifier: MIT
import { test, expect } from '@playwright/test';
import * as protocol from '../src/index.js';

const api = protocol as unknown as { readActivitySnapshotList(value: unknown): unknown };
const entry = {
  snapshot: 'snapshot_1', state: 'published', expires_at: '2026-11-18T00:00:00.000Z',
  created_at: '2026-10-19T00:00:00.000Z', project_count: 2,
  range: { since: '2020-01-01', until: '2020-12-31' }, size_bytes: 2048,
  share_url: '/?log=%2Fapi%2Fsnapshots%2Fsnapshot_1%2Flog',
};
const page = { snapshots: [entry], next_cursor: null };

test('snapshot lists copy bounded owner metadata and freeze every nested contract', () => {
  expect(api.readActivitySnapshotList).toBeDefined();
  const read = api.readActivitySnapshotList({ ...page, account: 'SEEDED_SECRET', snapshots: [{ ...entry, object_key: 'SEEDED_SECRET', sources: ['private'] }] });
  expect(read).toEqual(page);
  expect(JSON.stringify(read)).not.toMatch(/SEEDED_SECRET|object_key|sources|account/);
  expect(Object.isFrozen(read)).toBe(true);
  const typed = read as typeof page;
  expect(Object.isFrozen(typed.snapshots)).toBe(true);
  expect(Object.isFrozen(typed.snapshots[0])).toBe(true);
  expect(Object.isFrozen(typed.snapshots[0]!.range)).toBe(true);
});

test('published share URLs are canonical site-relative playback routes, never credentials', () => {
  for (const share_url of ['https://storage.invalid/object', '//storage.invalid/object', '/?log=/api/snapshots/other/log', '/?log=%2Fapi%2Fsnapshots%2Fsnapshot_1%2Flog&token=secret']) {
    expect(protocol.readActivitySnapshot({ ...entry, share_url })).toBeNull();
  }
  expect(protocol.readActivitySnapshot(entry)).toEqual({ snapshot: entry.snapshot, state: entry.state, expires_at: entry.expires_at, share_url: entry.share_url });
  expect(protocol.readActivitySnapshot({ ...entry, state: 'owner-only' })).toBeNull();
  expect(protocol.readActivitySnapshot({ snapshot: 'legacy', state: 'published', expires_at: entry.expires_at })).toEqual({ snapshot: 'legacy', state: 'published', expires_at: entry.expires_at });
});

test('snapshot list readers reject forged dates, counts, cursors, duplicate ids and oversize pages', () => {
  expect(api.readActivitySnapshotList).toBeDefined();
  for (const change of [
    { created_at: '2026-02-31T00:00:00Z' }, { created_at: entry.expires_at },
    { project_count: 0 }, { project_count: 51 }, { project_count: '2' },
    { size_bytes: -1 }, { size_bytes: 256 * 1024 * 1024 + 1 }, { size_bytes: 1.5 },
    { range: { since: '2020-02-31', until: '2020-12-31' } },
    { range: { since: '2021-01-01', until: '2020-12-31' } },
    { range: { since: '2020-01-01', until: '2030-01-01' } },
  ]) expect(api.readActivitySnapshotList({ ...page, snapshots: [{ ...entry, ...change }] })).toBeNull();
  for (const next_cursor of ['../forged', '123_id?secret=1', '', '9'.repeat(200), 'https://invalid.test/cursor'])
    expect(api.readActivitySnapshotList({ ...page, next_cursor })).toBeNull();
  expect(api.readActivitySnapshotList({ ...page, snapshots: [entry, entry] })).toBeNull();
  expect(api.readActivitySnapshotList({ ...page, snapshots: Array(1) })).toBeNull();
  expect(api.readActivitySnapshotList({ ...page, snapshots: Array.from({ length: 11 }, (_, i) => ({ ...entry, snapshot: `s${i}`, share_url: undefined })) })).toBeNull();
  expect(api.readActivitySnapshotList({ snapshots: [{ ...entry, range: null }], next_cursor: '1794960000000_snapshot_1' })).toEqual({ snapshots: [{ ...entry, range: null }], next_cursor: '1794960000000_snapshot_1' });
});
