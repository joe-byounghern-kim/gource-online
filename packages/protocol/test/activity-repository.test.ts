// SPDX-License-Identifier: MIT
import { test, expect } from '@playwright/test';
import { activityRepositoryActor, activityRepositoryDirectory, activityRepositoryFilter, activityRepositoryOf } from '../src/activity-repository.js';

test('each repository owns one top-level directory and one actor', () => {
  expect(activityRepositoryDirectory('octo/app')).toBe('octo:app');
  expect(activityRepositoryDirectory('octo/app.git')).toBe('octo:app.git');
  expect(activityRepositoryDirectory('Octo-1/a_b.c')).toBe('Octo-1:a_b.c');
  expect(activityRepositoryActor('octo/app')).toBe('octo/app');
  // Same owner, different repositories: no shared directory node.
  expect(activityRepositoryDirectory('octo/app')).not.toBe(activityRepositoryDirectory('octo/lib'));
});

test('non-canonical names are rejected, not rewritten', () => {
  for (const name of ['', 'octo', 'octo/app/x', '/octo/app', 'octo/app/', 'octo:app', 'octo/a|b', 'octo/a b',
    'octo/../app', 'https://github.com/octo/app', `octo/${'a'.repeat(201)}`]) {
    expect(() => activityRepositoryDirectory(name)).toThrow('invalid-repository');
    expect(() => activityRepositoryActor(name)).toThrow('invalid-repository');
  }
  for (const name of ['', 'octo', 'octo/app/x', '/octo/app', 'octo:app', 'octo/a|b', 'octo/a b', `octo/${'a'.repeat(201)}`])
    expect(() => activityRepositoryFilter(name)).toThrow('invalid-repository');
  // The filter stays usable for names the strict rule rejects but a service might return.
  expect(new RegExp(activityRepositoryFilter('octo/..x')).test('/octo/..x/y')).toBe(true);
});

test('paths map to their repository in the current and the legacy layout', () => {
  const names = ['octo/app', 'octo/app-docs', 'other/app'];
  expect(activityRepositoryOf('/octo:app/src/main.rs', names)).toBe('octo/app');
  expect(activityRepositoryOf('/octo:app-docs/README.md', names)).toBe('octo/app-docs');
  expect(activityRepositoryOf('/other:app/x', names)).toBe('other/app');
  expect(activityRepositoryOf('/octo/app/src/main.rs', names)).toBe('octo/app');
  expect(activityRepositoryOf('octo/app-docs/README.md', names)).toBe('octo/app-docs');
  for (const path of ['/octo:apps/x', '/octo/apps/x', '/octo:app', '/octo', '/elsewhere:app/x', ''])
    expect(activityRepositoryOf(path, names)).toBeNull();
  expect(activityRepositoryOf('/octo:app/x', ['bad name', 'octo/app'])).toBe('octo/app');
});

test('the project filter matches only its own repository in either layout', () => {
  const filter = new RegExp(activityRepositoryFilter('octo/a.b'));
  for (const path of ['/octo:a.b/x', 'octo:a.b/x', '/octo/a.b/x']) expect(filter.test(path)).toBe(true);
  for (const path of ['/octo:aXb/x', '/octo/aXb/x', '/octo:a.bc/x', '/other:a.b/x', '/x/octo:a.b/y']) expect(filter.test(path)).toBe(false);
  // Prefix pairs and a literal .git suffix stay apart.
  const app = new RegExp(activityRepositoryFilter('octo/app'));
  for (const path of ['/octo/app-docs/x', '/octo:app-docs/x', '/octo:app.git/x']) expect(app.test(path)).toBe(false);
  expect(new RegExp(activityRepositoryFilter('octo/a.git')).test('/octo/a/x')).toBe(false);
  // A saved snapshot setting cannot contain '|'.
  expect(activityRepositoryFilter('octo/a.b')).toBe('^/?octo[:/]a\\.b/');
});
