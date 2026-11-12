// SPDX-License-Identifier: MIT

import { expect, test } from '@playwright/test';

import { FALLBACK_FEATURES, parseFeatures } from '../src/features.js';

test('a registry is read; a feature is never both available and requestable', () => {
  expect(parseFeatures({ available: ['public-repos', 'ci-runs'], requestable: ['ci-runs', 'user-activity'] })).toEqual({
    available: ['public-repos', 'ci-runs'],
    requestable: ['user-activity'],
  });
});

test('anything unexpected gives the fallback', () => {
  for (const bad of [null, 'x', {}, { available: [1], requestable: [] }, { available: [], requestable: 'x' }]) {
    expect(parseFeatures(bad)).toEqual(FALLBACK_FEATURES);
  }
});

test('legal links are kept only as same-origin paths', () => {
  const read = (links: unknown) => parseFeatures({ available: [], requestable: [], links }).links;
  expect(read({ privacy: '/privacy', terms: '/terms' })).toEqual({ privacy: '/privacy', terms: '/terms' });
  expect(read({ privacy: 'https://evil.example/', terms: '//evil.example' })).toBeUndefined();
  expect(read({ privacy: 'javascript:alert(1)', terms: '/\\evil' })).toBeUndefined();
  expect(read({ privacy: '/ok', terms: 7 })).toEqual({ privacy: '/ok' });
  expect(read('nope')).toBeUndefined();
});

test('a deployment notice is kept as plain text of at most 300 characters', () => {
  const read = (notice: unknown) => parseFeatures({ available: [], requestable: [], notice }).notice;
  expect(read('Maintenance at 10:00 UTC')).toBe('Maintenance at 10:00 UTC');
  expect(read('  padded  ')).toBe('padded');
  expect(read('')).toBeUndefined();
  expect(read(7)).toBeUndefined();
  expect(read('line\nbreak')).toBeUndefined();
  expect(read('x'.repeat(301))).toBeUndefined();
});
