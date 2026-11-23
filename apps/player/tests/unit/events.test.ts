// SPDX-License-Identifier: GPL-3.0-or-later

// The player's events for the page that opened or embeds it (src/events.ts): where they go, and what they carry.

import { expect, test } from '@playwright/test';

import { failureCode, roughCount, senderEvents } from '../../src/events';

function fakeWindow() {
  const posted: { message: unknown; origin: string }[] = [];
  return { posted, win: { postMessage: (message: unknown, origin: string) => posted.push({ message, origin }) } as unknown as Window };
}

test('events go to each allowed origin, then only to the one a handoff came from', () => {
  const { posted, win } = fakeWindow();
  const events = senderEvents(win, ['https://a.example', 'https://b.example']);
  events.track('playback_ended', { watched_s: 3 });
  expect(posted.map((p) => p.origin)).toEqual(['https://a.example', 'https://b.example']);
  events.only('https://evil.example');
  events.only('https://b.example');
  events.track('playback_ended', { watched_s: 4 });
  expect(posted.slice(2).map((p) => p.origin)).toEqual(['https://b.example']);
  expect(posted[2]!.message).toEqual({ protocol: 'gource-player', version: 1, type: 'event', event: 'playback_ended', props: { watched_s: 4 } });
});

test('no sender, no allowed origin, or an event the protocol refuses: nothing is posted', () => {
  const { posted, win } = fakeWindow();
  senderEvents(null, ['https://a.example']).track('playback_ended', { watched_s: 1 });
  senderEvents(win, []).track('playback_ended', { watched_s: 1 });
  const events = senderEvents(win, ['https://a.example']);
  events.track('recording_failed', { reason: 'My Repo' });
  events.track('log_opened', { source: 'file', path: '/src/a.rs' });
  events.track('analysis_started', { host: 'github.com', repo: 'acme/secret' });
  expect(posted).toEqual([]);
});

test('commit counts are rounded to two significant figures; failure codes are short codes', () => {
  expect([0, -3, Number.NaN, 7, 99, 100, 1234, 1250, 98_765, 999].map(roughCount)).toEqual([0, 0, 0, 7, 99, 100, 1200, 1300, 99_000, 1000]);
  expect(failureCode('encoder_unavailable')).toBe('encoder_unavailable');
  for (const code of [22, '', 'Secret', 'my repo', undefined, 'x'.repeat(65)]) expect(failureCode(code)).toBe('internal');
});
