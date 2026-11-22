// SPDX-License-Identifier: MIT
import { test, expect } from '@playwright/test';
import * as protocol from '../src/index.js';
import type { ActivityRepository, ActivityResultProvenance, ActivitySourceAccessState, PrivateAnalyzeRequest, PrivateConsentRequest, PrivateRepositoryRequest } from '../src/index.js';

const validProvenance: ActivityResultProvenance[] = [
  { visibility: 'public', publication: 'review-required' },
  { visibility: 'private', publication: 'forbidden' },
  { visibility: 'internal', publication: 'forbidden' },
  { visibility: 'mixed', publication: 'forbidden' },
];

for (const provenance of validProvenance) {
  test(`copies and freezes ${provenance.visibility} result provenance`, () => {
    expect(protocol.readActivityResultProvenance).toBeDefined();
    const input = { ...provenance };
    const result = protocol.readActivityResultProvenance(input);
    expect(result).toEqual(provenance);
    expect(result).not.toBe(input);
    expect(Object.isFrozen(result)).toBe(true);
    input.visibility = 'mixed';
    input.publication = 'review-required';
    expect(result).toEqual(provenance);
  });
}

for (const provenance of [
  { visibility: 'public', publication: 'forbidden' },
  { visibility: 'private', publication: 'review-required' },
  { visibility: 'internal', publication: 'review-required' },
  { visibility: 'mixed', publication: 'review-required' },
]) {
  test(`rejects ${provenance.visibility} provenance with ${provenance.publication} publication`, () => {
    expect(protocol.readActivityResultProvenance).toBeDefined();
    expect(protocol.readActivityResultProvenance(provenance)).toBeNull();
  });
}

test('provenance requires exact primitive closed values and both fields', () => {
  expect(protocol.readActivityResultProvenance).toBeDefined();
  for (const value of [
    null, undefined, true, 1, 'public', [], {},
    { visibility: 'public' }, { publication: 'review-required' },
    { visibility: 'PUBLIC', publication: 'review-required' },
    { visibility: 'unknown', publication: 'forbidden' },
    { visibility: ['public'], publication: 'review-required' },
    { visibility: { toString: () => 'public' }, publication: 'review-required' },
    { visibility: 'public', publication: 'allowed' },
    { visibility: 'public', publication: ['review-required'] },
    { visibility: 'private', publication: { toString: () => 'forbidden' } },
    Object.assign([], validProvenance[0]),
  ]) expect(protocol.readActivityResultProvenance(value)).toBeNull();
});

test('provenance never copies authority, credentials or other provider fields', () => {
  expect(protocol.readActivityResultProvenance).toBeDefined();
  const input = {
    visibility: 'private', publication: 'forbidden', account_id: 17, subject_id: 18,
    installation_id: 19, repository_id: 20, token: 'SEEDED_SECRET',
    provider_response: { email: 'seeded-private@example.invalid' },
    get credentials(): never { throw new Error('extra fields must not be read'); },
  };
  expect(protocol.readActivityResultProvenance(input)).toEqual({ visibility: 'private', publication: 'forbidden' });
});

test('unreadable provenance fails closed without throwing', () => {
  expect(protocol.readActivityResultProvenance).toBeDefined();
  for (const key of ['visibility', 'publication']) {
    const value = Object.defineProperty({ ...validProvenance[0] }, key, {
      get(): never { throw new Error('unreadable field'); },
    });
    expect(protocol.readActivityResultProvenance(value)).toBeNull();
  }
  const { proxy, revoke } = Proxy.revocable({}, {});
  revoke();
  expect(protocol.readActivityResultProvenance(proxy)).toBeNull();
});

const accessStates: ActivitySourceAccessState[] = [
  'available', 'approval_required', 'reauthentication_required', 'access_lost', 'unavailable',
];
for (const state of accessStates) {
  test(`reads the exact ${state} source access state`, () => {
    expect(protocol.readActivitySourceAccessState).toBeDefined();
    expect(protocol.readActivitySourceAccessState(state)).toBe(state);
  });
}

test('source access states reject unknown values and coercible objects', () => {
  expect(protocol.readActivitySourceAccessState).toBeDefined();
  for (const value of [
    null, undefined, false, 0, '', 'ready', 'AVAILABLE', 'available ',
    'approval-required', 'reauthentication-required', 'access-lost',
    [], ['available'], { state: 'available' }, { toString: () => 'available' },
    new String('available'), Symbol('available'),
  ]) expect(protocol.readActivitySourceAccessState(value)).toBeNull();
  const { proxy, revoke } = Proxy.revocable({}, {});
  revoke();
  expect(protocol.readActivitySourceAccessState(proxy)).toBeNull();
});

test('M1 provenance does not widen public-only discovery before M4', () => {
  const repository: ActivityRepository = {
    id: 'repo_1', url: 'https://github.com/example/project', name: 'example/project',
    default_branch: 'main', head: 'a'.repeat(40), fork: false, visibility: 'public',
  };
  const discovery = {
    discovery: 'discovery_1', repositories: [repository], coverage: 'incomplete',
    expires_at: '2026-10-19T05:00:00.000Z',
  };
  expect(protocol.readActivityDiscovery(discovery)).toEqual(discovery);
  for (const visibility of ['private', 'internal', 'mixed']) {
    expect(protocol.readActivityDiscovery({
      ...discovery, repositories: [{ ...repository, visibility }],
      provenance: { visibility, publication: 'forbidden' },
    })).toBeNull();
  }
});

const repository = { id: 'r_1', url: 'https://github.com/org/secret', name: 'org/secret', default_branch: 'main', head: 'a'.repeat(40), fork: false, visibility: 'private', owner: 'organization' };
const discovery = { discovery: 'd_1', repositories: [repository], coverage: 'incomplete', expires_at: '2030-01-01T00:00:00.000Z' };

test('private discovery copies only the closed contract and freezes it', () => {
  const result = protocol.readActivityPrivateDiscovery({ ...discovery, repositories: [{ ...repository, installation_id: 3, permissions: { admin: true } }], account: 'acct' });
  expect(result).toEqual(discovery);
  expect(Object.isFrozen(result) && Object.isFrozen(result!.repositories) && Object.isFrozen(result!.repositories[0])).toBe(true);
  expect(protocol.readActivityPrivateDiscovery({ ...discovery, repositories: [{ ...repository, visibility: 'internal', owner: 'personal' }] })!.repositories[0])
    .toEqual({ ...repository, visibility: 'internal', owner: 'personal' });
});

test('private discovery refuses public, unknown, duplicated, oversized and non-canonical entries', () => {
  for (const patch of [{ visibility: 'public' }, { visibility: 'secret' }, { owner: 'team' }, { owner: undefined }, { url: 'https://github.com/org/other' },
    { url: 'http://github.com/org/secret' }, { name: 'org/secret/x' }, { head: 'HEAD' }, { default_branch: '-x' }, { default_branch: '' }, { fork: 'no' }, { id: 'has space' }])
    expect(protocol.readActivityPrivateDiscovery({ ...discovery, repositories: [{ ...repository, ...patch }] }), JSON.stringify(patch)).toBeNull();
  expect(protocol.readActivityPrivateDiscovery({ ...discovery, repositories: [repository, repository] })).toBeNull();
  expect(protocol.readActivityPrivateDiscovery({ ...discovery, repositories: Array.from({ length: 51 }, (_, i) => ({ ...repository, id: 'r' + i })) })).toBeNull();
  for (const patch of [{ coverage: 'complete' }, { expires_at: '2030-01-01' }, { discovery: '' }, { repositories: {} }])
    expect(protocol.readActivityPrivateDiscovery({ ...discovery, ...patch }), JSON.stringify(patch)).toBeNull();
});

test('the install response is only GitHub\'s own install page for one App, with one state', () => {
  const ok = 'https://github.com/apps/gource-activity/installations/new?state=abc.DEF_-1';
  expect(protocol.readPrivateInstallResponse({ install_url: ok })).toEqual({ install_url: ok });
  for (const url of ['https://evil.example/apps/x/installations/new?state=a', 'http://github.com/apps/x/installations/new?state=a',
    'https://github.com/apps/x/installations/new', 'https://github.com/apps/x/installations/new?state=a&redirect=https://evil.example',
    'https://github.com/apps/X/installations/new?state=a', 'https://github.com/login/oauth/authorize?state=a', 'https://u:p@github.com/apps/x/installations/new?state=a',
    'https://github.com/apps/x/installations/new?state=a#frag', 'https://github.com/apps/x/installations/new?state=a%20b', 'not a url',
    'https://github.com\\apps\\x\\installations\\new?state=a', 'https://github.com/apps/y/../x/installations/new?state=a', 'https://GITHUB.com/apps/x/installations/new?state=a',
    'https://github.com/apps/x/installations/new\t?state=a', 'https://ｇｉｔｈｕｂ.com/apps/x/installations/new?state=a'])
    expect(protocol.readPrivateInstallResponse({ install_url: url }), url).toBeNull();
  expect(protocol.readPrivateInstallResponse({ install_url: ok, extra: 1 })).toBeNull();
});

test('install outcomes are closed', () => {
  expect(protocol.PRIVATE_INSTALL_PARAM).toBe('private-install');
  for (const outcome of ['ready', 'pending', 'unavailable'] as const) expect(protocol.readPrivateInstallOutcome(outcome)).toBe(outcome);
  for (const outcome of ['READY', '', null, 1, 'denied']) expect(protocol.readPrivateInstallOutcome(outcome)).toBeNull();
});

test('private provenance on jobs and snapshots is copied, never public, and keeps a snapshot owner-only', () => {
  const provenance = { visibility: 'private', publication: 'forbidden' };
  const job = protocol.readActivityJob({ job: 'j_1', status: 'running', cached: false, provenance });
  expect(job?.provenance).toEqual(provenance);
  expect(protocol.readActivityJob({ job: 'j_1', status: 'running', cached: false, provenance: { visibility: 'public', publication: 'review-required' } })).toBeNull();
  expect(protocol.readActivityJob({ job: 'j_1', status: 'running', cached: false, provenance: { visibility: 'private', publication: 'review-required' } })).toBeNull();
  const owner = { snapshot: 's_1', state: 'owner-only', expires_at: '2030-01-01T00:00:00.000Z' };
  expect(protocol.readActivitySnapshot({ ...owner, provenance: { visibility: 'mixed', publication: 'forbidden' } })?.provenance).toEqual({ visibility: 'mixed', publication: 'forbidden' });
  expect(protocol.readActivitySnapshot({ ...owner, state: 'published', share_url: protocol.snapshotShareUrl('s_1'), provenance })).toBeNull();
  expect(protocol.readActivitySnapshot(owner)).toEqual(owner);
});

test('private error codes are closed codes', () => {
  for (const code of ['private_install_required', 'private_reauthorize', 'private_access_lost'])
    expect((protocol.ACTIVITY_ERROR_CODES as readonly string[]).includes(code)).toBe(true);
});

test('private request shapes match the routes and stay closed', () => {
  expect(protocol.PRIVATE_SELECTION_LIMIT).toBe(10);
  const consent = { discovery: 'd1', repository_ids: ['p1'], purpose: 'user-activity', link_self: true } satisfies PrivateConsentRequest;
  const analyze = { discovery: 'd1', consent: 'c1', repository_ids: ['p1'], range: { since: '2026-01-01', until: '2026-06-30' }, idempotency_key: 'k1' } satisfies PrivateAnalyzeRequest;
  const repository = { url: 'https://github.com/org/r', consent: 'c2', since: '2026-01-01', until: '2026-06-30', pull_request_mode: 'full', idempotency_key: 'k2', continue_with_available: true } satisfies PrivateRepositoryRequest;
  expect(Object.keys(consent).sort()).toEqual(['discovery', 'link_self', 'purpose', 'repository_ids']);
  expect(Object.keys(analyze).sort()).toEqual(['consent', 'discovery', 'idempotency_key', 'range', 'repository_ids']);
  expect(Object.keys(repository).sort()).toEqual(['consent', 'continue_with_available', 'idempotency_key', 'pull_request_mode', 'since', 'until', 'url']);
  // @ts-expect-error verified-email matching is not offered for private sources
  const matched: PrivateConsentRequest = { ...consent, verified_email_match: true };
  // @ts-expect-error pull request activity has one mode
  const basic: PrivateRepositoryRequest = { ...repository, pull_request_mode: 'basic' };
  expect([matched, basic]).toHaveLength(2);
});

test('private access status is a bounded count and nothing else', () => {
  const { readPrivateAccessStatus } = protocol;
  for (const installations of [0, 1, 10]) expect(readPrivateAccessStatus({ installations })).toEqual({ installations });
  expect(Object.isFrozen(readPrivateAccessStatus({ installations: 1 }))).toBe(true);
  for (const value of [null, [], {}, { installations: -1 }, { installations: 11 }, { installations: 1.5 }, { installations: '1' },
    { installations: 1, names: ['org'] }, { installations: 1, installation_id: 3 }]) expect(readPrivateAccessStatus(value)).toBeNull();
});

test('private repository list copies names and display hints only, one per name, at most 100', () => {
  const { readPrivateRepositoryList, MAX_PRIVATE_REPOSITORY_LIST } = protocol;
  expect(MAX_PRIVATE_REPOSITORY_LIST).toBe(100);
  const entry = { name: 'org/secret', visibility: 'private', owner: 'organization', fork: false, pushed_at: '2026-11-10T00:00:00Z' };
  const input = { repositories: [entry, { name: 'me/tool', visibility: 'internal', owner: 'personal', fork: true, pushed_at: null }], truncated: false };
  const list = readPrivateRepositoryList(input);
  expect(list).toEqual(input);
  expect(Object.isFrozen(list)).toBe(true);
  expect(Object.isFrozen(list!.repositories)).toBe(true);
  expect(Object.isFrozen(list!.repositories[0])).toBe(true);
  entry.name = 'org/changed';
  expect(list!.repositories[0]!.name).toBe('org/secret');
  // Unknown fields, ids of any kind among them, never propagate.
  const extra = readPrivateRepositoryList({ repositories: [{ ...input.repositories[1], id: 42, installation_id: 7, node_id: 'R_x' }], truncated: true, account_id: 3 });
  expect(extra).toEqual({ repositories: [input.repositories[1]], truncated: true });
  expect(readPrivateRepositoryList({ repositories: [], truncated: false })).toEqual({ repositories: [], truncated: false });
  // GitHub's own spelling is kept: case, and a repository whose name ends in a literal `.git`.
  for (const name of ['Org/Repo', 'org/tool.git', 'org/.github']) {
    expect(readPrivateRepositoryList({ repositories: [{ ...input.repositories[1], name }], truncated: false })!.repositories[0]!.name).toBe(name);
  }
  const many = Array.from({ length: 100 }, (_, i) => ({ name: `org/r${i}`, visibility: 'private', owner: 'organization', fork: false, pushed_at: null }));
  expect(readPrivateRepositoryList({ repositories: many, truncated: true })!.repositories).toHaveLength(100);
});

test('private repository list refuses anything outside the contract', () => {
  const { readPrivateRepositoryList } = protocol;
  const ok = { name: 'org/secret', visibility: 'private', owner: 'organization', fork: false, pushed_at: '2026-11-10T00:00:00.000Z' };
  const many = Array.from({ length: 101 }, (_, i) => ({ ...ok, name: `org/r${i}` }));
  for (const value of [
    null, [], {}, { repositories: [] }, { repositories: [], truncated: 'no' }, { repositories: {}, truncated: false },
    { repositories: many, truncated: true },
    { repositories: [ok, { ...ok, name: 'ORG/Secret' }], truncated: false },
    { repositories: [{ ...ok, visibility: 'public' }], truncated: false },
    { repositories: [{ ...ok, owner: 'enterprise' }], truncated: false },
    { repositories: [{ ...ok, fork: 'false' }], truncated: false },
    { repositories: [{ ...ok, pushed_at: 'yesterday' }], truncated: false },
    { repositories: [{ ...ok, pushed_at: '2026-11-10T00:00:00+09:00' }], truncated: false },
    { repositories: [{ ...ok, pushed_at: undefined }], truncated: false },
    { repositories: [{ ...ok, name: 'org/secret/tree/main' }], truncated: false },
    { repositories: [{ ...ok, name: 'https://github.com/org/secret' }], truncated: false },
    { repositories: [{ ...ok, name: 'org' }], truncated: false },
    { repositories: [{ ...ok, name: '../org/secret' }], truncated: false },
    { repositories: [{ ...ok, name: `org/${'a'.repeat(300)}` }], truncated: false },
    { repositories: [null], truncated: false },
  ]) expect(readPrivateRepositoryList(value), JSON.stringify(value).slice(0, 80)).toBeNull();
});
