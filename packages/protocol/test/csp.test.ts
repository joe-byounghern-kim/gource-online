// SPDX-License-Identifier: MIT
import { test, expect } from '@playwright/test';
import * as protocol from '../src/index.js';

const api = protocol as unknown as {
  parseCspConnectOrigins(value?: string): readonly string[];
  withCspConnectOrigins(policy: string, value?: string): string;
};
const base = "default-src 'none'; connect-src 'self'; worker-src 'self'; object-src 'none'";

function available(): void {
  expect(api.parseCspConnectOrigins, 'the shared origin allowlist validator is exported').toBeDefined();
  expect(api.withCspConnectOrigins, 'the shared CSP builder is exported').toBeDefined();
}

test('unconfigured log origins retain the exact same-origin CSP', () => {
  available();
  for (const value of [undefined, '', ' \n\t ']) {
    expect(api.parseCspConnectOrigins(value)).toEqual([]);
    expect(api.withCspConnectOrigins(base, value)).toBe(base);
  }
});

test('explicit origins are canonicalized, deduplicated, bounded and immutable', () => {
  available();
  const origins = api.parseCspConnectOrigins('https://LOGS.example:443/ https://logs.example http://127.0.0.1:9000 https://[::1]:9443');
  expect(origins).toEqual(['https://logs.example', 'http://127.0.0.1:9000', 'https://[::1]:9443']);
  expect(Object.isFrozen(origins)).toBe(true);
  expect(api.withCspConnectOrigins(base, 'https://logs.example https://cdn.example:8443')).toBe(
    "default-src 'none'; connect-src 'self' https://logs.example https://cdn.example:8443; worker-src 'self'; object-src 'none'",
  );
});

test('wildcards, credentials, paths and directive injection fail without echoing input', () => {
  available();
  for (const value of [
    '*', 'https:', 'data:', 'blob:https://logs.example', 'ws://logs.example', 'ftp://logs.example',
    'https://*.example', 'https://logs.example:*', "'self'", "https://logs.example; script-src *",
    'https://seeded-token@logs.example', 'https://user:seeded-token@logs.example',
    'https://logs.example/path', 'https://logs.example/./', 'https://logs.example//',
    'https://logs.example?', 'https://logs.example#', 'https://logs.example?token=seeded-token',
    'https://logs.example/#seeded-token', 'https://%6cogs.example', 'https://logs.example\\path',
    'https://logs.example:99999', 'https://logs.example\u0000',
    Array.from({ length: 33 }, (_, i) => `https://logs${i}.example`).join(' '),
    'x'.repeat(4097),
  ]) {
    expect(() => api.parseCspConnectOrigins(value), value).toThrow('Invalid CSP connect origins.');
    try { api.parseCspConnectOrigins(value); } catch (error) {
      expect(String(error)).not.toContain('seeded-token');
    }
  }
});

test('the builder refuses missing, duplicate or already widened connect directives', () => {
  available();
  for (const policy of ["default-src 'none'", `${base}; connect-src 'self'`, "connect-src *; object-src 'none'"])
    expect(() => api.withCspConnectOrigins(policy, 'https://logs.example')).toThrow('Invalid base CSP.');
});

test('the shared base policy is widenable and relaxes only WebAssembly compilation', () => {
  const policy = (protocol as unknown as { GOURCE_BASE_CSP: string }).GOURCE_BASE_CSP;
  expect(typeof policy).toBe('string');
  expect(api.withCspConnectOrigins(policy)).toBe(policy);
  expect(policy).not.toMatch(/'unsafe-inline'|'unsafe-eval'|https?:|\*/);
  expect(policy.match(/'wasm-unsafe-eval'/g)).toHaveLength(1);
  expect(policy).toMatch(/^default-src 'none';/);
  expect(policy).toMatch(/frame-ancestors 'none'; object-src 'none'$/);
});
