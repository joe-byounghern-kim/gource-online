// SPDX-License-Identifier: MIT

import type { PullRequestMode } from './activity.js';

// The clone service API as clients see it: POST /api/analyze starts a job,
// GET /api/jobs/:id/events streams its progress (server-sent events),
// GET /api/jobs/:id answers the same history as JSON, and the log is a
// custom log at the URL of the `log` event.

export type ServerStage = 'queued' | 'resolving' | 'cloning' | 'counting' | 'history';

export interface AnalyzeRequest {
  url: string;
  branch?: string;
  since?: string;
  until?: string;
  path?: string;
  /** Merge pull request events in (GitHub, when the server offers it). */
  pull_requests?: boolean;
  /** Additive paid hosted mode. Omitted mode and old true requests remain basic. */
  pull_request_mode?: PullRequestMode;
  /** Receipt for configured account-owned rich acquisition, never browser authority. */
  consent?: string;
  idempotency_key?: string;
  /** Explicit acknowledgement of reduced sources, never implicit success. */
  continue_with_available?: boolean;
}

export interface AnalyzeResponse {
  job: string;
  /** `visibility` is `"private"` for a private repository (private mode, F2). */
  repo: { host: string; path: string; display: string; visibility?: 'private' };
  cached: boolean;
  /** F1 AI-29: whether the server classified AI attributions into the log. */
  ai_detect?: boolean;
  /** Pull request events are merged into the log. */
  pull_requests?: boolean;
  /** Additive paid hosted mode. Omitted mode and old true requests remain basic. */
  pull_request_mode?: PullRequestMode;
}

export interface ProgressEvent {
  stage: ServerStage;
  label?: string;
  percent?: number;
  current?: number;
  total?: number;
  bytes?: number;
  /** Jobs ahead of this one (`queued` only). */
  position?: number;
}

export interface LogEvent {
  url: string;
  commits_total?: number;
}

export interface DoneEvent {
  commits: number;
  bytes: number;
  head?: string;
  cached: boolean;
  elapsed_ms?: number;
}

export type JobErrorCode =
  | 'not_found'
  | 'too_large'
  | 'timeout'
  | 'git_failed'
  | 'empty'
  | 'internal'
  // Private mode (F2 PR-88).
  | 'auth_required'
  | 'csrf_rejected'
  | 'provider_unavailable';

export interface JobErrorEvent {
  code: JobErrorCode | string;
  message: string;
}

/** Every stage, in order. */
export const SERVER_STAGES: readonly ServerStage[] = ['queued', 'resolving', 'cloning', 'counting', 'history'];

/** `GET /api/jobs/:id` as a clone service answers it, before validation. */
export interface JobStateRaw {
  id?: string;
  state?: 'running' | 'done' | 'error';
  events?: Array<{ event: string; data: unknown }>;
}
