// SPDX-License-Identifier: MIT

/**
 * The one page policy every host serves: the static `_headers`, the local CLI server and the hosted edge.
 * `withCspConnectOrigins` is the only permitted widening. `'wasm-unsafe-eval'` is the only relaxation.
 */
export const GOURCE_BASE_CSP =
  "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; " +
  "img-src 'self' data: blob:; media-src 'self' blob:; font-src 'self'; " +
  "connect-src 'self'; worker-src 'self'; manifest-src 'self'; base-uri 'none'; " +
  "form-action 'self'; frame-ancestors 'none'; object-src 'none'";

/** Explicit HTTP(S) origins for direct log loading, never arbitrary CSP syntax. */
export function parseCspConnectOrigins(value?: string): readonly string[] {
  const invalid = () => new Error('Invalid CSP connect origins.');
  if (value !== undefined && (typeof value !== 'string' || value.length > 4096)) throw invalid();
  const entries = (value ?? '').trim().split(/\s+/).filter(Boolean);
  if (entries.length > 32) throw invalid();
  const origins: string[] = [];
  for (const entry of entries) {
    // Reject paths before URL normalization can erase /./ or backslashes.
    if (!/^https?:\/\/(?:\[[0-9a-fA-F:]+\]|[a-zA-Z0-9.-]+)(?::[0-9]{1,5})?\/?$/.test(entry)) throw invalid();
    let url: URL;
    try { url = new URL(entry); } catch { throw invalid(); }
    if (!url.hostname || url.username || url.password || url.pathname !== '/' || url.search || url.hash) throw invalid();
    if (!origins.includes(url.origin)) origins.push(url.origin);
  }
  return Object.freeze(origins);
}

/** Widen only the exact single same-origin connect directive, preserving all other bytes. */
export function withCspConnectOrigins(policy: string, value?: string): string {
  const origins = parseCspConnectOrigins(value);
  const directives = policy.split(';').filter((part) => part.trim().startsWith('connect-src'));
  if (directives.length !== 1 || directives[0]?.trim() !== "connect-src 'self'") throw new Error('Invalid base CSP.');
  return origins.length ? policy.replace("connect-src 'self'", `connect-src 'self' ${origins.join(' ')}`) : policy;
}
