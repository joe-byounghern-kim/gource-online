// SPDX-License-Identifier: MIT

// The product analytics contract between the page and a deployment's edge:
// which events exist and which properties each may carry. The page only
// sends what is listed here; the edge drops anything else before
// forwarding, so a buggy or hostile client cannot add fields (a file name,
// a log line) that the schema does not allow.

export type PropType = 'string' | 'number' | 'boolean';
export type PropValue = string | number | boolean;

const COMMON = { app_version: 'string', screen: 'string' } as const;

export const ANALYTICS_EVENTS = {
  page_view: { ...COMMON, referrer_host: 'string', viewport: 'string', dpr: 'number', webgl2: 'boolean' },
  analysis_started: { ...COMMON, host: 'string', repo: 'string', opt_branch: 'boolean', opt_dates: 'boolean', opt_path: 'boolean', opt_pull_requests: 'boolean' },
  analysis_finished: { ...COMMON, cached: 'boolean', commits: 'number', log_size: 'string', elapsed_ms: 'number', queue_ms: 'number' },
  analysis_failed: { ...COMMON, code: 'string', stage: 'string', elapsed_ms: 'number' },
  log_opened: { ...COMMON, source: 'string', log_size: 'string', commits: 'number' },
  playback_started: { ...COMMON, first_frame_ms: 'number', fps_p50_5s: 'number' },
  playback_ended: { ...COMMON, watched_s: 'number' },
  recording_started: { ...COMMON, resolution: 'string', fps: 'number', duration_s: 'number' },
  recording_finished: { ...COMMON, resolution: 'string', fps: 'number', duration_s: 'number' },
  recording_failed: { ...COMMON, resolution: 'string', fps: 'number', duration_s: 'number', reason: 'string' },
  feature_requested: { ...COMMON, feature: 'string' },
  client_error: { ...COMMON, kind: 'string', message: 'string' },
} as const satisfies Record<string, Record<string, PropType>>;

export type AnalyticsEvent = keyof typeof ANALYTICS_EVENTS;

export interface AnalyticsRecord {
  event: AnalyticsEvent;
  /** Milliseconds since the epoch, on the client. */
  ts: number;
  props: Record<string, PropValue>;
}

export interface AnalyticsBatch {
  /** The anonymous visitor id: a UUID v4. */
  id: string;
  events: AnalyticsRecord[];
}

export const MAX_BATCH_EVENTS = 50;
export const MAX_STRING = 200;
const UUID_V4 = /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/;

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === 'object' && v !== null && !Array.isArray(v);

function cleanProps(schema: Record<string, PropType>, raw: unknown): Record<string, PropValue> {
  const out: Record<string, PropValue> = {};
  if (!isObject(raw)) return out;
  for (const [key, type] of Object.entries(schema)) {
    const v = raw[key];
    if (type === 'string' && typeof v === 'string') out[key] = v.slice(0, MAX_STRING);
    else if (type === 'number' && typeof v === 'number' && Number.isFinite(v)) out[key] = v;
    else if (type === 'boolean' && typeof v === 'boolean') out[key] = v;
  }
  return out;
}

/**
 * The batch with everything outside the schema removed, or null when the
 * id is not a lowercase UUID v4 or no event is left.
 */
export function sanitizeBatch(input: unknown): AnalyticsBatch | null {
  if (!isObject(input) || typeof input.id !== 'string' || !UUID_V4.test(input.id) || !Array.isArray(input.events)) return null;
  const events: AnalyticsRecord[] = [];
  for (const raw of input.events) {
    if (events.length >= MAX_BATCH_EVENTS) break;
    if (!isObject(raw) || typeof raw.event !== 'string' || !Object.hasOwn(ANALYTICS_EVENTS, raw.event)) continue;
    if (typeof raw.ts !== 'number' || !Number.isFinite(raw.ts)) continue;
    const event = raw.event as AnalyticsEvent;
    events.push({ event, ts: raw.ts, props: cleanProps(ANALYTICS_EVENTS[event], raw.props) });
  }
  return events.length ? { id: input.id, events } : null;
}

const SIZE_STEPS: [number, string][] = [
  [10_000, '<10KB'],
  [100_000, '<100KB'],
  [1_000_000, '<1MB'],
  [10_000_000, '<10MB'],
  [100_000_000, '<100MB'],
];

/** A coarse size class, so a log's exact size never identifies it. */
export function sizeBucket(bytes: number): string {
  for (const [limit, label] of SIZE_STEPS) if (bytes < limit) return label;
  return '>=100MB';
}

/** The viewport class by its shorter side. */
export function viewportBucket(width: number, height: number): string {
  const side = Math.min(width, height);
  if (side < 480) return '<480';
  if (side < 768) return '<768';
  if (side < 1200) return '<1200';
  return '>=1200';
}
