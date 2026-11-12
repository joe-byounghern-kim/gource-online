// SPDX-License-Identifier: MIT
import type { ActivityRange, PaidActivityFeature } from './activity.js';
import { parseRepoUrl, validateBranch, validateDate } from './repo-url.js';
import { readActivityResultProvenance, type ActivityResultProvenance } from './private-activity.js';

export interface ActivityRepository {
  /** Opaque discovery-local receipt, not an account or identity id. */
  id: string;
  url: string;
  name: string;
  default_branch: string;
  head: string;
  fork: boolean;
  visibility: 'public';
  /** GitHub contribution hint for the requested range, not analysis/matching evidence. Null means unavailable or truncated. */
  commit_count?: number | null;
  /** Latest observed contribution in the range. Never the repository's unrelated head date. */
  last_activity_at?: string | null;
}
export interface HostedPersonalDiscoveryRequest { range: ActivityRange; manual_urls?: string[] }
export interface HostedPersonalDiscoveryResponse {
  discovery: string;
  repositories: readonly ActivityRepository[];
  coverage: 'incomplete';
  expires_at: string;
}
export interface ActivityConsentRequest {
  discovery: string;
  repository_ids: string[];
  purpose: PaidActivityFeature;
  link_self: true;
  /** Separate explicit opt-in. Capability alone never grants email matching. Emails remain server-only. */
  verified_email_match?: boolean;
  /** Personal activity only: also show the other contributors of the selected repositories, under anonymous actors that
   * name no one (cloud #96). Recorded in the consent; the analysis request must repeat it. */
  other_contributors?: true;
}
export interface ActivityConsentResponse { consent: string; expires_at: string }
export interface PersonalAnalyzeRequest {
  discovery: string;
  consent: string;
  repository_ids: string[];
  range: ActivityRange;
  idempotency_key: string;
  continue_with_available?: boolean;
  /** Must match the consent. */
  other_contributors?: true;
}
export type ActivityJobStatus = 'queued' | 'running' | 'ready' | 'failed' | 'cancelled';
export const ACTIVITY_COVERAGE_CODES = Object.freeze([
  'discovery-incomplete', 'mailmap-unavailable', 'ai-evidence-unavailable', 'commit-files-ceiling',
  'lifecycle-incomplete', 'reviews-incomplete', 'code-area-unknown', 'coverage-unknown',
  'snapshot-diff-anchors', 'merge-association-unknown', 'project-unavailable',
] as const);
export type ActivityCoverageCode = typeof ACTIVITY_COVERAGE_CODES[number];
export type ActivityProjectStage = 'queued' | 'fetching' | 'matching' | 'ready' | 'failed';
export interface ActivityProjectProgress { id: string; name: string; stage: ActivityProjectStage }
/** Account-owned display state, never authority to acquire sources or extend retention. */
export interface ActivityAnalysisState {
  feature: PaidActivityFeature;
  range: ActivityRange;
  expires_at: string;
  revision: number;
  projects: readonly ActivityProjectProgress[];
  completed: number;
  total: number;
  estimate_seconds: null;
  coverage: readonly ActivityCoverageCode[];
  failure?: ActivityErrorCode;
  retry?: ActivityAnalysisRetry;
}
/** Explicitly confirmed new analysis of exactly `projects`, never revival of the failed job. */
export interface ActivityAnalysisRetry {
  kind: 'new-analysis'; requires_confirmation: true; reacquires_sources: true; additional_analysis_jobs: 1;
  /** Project IDs, in analysis order, that the new selection keeps. Failed projects are excluded. */
  projects: readonly string[];
}
export interface ActivityJobResponse {
  job: string;
  status: ActivityJobStatus;
  cached: boolean;
  log_url?: string;
  /** Closed, bounded warning codes, never raw provider errors. */
  coverage?: readonly ActivityCoverageCode[];
  /** Optional for compatibility with existing job responses. */
  analysis?: ActivityAnalysisState;
  /** Present for results that include a private or internal source: owner-only, never publishable. */
  provenance?: ActivityResultProvenance;
}
/** GET /api/activity/resume looks up only the authenticated current account. */
export interface ActivityResumeResponse { analysis: ActivityJobResponse | null }
/** D9 JSON is validated and bounded by the server, never a source of authority. */
export interface SnapshotCreateRequest { job: string; recipe: Record<string, unknown>; idempotency_key: string }
export interface SnapshotResponse {
  snapshot: string;
  state: 'owner-only' | 'published';
  expires_at: string;
  /** Canonical site-relative playback link. Present only for published snapshots. Never a signed storage URL. */
  share_url?: string;
  /** Present for snapshots that include a private or internal source. Such a snapshot is owner-only for its lifetime. */
  provenance?: ActivityResultProvenance;
}
export const SNAPSHOT_PAGE_SIZE = 10;
export interface SnapshotListEntry extends SnapshotResponse {
  created_at: string;
  project_count: number;
  /** Older saved results may not have listing metadata. Null means unknown, not an unbounded period. */
  range: ActivityRange | null;
  size_bytes: number;
}
export interface SnapshotListResponse { snapshots: readonly SnapshotListEntry[]; next_cursor: string | null }
/** A keyset position is an opaque display receipt, never account authority. */
export function validSnapshotCursor(value: unknown): value is string {
  return typeof value === 'string' && /^\d{1,16}_[A-Za-z0-9_-]{1,128}$/.test(value)
    && Number.isSafeInteger(Number(value.slice(0, value.indexOf('_'))));
}
export function snapshotShareUrl(snapshot: string): string {
  if (!id(snapshot)) throw new RangeError('invalid-snapshot');
  return '/?log=' + encodeURIComponent(`/api/snapshots/${snapshot}/log`);
}
export interface SnapshotPreviewResponse {
  preview: string;
  names: readonly string[];
  paths: readonly string[];
  requires_alias_disclosure: boolean;
}
export interface SnapshotPublishRequest { preview: string; disclose_aliases: boolean }

/** Session-only display hints, never authorization. Usage is cumulative, not a renewal period. */
export interface ActivityQuota {
  feature: PaidActivityFeature;
  used: number;
  limit: number;
  remaining: number;
}
/** Optional session fields allow older services to omit the account summary. */
export interface ActivityAccountSummary {
  activity_quotas?: readonly ActivityQuota[];
  beta_access_requested?: boolean;
}
/** POST /api/activity/access-request with {}. Repeated requests return the same result. */
export interface ActivityAccessRequestResponse { requested: true }

export function readActivityQuotas(value: unknown): readonly ActivityQuota[] | null {
  try {
    if (!Array.isArray(value) || value.length !== 2) return null;
    const seen = new Set<string>();
    const quotas: ActivityQuota[] = [];
    for (const raw of value) {
      const row = object(raw);
      if (!row || (row.feature !== 'user-activity' && row.feature !== 'pull-request-reviews') || seen.has(row.feature)) return null;
      for (const key of ['used', 'limit', 'remaining']) {
        if (typeof row[key] !== 'number' || !Number.isSafeInteger(row[key]) || row[key] < 0) return null;
      }
      const used = row.used as number, limit = row.limit as number, remaining = row.remaining as number;
      if (remaining !== Math.max(0, limit - used)) return null;
      quotas.push(Object.freeze({ feature: row.feature, used, limit, remaining }));
      seen.add(row.feature);
    }
    return Object.freeze(quotas);
  } catch { return null; }
}

/** Frozen D9 settings are data, never URLs, credentials or account authority. */
export const SNAPSHOT_OPTION_KEYS = Object.freeze([
  'background-colour', 'dir-colour', 'filename-colour', 'font-colour', 'highlight-colour', 'selection-colour',
  'bloom-intensity', 'bloom-multiplier', 'hash-seed', 'seconds-per-day', 'time-scale',
  'auto-skip-seconds', 'max-file-lag', 'camera-mode', 'follow-user', 'padding',
  'disable-auto-rotate', 'max-user-speed', 'user-friction', 'elasticity', 'loop',
  'no-time-travel', 'hide', 'key', 'hide-pull-requests', 'pr-flow-hud',
  'pull-request-style', 'pull-request-mode', 'title', 'font-scale', 'font-size',
  'file-font-size', 'dir-font-size', 'user-font-size', 'dir-name-depth', 'dir-name-position',
  'highlight-dirs', 'highlight-users', 'highlight-user', 'file-extensions', 'user-scale',
  'file-idle-time', 'filename-time', 'file-filter', 'file-show-filter', 'user-filter',
  'user-show-filter', 'start-date', 'stop-date', 'ai-style', 'agent-key', 'ai-filter',
  'decor-motion', 'user-style', 'max-files',
] as const);
export const SNAPSHOT_RECORD_KEYS = Object.freeze([
  'record-framerate', 'record-viewport', 'record-ui-scale', 'record-format', 'record-layout',
  'record-quality', 'record-bitrate', 'record-keyframe-seconds', 'record-hardware',
  'record-tail-seconds', 'record-max-seconds', 'record-metadata', 'record-name',
] as const);
export const ACTIVITY_ERROR_CODES = Object.freeze([
  'auth_required', 'entitlement_required', 'feature_unavailable', 'csrf_rejected',
  'consent_required', 'coverage_incomplete', 'quota_exceeded', 'not_found',
  'invalid_option', 'preview_too_large', 'provider_unavailable', 'busy', 'activity_failed',
  // Private repositories: no usable installation yet, the GitHub sign-in must be renewed (SSO, a revoked or
  // different authorization), and a private source this result needs can no longer be read.
  'private_install_required', 'private_reauthorize', 'private_access_lost',
] as const);
export type ActivityErrorCode = typeof ACTIVITY_ERROR_CODES[number];

export interface ActivityBusyDetails {
  reason: 'account_running' | 'global_busy' | 'slot_held';
  /** Relative seconds from receipt, avoiding client/server clock skew. */
  retry_after: number;
  expires_at: string;
}
export interface ActivityDiscoveryProgress {
  type: 'progress';
  stage: 'repositories' | 'contributions' | 'metadata';
  completed: number;
  total: number | null;
}
/** POST /api/activity/discover/events uses UTF-8 application/x-ndjson, one frame per line.
 * Admission errors use the ordinary HTTP JSON envelope before streaming begins.
 * Exactly one terminal result/error is required. Cancellation aborts the request transport.
 */
export type ActivityDiscoveryEvent = ActivityDiscoveryProgress
  | { type: 'result'; result: HostedPersonalDiscoveryResponse }
  | { type: 'error'; code: ActivityErrorCode };

const object = (value: unknown): Record<string, unknown> | null =>
  value !== null && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : null;
const id = (value: unknown): value is string => typeof value === 'string' && /^[A-Za-z0-9_-]{1,128}$/.test(value);
const expiry = (value: unknown): value is string => {
  if (typeof value !== 'string' || !/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d{3})?Z$/.test(value)) return false;
  const time = Date.parse(value);
  return Number.isFinite(time) && new Date(time).toISOString() === (value.length === 20 ? value.replace('Z', '.000Z') : value);
};
const pathText = (value: unknown, limit: number): value is string =>
  typeof value === 'string' && value.length > 0 && value.length <= limit && !/[\u0000-\u001f\u007f]/.test(value);
const text = (value: unknown, limit: number): value is string => {
  if (!pathText(value, limit)) return false;
  // Conservative and linear. A preview with any @ in a name stays owner-only
  // instead of risking an embedded or multiple-@ email substring reaching the page.
  // A path is the repository's own file name and may hold one (`logo@2x.png`),
  // which is what a publication preview shows its owner before anything is public.
  return !value.includes('@');
};

/** Copy only the public contract. No raw identity or provider fields propagate. */
export function readActivityDiscovery(value: unknown): HostedPersonalDiscoveryResponse | null {
  try {
    const o = object(value);
    if (!o || !id(o.discovery) || !expiry(o.expires_at) || o.coverage !== 'incomplete' ||
      !Array.isArray(o.repositories) || o.repositories.length > 50) return null;
    const repositories: ActivityRepository[] = [];
    const seen = new Set<string>();
    for (const raw of o.repositories) {
      const r = object(raw);
      if (!r || !id(r.id) || seen.has(r.id) || typeof r.url !== 'string' || r.url.length > 512 ||
        typeof r.name !== 'string' || typeof r.default_branch !== 'string' || !r.default_branch ||
        r.default_branch.length > 256 || validateBranch(r.default_branch) || typeof r.head !== 'string' ||
        !/^(?:[a-f0-9]{40}|[a-f0-9]{64})$/.test(r.head) || typeof r.fork !== 'boolean' || r.visibility !== 'public') return null;
      // Response URLs are canonical names, not clone input. Consume a disposable
      // clone suffix so a literal .git in the repository name is not removed.
      const parsed = parseRepoUrl(`${r.url}.git`);
      if (!parsed.ok || parsed.repo.host !== 'github.com' || parsed.repo.path !== r.name ||
        r.url !== `https://github.com/${r.name}`) return null;
      if (r.commit_count !== undefined && r.commit_count !== null && (!Number.isSafeInteger(r.commit_count) || (r.commit_count as number) < 0)) return null;
      if (r.last_activity_at !== undefined && r.last_activity_at !== null && !expiry(r.last_activity_at)) return null;
      repositories.push(Object.freeze({ id: r.id, url: r.url, name: r.name, default_branch: r.default_branch,
        head: r.head, fork: r.fork, visibility: 'public',
        ...(r.commit_count !== undefined ? { commit_count: r.commit_count as number | null } : {}),
        ...(r.last_activity_at !== undefined ? { last_activity_at: r.last_activity_at as string | null } : {}) }));
      seen.add(r.id);
    }
    return Object.freeze({ discovery: o.discovery, repositories: Object.freeze(repositories), coverage: 'incomplete', expires_at: o.expires_at });
  } catch { return null; }
}

export function readActivityBusy(value: unknown): ActivityBusyDetails | null {
  try {
    const o = object(value);
    if (!o || (o.reason !== 'account_running' && o.reason !== 'global_busy' && o.reason !== 'slot_held') ||
      !Number.isSafeInteger(o.retry_after) || (o.retry_after as number) < 1 || (o.retry_after as number) > 600 || !expiry(o.expires_at)) return null;
    return Object.freeze({ reason: o.reason, retry_after: o.retry_after as number, expires_at: o.expires_at });
  } catch { return null; }
}

export function readActivityDiscoveryEvent(value: unknown): ActivityDiscoveryEvent | null {
  try {
    const o = object(value);
    if (!o) return null;
    if (o.type === 'result') {
      const result = readActivityDiscovery(o.result);
      return result ? Object.freeze({ type: 'result', result }) : null;
    }
    if (o.type === 'error') {
      return typeof o.code === 'string' && (ACTIVITY_ERROR_CODES as readonly string[]).includes(o.code)
        ? Object.freeze({ type: 'error', code: o.code as ActivityErrorCode }) : null;
    }
    if (o.type !== 'progress' || (o.stage !== 'repositories' && o.stage !== 'contributions' && o.stage !== 'metadata')) return null;
    const limit = o.stage === 'metadata' ? 50 : 500;
    if (!Number.isSafeInteger(o.completed) || (o.completed as number) < 0 || (o.completed as number) > limit ||
      (o.total !== null && (!Number.isSafeInteger(o.total) || (o.total as number) < (o.completed as number) || (o.total as number) > limit))) return null;
    return Object.freeze({ type: 'progress', stage: o.stage, completed: o.completed as number, total: o.total as number | null });
  } catch { return null; }
}

// A new acquisition can help only with source/provider failures, not missing session, consent, grant or quota.
const PARTIAL_FAILURES: readonly string[] = ['activity_failed', 'provider_unavailable', 'feature_unavailable'];
/** The only valid retry receipt for this failure and these projects, if any. */
export function activityAnalysisRetry(failure: string | undefined, projects: readonly ActivityProjectProgress[]): ActivityAnalysisRetry | undefined {
  const kept = failure === 'coverage_incomplete' ? projects
    : failure !== undefined && PARTIAL_FAILURES.includes(failure) && projects.some(p => p.stage === 'ready') && projects.some(p => p.stage === 'failed')
      ? projects.filter(p => p.stage !== 'failed') : undefined;
  return kept && Object.freeze({ kind: 'new-analysis', requires_confirmation: true, reacquires_sources: true, additional_analysis_jobs: 1,
    projects: Object.freeze(kept.map(p => p.id)) });
}

/** Validate, copy and freeze the latest public state, not a history of inferred stages. */
export function readActivityAnalysis(value: unknown): ActivityAnalysisState | null {
  try {
    const o = object(value);
    if (!o || (o.feature !== 'user-activity' && o.feature !== 'pull-request-reviews') ||
      !expiry(o.expires_at) || !Number.isSafeInteger(o.revision) || (o.revision as number) < 0 || (o.revision as number) > 1024 ||
      !Array.isArray(o.projects) || o.projects.length < 1 || o.projects.length > 50 ||
      (o.feature === 'pull-request-reviews' && o.projects.length !== 1) ||
      o.total !== o.projects.length || !Number.isSafeInteger(o.completed) || o.estimate_seconds !== null) return null;
    const range = object(o.range);
    if (!range || typeof range.since !== 'string' || range.since.length !== 10 || !expiry(`${range.since}T00:00:00Z`) ||
      typeof range.until !== 'string' || range.until.length !== 10 || !expiry(`${range.until}T00:00:00Z`) || range.since > range.until) return null;
    const years = Number(range.until.slice(0, 4)) - Number(range.since.slice(0, 4));
    if (years > 10 || (years === 10 && range.until.slice(5) >= range.since.slice(5))) return null;
    const projects: ActivityProjectProgress[] = [];
    const ids = new Set<string>(), names = new Set<string>();
    let completed = 0;
    for (const raw of o.projects) {
      const r = object(raw);
      if (!r || !id(r.id) || r.id.trim() !== r.id || ids.has(r.id) || typeof r.name !== 'string' || r.name.length > 201 ||
        names.has(r.name) || typeof r.stage !== 'string' || !['queued', 'fetching', 'matching', 'ready', 'failed'].includes(r.stage)) return null;
      // Match canonical owner/repo exactly, preserving a literal .git repository suffix.
      const parsed = parseRepoUrl(`https://github.com/${r.name}.git`);
      if (!parsed.ok || parsed.repo.path !== r.name) return null;
      projects.push(Object.freeze({ id: r.id, name: r.name, stage: r.stage as ActivityProjectStage }));
      ids.add(r.id);
      names.add(r.name);
      if (r.stage === 'ready') completed++;
    }
    if (o.completed !== completed || !Array.isArray(o.coverage) || o.coverage.length > ACTIVITY_COVERAGE_CODES.length) return null;
    const coverage: ActivityCoverageCode[] = [];
    for (const code of o.coverage) {
      if (typeof code !== 'string' || !(ACTIVITY_COVERAGE_CODES as readonly string[]).includes(code) || coverage.includes(code as ActivityCoverageCode)) return null;
      coverage.push(code as ActivityCoverageCode);
    }
    if ('failure' in o && (typeof o.failure !== 'string' || !(ACTIVITY_ERROR_CODES as readonly string[]).includes(o.failure))) return null;
    const retry = activityAnalysisRetry(o.failure as string | undefined, projects);
    // A partial failure must carry its receipt so continuing is a choice, not a guess.
    if (retry && o.failure !== 'coverage_incomplete' && !('retry' in o)) return null;
    if ('retry' in o) {
      const r = object(o.retry);
      if (!retry || !r || r.kind !== 'new-analysis' || r.requires_confirmation !== true ||
        r.reacquires_sources !== true || r.additional_analysis_jobs !== 1 || !Array.isArray(r.projects) ||
        r.projects.length !== retry.projects.length || r.projects.some((value, i) => value !== retry.projects[i])) return null;
    }
    return Object.freeze({ feature: o.feature, range: Object.freeze({ since: range.since, until: range.until }),
      expires_at: o.expires_at, revision: o.revision as number, projects: Object.freeze(projects), completed, total: projects.length,
      estimate_seconds: null, coverage: Object.freeze(coverage),
      ...('failure' in o ? { failure: o.failure as ActivityErrorCode } : {}), ...('retry' in o ? { retry } : {}) });
  } catch { return null; }
}

export function readActivityJob(value: unknown): ActivityJobResponse | null {
  try {
    const o = object(value);
    if (!o || !id(o.job) || typeof o.status !== 'string' || !['queued', 'running', 'ready', 'failed', 'cancelled'].includes(o.status) || typeof o.cached !== 'boolean') return null;
    if (o.log_url !== undefined && o.log_url !== `/api/activity/jobs/${o.job}/log`) return null;
    if (o.status === 'ready' && o.log_url === undefined) return null;
    const analysis = 'analysis' in o ? readActivityAnalysis(o.analysis) : undefined;
    if (analysis === null) return null;
    if (o.coverage !== undefined) {
      if (!Array.isArray(o.coverage) || o.coverage.length > ACTIVITY_COVERAGE_CODES.length) return null;
      for (const entry of o.coverage) {
        if (typeof entry !== 'string' || !(ACTIVITY_COVERAGE_CODES as readonly string[]).includes(entry)) return null;
      }
    }
    const provenance = o.provenance === undefined ? undefined : readActivityResultProvenance(o.provenance);
    if (provenance === null || provenance?.visibility === 'public') return null;
    return Object.freeze({ job: o.job, status: o.status as ActivityJobStatus, cached: o.cached,
      ...(o.log_url !== undefined ? { log_url: o.log_url as string } : {}),
      ...(provenance ? { provenance } : {}),
      ...(analysis ? { analysis } : {}),
      ...(o.coverage !== undefined ? { coverage: Object.freeze([...(o.coverage as ActivityCoverageCode[])]) } : {}) });
  } catch { return null; }
}

export function readActivityResume(value: unknown): ActivityResumeResponse | null {
  try {
    const o = object(value);
    if (!o) return null;
    if (o.analysis === null) return Object.freeze({ analysis: null });
    const analysis = readActivityJob(o.analysis);
    if (!analysis?.analysis || !['running', 'ready', 'failed'].includes(analysis.status)) return null;
    return Object.freeze({ analysis });
  } catch { return null; }
}

export function readActivitySnapshot(value: unknown): SnapshotResponse | null {
  try {
    const o = object(value);
    if (!o || !id(o.snapshot) || !expiry(o.expires_at) || (o.state !== 'owner-only' && o.state !== 'published')) return null;
    if (o.share_url !== undefined && (o.state !== 'published' || o.share_url !== snapshotShareUrl(o.snapshot))) return null;
    const provenance = o.provenance === undefined ? undefined : readActivityResultProvenance(o.provenance);
    // Private provenance is sticky: such a snapshot is never published, whatever the source's visibility is now.
    if (provenance === null || provenance?.visibility === 'public' || (provenance && o.state !== 'owner-only')) return null;
    return Object.freeze({ snapshot: o.snapshot, state: o.state, expires_at: o.expires_at,
      ...(o.share_url !== undefined ? { share_url: o.share_url as string } : {}),
      ...(provenance ? { provenance } : {}) });
  } catch { return null; }
}

export function readActivitySnapshotList(value: unknown): SnapshotListResponse | null {
  try {
    const o = object(value);
    if (!o || !Array.isArray(o.snapshots) || o.snapshots.length > SNAPSHOT_PAGE_SIZE ||
      (o.next_cursor !== null && !validSnapshotCursor(o.next_cursor))) return null;
    const snapshots: SnapshotListEntry[] = [], seen = new Set<string>();
    for (const raw of o.snapshots) {
      const entry = object(raw), snapshot = readActivitySnapshot(raw);
      if (!entry || !snapshot || seen.has(snapshot.snapshot) || !expiry(entry.created_at) ||
        Date.parse(entry.created_at) >= Date.parse(snapshot.expires_at) ||
        !Number.isInteger(entry.project_count) || Number(entry.project_count) < 1 || Number(entry.project_count) > 50 ||
        !Number.isSafeInteger(entry.size_bytes) || Number(entry.size_bytes) < 0 || Number(entry.size_bytes) > 256 * 1024 * 1024) return null;
      let range: ActivityRange | null = null;
      if (entry.range !== null) {
        const r = object(entry.range);
        if (!r || typeof r.since !== 'string' || typeof r.until !== 'string' ||
          !/^\d{4}-\d{2}-\d{2}$/.test(r.since) || !/^\d{4}-\d{2}-\d{2}$/.test(r.until) ||
          validateDate(r.since) || validateDate(r.until) || r.since > r.until) return null;
        const anniversary = new Date(`${r.since}T00:00:00Z`); anniversary.setUTCFullYear(anniversary.getUTCFullYear() + 10);
        if (Date.parse(`${r.until}T00:00:00Z`) >= +anniversary) return null;
        range = Object.freeze({ since: r.since, until: r.until });
      }
      snapshots.push(Object.freeze({ ...snapshot, created_at: entry.created_at, project_count: Number(entry.project_count),
        size_bytes: Number(entry.size_bytes), range }));
      seen.add(snapshot.snapshot);
    }
    return Object.freeze({ snapshots: Object.freeze(snapshots), next_cursor: o.next_cursor as string | null });
  } catch { return null; }
}

export function readActivityConsent(value: unknown): ActivityConsentResponse | null {
  try {
    const o = object(value);
    return o && id(o.consent) && expiry(o.expires_at)
      ? Object.freeze({ consent: o.consent, expires_at: o.expires_at }) : null;
  } catch { return null; }
}

export function readActivityPreview(value: unknown): SnapshotPreviewResponse | null {
  try {
    const o = object(value);
    if (!o || !id(o.preview) || typeof o.requires_alias_disclosure !== 'boolean' ||
      !Array.isArray(o.names) || o.names.length > 25000 ||
      !Array.isArray(o.paths) || o.paths.length > 25000) return null;
    const encoder = new TextEncoder();
    let bytes = 0;
    // for..of visits holes as undefined, unlike Array.every. Count aggregate
    // UTF8 bytes before copying either array into the browser-facing receipt.
    for (const [entries, limit, valid] of [[o.names, 256, text], [o.paths, 4096, pathText]] as const) {
      for (const entry of entries) {
        if (typeof entry !== 'string' || entry.length > limit) return null;
        bytes += encoder.encode(entry).byteLength;
        if (bytes > ACTIVITY_JSON_MAX_BYTES) return null;
        if (!valid(entry, limit)) return null;
      }
    }
    return Object.freeze({ preview: o.preview, names: Object.freeze([...o.names]), paths: Object.freeze([...o.paths]),
      requires_alias_disclosure: o.requires_alias_disclosure });
  } catch { return null; }
}

/** All browser activity JSON, including publication previews, is capped before parsing. */
export const ACTIVITY_JSON_MAX_BYTES = 1024 * 1024;
