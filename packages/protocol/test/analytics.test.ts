// SPDX-License-Identifier: MIT

import { expect, test } from '@playwright/test';

import { ANALYTICS_EVENTS, sanitizeBatch, sizeBucket, viewportBucket } from '../src/analytics.js';

const ID = '3b241101-e2bb-4255-8caf-4136c566a962';

test('a valid batch passes unchanged', () => {
  const batch = {
    id: ID,
    events: [
      { event: 'analysis_finished', ts: 1, props: { cached: true, commits: 12, log_size: '<1MB', app_version: 'v1', screen: 'analysis' } },
      { event: 'page_view', ts: 2, props: { screen: 'landing', webgl2: true, dpr: 2 } },
    ],
  };
  expect(sanitizeBatch(batch)).toEqual(batch);
});

test('unknown events, unknown keys, wrong types and nested values are dropped', () => {
  const out = sanitizeBatch({
    id: ID,
    events: [
      { event: 'evil', ts: 1, props: {} },
      { event: 'page_view', ts: 1, props: { screen: 'landing', file_name: 'secret.log', dpr: 'two', webgl2: { x: 1 }, viewport: ['<480'] } },
      { event: 'analysis_failed', ts: 'soon', props: { code: 'busy' } },
      { event: 'analysis_failed', ts: 3, props: { code: 'busy', elapsed_ms: Infinity } },
    ],
  });
  expect(out).toEqual({
    id: ID,
    events: [
      { event: 'page_view', ts: 1, props: { screen: 'landing' } },
      { event: 'analysis_failed', ts: 3, props: { code: 'busy' } },
    ],
  });
});

test('strings are cut to 200 characters and batches to 50 events', () => {
  const long = 'x'.repeat(500);
  const events = Array.from({ length: 80 }, (_, i) => ({ event: 'client_error', ts: i, props: { kind: 'unhandled', message: long } }));
  const out = sanitizeBatch({ id: ID, events });
  expect(out?.events.length).toBe(50);
  expect((out?.events[0]?.props.message as string).length).toBe(200);
});

test('a bad id, a non-object or an empty result gives null', () => {
  for (const bad of [null, 'x', [], { id: 'nope', events: [{ event: 'page_view', ts: 1, props: {} }] }, { id: ID, events: 'x' }, { id: ID, events: [{ event: 'evil', ts: 1, props: {} }] }, { id: ID.toUpperCase().replace('4255', '1255'), events: [] }]) {
    expect(sanitizeBatch(bad)).toBeNull();
  }
});

test('props may be missing', () => {
  expect(sanitizeBatch({ id: ID, events: [{ event: 'page_view', ts: 1 }] })).toEqual({ id: ID, events: [{ event: 'page_view', ts: 1, props: {} }] });
});

test('every event allows the common keys', () => {
  for (const keys of Object.values(ANALYTICS_EVENTS)) {
    expect(keys.app_version).toBe('string');
    expect(keys.screen).toBe('string');
  }
});

test('size and viewport buckets', () => {
  expect([0, 9_999, 10_000, 99_999, 100_000, 999_999, 1_000_000, 9_999_999, 10_000_000, 99_999_999, 100_000_000].map(sizeBucket)).toEqual([
    '<10KB', '<10KB', '<100KB', '<100KB', '<1MB', '<1MB', '<10MB', '<10MB', '<100MB', '<100MB', '>=100MB',
  ]);
  expect(viewportBucket(1920, 479)).toBe('<480');
  expect(viewportBucket(480, 1000)).toBe('<768');
  expect(viewportBucket(1199, 1300)).toBe('<1200');
  expect(viewportBucket(2560, 1440)).toBe('>=1200');
});
