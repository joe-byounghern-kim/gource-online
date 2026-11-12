// SPDX-License-Identifier: MIT

/** API-only descriptors. Identity evidence, account IDs and index rows stay server-side. */
export const SOURCE_IDENTITY_INDEX_SCHEMA = 1 as const;
/** SHA-256 of empty bytes, the descriptor sentinel for an absent mailmap. */
export const ABSENT_MAILMAP_SHA256 = 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855' as const;

export interface SourceIdentityDescriptor {
  /** Descriptor schema 1 projects the server's source-index-v1 contract. */
  readonly schema: typeof SOURCE_IDENTITY_INDEX_SCHEMA;
  /** Coordinates refer to final emitted custom-log lines, not parser commit groups. */
  readonly coordinate: 'custom-lines-v1';
  readonly lines: number;
  readonly repository_id: number;
  readonly head: string;
  readonly log_sha256: string;
  /** Exact canonical serialized index DOCUMENT bytes, not the private body digest. */
  readonly index_sha256: string;
  /** ABSENT_MAILMAP_SHA256 when mailmap_state is absent. */
  readonly mailmap_sha256: string;
  readonly mailmap_state: 'present' | 'absent';
  readonly resolver_version: string;
  readonly label_version: string;
  readonly producer_version: string;
  readonly format_version: string;
  readonly options_sha256: string;
}

const HASH = /^[0-9a-f]{64}$/;
const HEAD = /^(?:[0-9a-f]{40}|[0-9a-f]{64})$/;
const VERSION = /^[A-Za-z0-9][A-Za-z0-9._+-]{0,127}$/;
const hash = (value: unknown): value is string => typeof value === 'string' && HASH.test(value);
const version = (value: unknown): value is string => typeof value === 'string' && VERSION.test(value);

/** Whitelist the public descriptor rather than copying a server-private index object. */
export function readSourceIdentityDescriptor(raw: unknown): Readonly<SourceIdentityDescriptor> | null {
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) return null;
  const r = raw as Record<string, unknown>;
  if (r.schema !== SOURCE_IDENTITY_INDEX_SCHEMA || r.coordinate !== 'custom-lines-v1'
    || !Number.isSafeInteger(r.lines) || (r.lines as number) < 0
    || !Number.isSafeInteger(r.repository_id) || (r.repository_id as number) <= 0
    || typeof r.head !== 'string' || !HEAD.test(r.head)
    || !hash(r.log_sha256) || !hash(r.index_sha256) || !hash(r.mailmap_sha256)
    || !hash(r.options_sha256) || (r.mailmap_state !== 'present' && r.mailmap_state !== 'absent')
    || !version(r.resolver_version) || !version(r.label_version)
    || (r.mailmap_state === 'absent' && r.mailmap_sha256 !== ABSENT_MAILMAP_SHA256)
    || r.format_version !== 'custom-log-v1'
    || !version(r.producer_version)) return null;
  return Object.freeze({
    schema: SOURCE_IDENTITY_INDEX_SCHEMA, coordinate: 'custom-lines-v1',
    lines: r.lines as number, repository_id: r.repository_id as number, head: r.head,
    log_sha256: r.log_sha256, index_sha256: r.index_sha256,
    mailmap_sha256: r.mailmap_sha256, mailmap_state: r.mailmap_state,
    resolver_version: r.resolver_version, label_version: r.label_version,
    producer_version: r.producer_version, format_version: r.format_version,
    options_sha256: r.options_sha256,
  });
}

/** Inclusive UTC calendar dates. Validation and account binding happen on the server. */
export interface ActivityRange {
  since: string;
  until: string;
}

export interface PersonalDiscoveryRequest extends ActivityRange {
  /** Supplemental repository URLs. This never accepts a target person or account ID. */
  repositories?: string[];
}

export interface PersonalProject {
  repository_id: number;
  owner: string;
  name: string;
  visibility: 'public' | 'private';
  default_branch: string;
  fork: boolean;
  selected: boolean;
  coverage: 'complete' | 'partial' | 'unknown';
}

export interface PersonalDiscoveryResponse {
  discovery: string;
  range: ActivityRange;
  projects: PersonalProject[];
  discovery_complete: boolean;
  expires_at: string;
}

export interface PersonalConsentRequest {
  repositories: number[];
  cross_repository_linking: true;
  /** Separate opt-in. The page does not receive or accept email/alias lists. */
  verified_email_matching?: boolean;
}

export interface PersonalConsentResponse {
  consent: string;
  cross_repository_linking: true;
  verified_email_matching: boolean;
  version: number;
  expires_at: string;
}

export interface PersonalAnalysisRequest extends ActivityRange {
  discovery: string;
  repositories: number[];
  consent: string;
  /** Partial sources need an explicit choice, never implicit success. */
  continue_with_available?: boolean;
}

export interface PersonalCoverage {
  matched_commits: number;
  unmatched_commits: number;
  sources_ready: number;
  sources_selected: number;
  complete: boolean;
}

/** Existing pull_requests: true remains basic. Full mode is additive and server-enforced. */
export type PullRequestMode = 'basic' | 'full';

/** Sanitized display hints only. Servers still authorize every new paid operation. */
export type PaidActivityFeature = 'user-activity' | 'pull-request-reviews';
