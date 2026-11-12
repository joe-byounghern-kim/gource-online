// SPDX-License-Identifier: MIT
import { test, expect } from '@playwright/test';
import { ABSENT_MAILMAP_SHA256, readSourceIdentityDescriptor, SOURCE_IDENTITY_INDEX_SCHEMA } from '../src/activity.js';

const descriptor = () => ({
  schema: 1, coordinate: 'custom-lines-v1', lines: 12, repository_id: 42,
  head: 'a'.repeat(40), log_sha256: 'b'.repeat(64), index_sha256: 'c'.repeat(64),
  mailmap_sha256: ABSENT_MAILMAP_SHA256, mailmap_state: 'absent',
  resolver_version: '1', label_version: '1', producer_version: '0.2.0-beta.1',
  format_version: 'custom-log-v1', options_sha256: 'e'.repeat(64),
});

test('identity descriptors pin the final sanitized log and line-coordinate index', () => {
  const value = readSourceIdentityDescriptor(descriptor());
  expect(SOURCE_IDENTITY_INDEX_SCHEMA).toBe(1);
  expect(value).toEqual(descriptor());
  expect(Object.isFrozen(value)).toBe(true);
});

test('private identity rows, emails and account IDs cannot be copied into browser descriptors', () => {
  const value = readSourceIdentityDescriptor({ ...descriptor(), account_id: 77,
    rows: [{ sha: 'a'.repeat(40), email: 'seeded-private@example.invalid' }],
    token: 'SEEDED_SECRET', access_url: 'https://private.invalid/artifact',
  });
  expect(value).toEqual(descriptor());
  expect(JSON.stringify(value)).not.toMatch(/seeded-private|SEEDED_SECRET|account_id|access_url|rows/);
});

test('malformed pins fail closed without echoing input or throwing', () => {
  for (const change of [
    { schema: 2 }, { coordinate: 'git-ordinal' }, { lines: -1 }, { lines: 1.5 },
    { lines: Number.MAX_SAFE_INTEGER + 1 }, { repository_id: 0 }, { repository_id: NaN },
    { head: 'short' }, { log_sha256: 'a'.repeat(63) }, { index_sha256: 'G'.repeat(64) },
    { mailmap_state: 'missing' }, { mailmap_sha256: undefined }, { mailmap_sha256: 'd'.repeat(64) },
    { format_version: 'custom-v1' }, { producer_version: '.leading' }, { producer_version: 'x'.repeat(129) },
    { resolver_version: 'seeded-private@example.invalid' }, { label_version: 'x|y' },
    { producer_version: 'x\ny' }, { options_sha256: '' },
  ]) expect(readSourceIdentityDescriptor({ ...descriptor(), ...change })).toBeNull();
  for (const value of [null, [], 'SEEDED_SECRET', false])
    expect(readSourceIdentityDescriptor(value)).toBeNull();
});

test('present mailmaps and private producer version boundaries round-trip', () => {
  const value = readSourceIdentityDescriptor({ ...descriptor(), mailmap_state: 'present',
    mailmap_sha256: 'd'.repeat(64), producer_version: 'x'.repeat(128) });
  expect(value?.mailmap_sha256).toBe('d'.repeat(64));
  expect(value?.producer_version).toHaveLength(128);
});

test('empty final logs and SHA-256 Git object IDs are valid pinned sources', () => {
  expect(readSourceIdentityDescriptor({ ...descriptor(), lines: 0, head: 'a'.repeat(64) })).not.toBeNull();
});
