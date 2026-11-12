// SPDX-License-Identifier: GPL-3.0-or-later

import { expect, test } from '@playwright/test';
import { playerConfig, playerCsp, playerHeaders } from '../../src/headers';

test('with no settings the player takes no log from elsewhere and cannot be embedded', () => {
  const config = playerConfig({});
  expect(config).toEqual({ senders: [], sources: [] });
  const csp = playerCsp(config);
  expect(csp).toContain("connect-src 'self';");
  expect(csp).toContain("frame-ancestors 'none'");
  expect(csp).toContain("form-action 'none'");
});

test('senders may embed the player and sources may serve it a log, and nothing else is widened', () => {
  const config = playerConfig({ senders: 'https://gource.online  https://example.com', sources: 'https://logs.gource.online' });
  const csp = playerCsp(config);
  expect(csp).toContain("connect-src 'self' https://logs.gource.online;");
  expect(csp).toContain('frame-ancestors https://gource.online https://example.com;');
  expect(csp).toContain("script-src 'self' 'wasm-unsafe-eval';");
  expect(csp).toContain("default-src 'none';");
  const headers = playerHeaders(config);
  expect(headers).toMatch(/^\/\*\n/);
  expect(headers).toContain('  Cross-Origin-Opener-Policy: unsafe-none\n');
  expect(headers).toContain('  Referrer-Policy: no-referrer\n');
});

test('origins are exact HTTPS origins; loopback only when asked', () => {
  for (const bad of ['http://gource.online', 'https://gource.online/', 'https://*.gource.online', "'self'", 'https://a.test;script-src', 'http://127.0.0.1:5000']) {
    expect(() => playerConfig({ senders: bad }), bad).toThrow();
    expect(() => playerConfig({ sources: bad }), bad).toThrow();
  }
  expect(playerConfig({ senders: 'http://127.0.0.1:5000', loopback: '1' }).senders).toEqual(['http://127.0.0.1:5000']);
});
