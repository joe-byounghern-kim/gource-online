// SPDX-License-Identifier: GPL-3.0-or-later

// The player's configuration and response headers. Both are fixed when the
// player is built (vite.config.ts): the pages allowed to hand it a log, and
// the origins it may download a log from.

import { GOURCE_BASE_CSP, parsePlayerOrigins } from '@gource-online/protocol';

export interface PlayerConfig {
  /** Exact origins whose pages may hand the player a log by postMessage, and embed it. */
  senders: readonly string[];
  /** Exact origins the player may download a log from (`#src=`), besides its own. */
  sources: readonly string[];
}

/**
 * Reads the build settings: space-separated exact origins. HTTPS only, unless `loopback` is `1`,
 * which also allows http loopback origins, for development and tests. An invalid entry fails the build.
 */
export function playerConfig(env: { senders?: string; sources?: string; loopback?: string }): PlayerConfig {
  const loopback = env.loopback === '1';
  const list = (value?: string) => parsePlayerOrigins((value ?? '').trim().split(/\s+/).filter(Boolean), { loopback });
  return { senders: list(env.senders), sources: list(env.sources) };
}

function replaceDirective(policy: string, from: string, to: string): string {
  const directives = policy.split(';').map((part) => part.trim());
  if (directives.filter((part) => part === from).length !== 1) throw new Error(`Invalid base CSP: no single "${from}".`);
  return directives.map((part) => (part === from ? to : part)).join('; ');
}

/**
 * The base policy, widened only where the handoff needs it: `connect-src` for the log sources and
 * `frame-ancestors` for the senders (the embedded mode). The player has no form, so `form-action` is `'none'`.
 */
export function playerCsp(config: PlayerConfig): string {
  let policy = GOURCE_BASE_CSP;
  if (config.sources.length) policy = replaceDirective(policy, "connect-src 'self'", `connect-src 'self' ${config.sources.join(' ')}`);
  if (config.senders.length) policy = replaceDirective(policy, "frame-ancestors 'none'", `frame-ancestors ${config.senders.join(' ')}`);
  return replaceDirective(policy, "form-action 'self'", "form-action 'none'");
}

/** Every response header of the player, by name. */
export function playerHeaderMap(config: PlayerConfig): Record<string, string> {
  return {
    'Content-Security-Policy': playerCsp(config),
    // The player sends nothing anywhere; a log URL never leaks through a referrer either.
    'Referrer-Policy': 'no-referrer',
    'X-Content-Type-Options': 'nosniff',
    // A page that opened the player keeps the link to it, so `ready` can reach that page (the new-window mode).
    'Cross-Origin-Opener-Policy': 'unsafe-none',
    'Cross-Origin-Resource-Policy': 'same-origin',
    'Permissions-Policy': 'camera=(), microphone=(), geolocation=(), payment=()',
  };
}

/** The static host's `_headers` file (Cloudflare Pages and Netlify syntax). */
export function playerHeaders(config: PlayerConfig): string {
  const lines = Object.entries(playerHeaderMap(config)).map(([name, value]) => `  ${name}: ${value}`);
  return `/*\n${lines.join('\n')}\n`;
}
