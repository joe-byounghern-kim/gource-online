// SPDX-License-Identifier: MIT
import { test, expect } from '@playwright/test';
import * as protocol from '../src/index.js';

const api = protocol as unknown as {
  readActivityDiscovery(value: unknown): unknown;
  readActivityJob(value: unknown): unknown;
  readActivitySnapshot(value: unknown): unknown;
  readActivityPreview(value: unknown): unknown;
};
const repository = { id: 'repo_1', url: 'https://github.com/example/project', name: 'example/project',
  default_branch: 'main', head: 'a'.repeat(40), fork: false, visibility: 'public' };
const discovery = { discovery: 'discovery_1', repositories: [repository], coverage: 'incomplete', expires_at: '2026-10-11T05:00:00.000Z' };

test('discovery copies only bounded public receipts, never identity fields or provider objects', () => {
  expect(api.readActivityDiscovery).toBeDefined();
  const result = api.readActivityDiscovery({ ...discovery, target_id: 77, email: 'seeded-private@example.invalid', token: 'SEEDED_SECRET',
    repositories: [{ ...repository, provider_response: { email: 'seeded-private@example.invalid' }, account_id: 77 }] });
  expect(result).toEqual(discovery);
  expect(JSON.stringify(result)).not.toMatch(/seeded-private|SEEDED_SECRET|target_id|account_id|provider_response/);
  expect(Object.isFrozen(result)).toBe(true);
});

test('private or malformed discoveries fail closed before browser display', () => {
  expect(api.readActivityDiscovery).toBeDefined();
  for (const change of [
    { repositories: [{ ...repository, visibility: 'private' }] },
    { repositories: [{ ...repository, url: 'https://secret@github.com/example/project' }] },
    { repositories: [{ ...repository, name: 'seeded-private@example.invalid' }] },
    { repositories: [{ ...repository, head: 'short' }] },
    { repositories: Array.from({ length: 51 }, () => repository) },
    { coverage: 'complete' }, { expires_at: 'not a date' }, { discovery: '../forged' },
  ]) expect(api.readActivityDiscovery({ ...discovery, ...change })).toBeNull();
});

test('canonical discovery names retain literal case-insensitive .git suffixes', () => {
  for (const suffix of ['.git', '.GIT', '.git.git']) {
    const selected = { ...repository, name: `example/project${suffix}`, url: `https://github.com/example/project${suffix}` };
    expect(api.readActivityDiscovery({ ...discovery, repositories: [selected] }))
      .toEqual({ ...discovery, repositories: [selected] });
    for (const url of [`${selected.url}.git`, `${selected.url}?ref=main`, `${selected.url}/tree/main`]) {
      expect(api.readActivityDiscovery({ ...discovery, repositories: [{ ...selected, url }] })).toBeNull();
    }
  }
});

test('ready logs can only use the guarded same-origin job route', () => {
  expect(api.readActivityJob).toBeDefined();
  const job = { job: 'job_1', status: 'ready', cached: true, log_url: '/api/activity/jobs/job_1/log', coverage: ['discovery-incomplete'] };
  expect(api.readActivityJob({ ...job, token: 'SEEDED_SECRET' })).toEqual(job);
  for (const log_url of ['https://storage.invalid/log', '//storage.invalid/log', '/api/logs/abc', '/api/activity/jobs/other/log', '/api/activity/jobs/job_1/log?token=secret'])
    expect(api.readActivityJob({ ...job, log_url })).toBeNull();
  expect(api.readActivityJob({ ...job, status: 'queued', log_url: undefined })).toEqual({ job: 'job_1', status: 'queued', cached: true, coverage: ['discovery-incomplete'] });
});

test('snapshot and preview readers are immutable, bounded and sanitize extra private fields', () => {
  expect(api.readActivitySnapshot).toBeDefined();
  expect(api.readActivityPreview).toBeDefined();
  const snapshot = { snapshot: 'snapshot_1', state: 'owner-only', expires_at: discovery.expires_at };
  expect(api.readActivitySnapshot({ ...snapshot, identity_index: ['SEEDED_SECRET'] })).toEqual(snapshot);
  const preview = { preview: 'preview_1', names: ['Example'], paths: ['/example/project/src/main.rs'], requires_alias_disclosure: true };
  expect(api.readActivityPreview({ ...preview, email: 'seeded-private@example.invalid' })).toEqual(preview);
  expect(api.readActivityPreview({ ...preview, names: ['seeded-private@example.invalid'] })).toBeNull();
  expect(api.readActivitySnapshot({ ...snapshot, state: 'public-again' })).toBeNull();
});

test('job status and coverage are primitive closed enums, not coerced provider data', () => {
  const job = { job: 'job_1', status: 'queued', cached: false };
  for (const status of [['ready'], ['queued'], { toString: () => 'queued' }, null])
    expect(api.readActivityJob({ ...job, status })).toBeNull();
  for (const coverage of [['seeded_private_provider_value'], Array(1)])
    expect(api.readActivityJob({ ...job, coverage })).toBeNull();
});

test('expiry rejects normalized impossible calendar dates', () => {
  for (const expires_at of ['2026-02-31T00:00:00Z', '2026-02-29T00:00:00.000Z', '2026-13-01T00:00:00Z']) {
    expect(api.readActivityDiscovery({ ...discovery, expires_at })).toBeNull();
    expect(api.readActivitySnapshot({ snapshot: 'snapshot_1', state: 'owner-only', expires_at })).toBeNull();
  }
});

test('preview arrays must be dense and fit an aggregate UTF8 budget', () => {
  const preview = { preview: 'preview_1', names: ['Example'], paths: ['/example/project'], requires_alias_disclosure: false };
  expect(api.readActivityPreview({ ...preview, names: Array(1) })).toBeNull();
  expect(api.readActivityPreview({ ...preview, paths: Array(1) })).toBeNull();
  expect(api.readActivityPreview({ ...preview, paths: Array.from({ length: 300 }, () => '界'.repeat(2000)) })).toBeNull();
});

test('long no-email paths fit the wire budget without regex backtracking amplification', () => {
  const paths = Array.from({ length: 200 }, (_, index) => `/${index}/${'x'.repeat(4000)}`);
  const preview = { preview: 'preview_1', names: ['Example'], paths, requires_alias_disclosure: false };
  expect(api.readActivityPreview(preview)).toEqual(preview);
});

test('a preview path may hold @, a preview name never does', () => {
  const preview = { preview: 'preview_1', names: ['Example'], paths: ['/a'], requires_alias_disclosure: false };
  for (const path of ['/logo@2x.png', '/node_modules/@types/a.d.ts', '/person@example.invalid', 'x'.repeat(3000) + '@example.invalid'])
    expect(api.readActivityPreview({ ...preview, paths: [path] })).toEqual({ ...preview, paths: [path] });
  for (const name of ['@alice@example.invalid', 'a@.alice@example.invalid', 'name@not-an-email'])
    expect(api.readActivityPreview({ ...preview, names: [name] })).toBeNull();
  for (const path of ['', '/a@b\u0000c', '/a@b\nc', '/a@b\u007fc', 'x'.repeat(4097)])
    expect(api.readActivityPreview({ ...preview, paths: [path] })).toBeNull();
});
