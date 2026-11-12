// SPDX-License-Identifier: MIT

// The feature registry (GET /api/features): features a deployment offers,
// and features a viewer can ask for (POST /api/interest). Optional links name
// the deployment's legal pages; an optional notice is shown on the page.

export type FeatureId = 'public-repos' | 'local-file' | 'pull-requests' | 'pull-request-reviews' | 'private-repos' | 'ci-runs' | 'user-activity';

export type PrivateHost = 'github' | 'gitlab' | 'other';

export interface FeatureLinks {
  privacy?: string;
  terms?: string;
}

export interface FeatureList {
  available: string[];
  requestable: string[];
  links?: FeatureLinks;
  /** A short plain-text message from the deployment (maintenance, an incident). */
  notice?: string;
}

/** What a client assumes when the registry cannot be read: no request buttons. */
export const FALLBACK_FEATURES: FeatureList = { available: ['public-repos', 'local-file'], requestable: [] };

/** A path on the page's own origin: one leading `/`, no `//`, `\` or control characters. */
function samePath(value: unknown): value is string {
  return typeof value === 'string' && /^\/(?!\/)[^\\\u0000-\u001f\u007f]*$/.test(value) && !value.includes('//');
}

/** Validates the registry's JSON; anything unexpected gives the fallback. */
export function parseFeatures(value: unknown): FeatureList {
  if (typeof value !== 'object' || value === null) return FALLBACK_FEATURES;
  const v = value as { available?: unknown; requestable?: unknown; links?: unknown; notice?: unknown };
  const strings = (x: unknown): string[] | null =>
    Array.isArray(x) && x.every((s) => typeof s === 'string') ? (x as string[]) : null;
  const available = strings(v.available);
  const requestable = strings(v.requestable);
  if (!available || !requestable) return FALLBACK_FEATURES;
  const list: FeatureList = { available, requestable: requestable.filter((f) => !available.includes(f)) };
  if (typeof v.links === 'object' && v.links !== null) {
    const l = v.links as { privacy?: unknown; terms?: unknown };
    const links: FeatureLinks = {};
    if (samePath(l.privacy)) links.privacy = l.privacy;
    if (samePath(l.terms)) links.terms = l.terms;
    if (links.privacy || links.terms) list.links = links;
  }
  if (typeof v.notice === 'string') {
    const notice = v.notice.trim();
    if (notice !== '' && notice.length <= 300 && !/[\u0000-\u001f\u007f]/.test(notice)) list.notice = notice;
  }
  return list;
}
