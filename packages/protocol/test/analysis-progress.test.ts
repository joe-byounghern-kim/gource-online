// SPDX-License-Identifier: MIT
import { test, expect } from '@playwright/test';
import * as protocol from '../src/index.js';
import type { ActivityAnalysisState, ActivityProjectProgress, ActivityProjectStage, ActivityResumeResponse } from '../src/index.js';

// Namespace lookup keeps the first regression run behavioral when the additive exports are absent.
const api = protocol as unknown as {
  readActivityAnalysis(value: unknown): ActivityAnalysisState | null;
  readActivityResume(value: unknown): ActivityResumeResponse | null;
  activityAnalysisRetry(failure: string | undefined, projects: readonly ActivityProjectProgress[]): ActivityAnalysisState['retry'];
};
const project: ActivityProjectProgress = { id: 'repo_1', name: 'example/project', stage: 'queued' };
const analysis: ActivityAnalysisState = {
  feature: 'user-activity', range: { since: '2025-01-01', until: '2025-12-31' },
  expires_at: '2026-10-19T14:00:00Z', revision: 0, projects: [project],
  completed: 0, total: 1, estimate_seconds: null, coverage: ['discovery-incomplete'],
};
const retry = { kind: 'new-analysis', requires_confirmation: true, reacquires_sources: true, additional_analysis_jobs: 1, projects: ['repo_1'] };
const job = { job: 'job_1', status: 'running', cached: false };

test('analysis copies and deeply freezes only public display state', () => {
  expect(api.readActivityAnalysis).toBeDefined();
  const input = { ...analysis, range: { ...analysis.range, account_id: 77 },
    projects: [{ ...project, email: 'seeded-private@example.invalid', provider_response: { token: 'SEEDED_SECRET' } }],
    coverage: [...analysis.coverage], failure: 'coverage_incomplete', retry: { ...retry, token: 'SEEDED_SECRET' },
    identity_index: ['SEEDED_SECRET'], account_id: 77 };
  const result = api.readActivityAnalysis(input);
  expect(result).toEqual({ ...analysis, failure: 'coverage_incomplete', retry });
  expect(JSON.stringify(result)).not.toMatch(/seeded-private|SEEDED_SECRET|account_id|identity_index|provider_response/);
  expect(result).not.toBe(input);
  for (const [copy, original] of [[result?.range, input.range], [result?.projects, input.projects],
    [result?.projects[0], input.projects[0]], [result?.coverage, input.coverage], [result?.retry, input.retry]]) {
    expect(copy).not.toBe(original);
    expect(Object.isFrozen(copy)).toBe(true);
  }
  expect(Object.isFrozen(result)).toBe(true);
  input.range.since = '2024-01-01';
  input.projects[0]!.name = 'other/project';
  input.coverage.length = 0;
  expect(result?.range).toEqual(analysis.range);
  expect(result?.projects).toEqual([project]);
  expect(result?.coverage).toEqual(['discovery-incomplete']);
});

test('project stages are closed and completion counts ready projects only', () => {
  expect(api.readActivityAnalysis).toBeDefined();
  const stages: ActivityProjectStage[] = ['queued', 'fetching', 'matching', 'ready', 'failed'];
  const projects = stages.map((stage, i) => ({ id: `repo_${i}`, name: `example/project${i}`, stage }));
  const state = { ...analysis, projects, completed: 1, total: 5 };
  expect(api.readActivityAnalysis(state)).toEqual(state);
  for (const completed of [0, 2, 5, -1, 0.5, '1', NaN, Infinity])
    expect(api.readActivityAnalysis({ ...state, completed })).toBeNull();
  for (const stage of ['resolving', 'READY', '', null, ['queued'], { toString: () => 'queued' }])
    expect(api.readActivityAnalysis({ ...analysis, projects: [{ ...project, stage }] })).toBeNull();
  for (const total of [0, 2, -1, 1.5, '1', null])
    expect(api.readActivityAnalysis({ ...analysis, total })).toBeNull();
});

test('projects have exact bounded opaque ids and unique safe canonical names', () => {
  expect(api.readActivityAnalysis).toBeDefined();
  const projects = Array.from({ length: 50 }, (_, i) => ({ ...project, id: `repo_${i}`, name: `example/project${i}` }));
  expect(api.readActivityAnalysis({ ...analysis, projects, total: 50 })?.total).toBe(50);
  for (const selected of [[], Array(1), [...projects, { ...project, id: 'extra', name: 'example/extra' }],
    [project, { ...project, name: 'example/other' }], [project, { ...project, id: 'repo_2' }]])
    expect(api.readActivityAnalysis({ ...analysis, projects: selected, total: selected.length })).toBeNull();
  for (const id of ['', ' repo_1', 'repo_1 ', 'repo_1\n', 'repo_1\r\n', '../repo', 'repo@example.invalid', 'x'.repeat(129), 1, null])
    expect(api.readActivityAnalysis({ ...analysis, projects: [{ ...project, id }] })).toBeNull();
  const boundary = { ...project, id: 'X_-'.repeat(42) + 'ab', name: `${'o'.repeat(100)}/${'r'.repeat(100)}` };
  expect(api.readActivityAnalysis({ ...analysis, projects: [boundary] })?.projects).toEqual([boundary]);
  for (const name of [' example/project', 'example/project ', 'example/project\n', 'example/../project',
    'example/project/tree/main', 'example/project/', 'example//project', 'example/project?ref=main',
    'example/project#fragment', 'example/project@example.invalid', 'example/%70roject', 'example/..',
    '.example/project', '-example/project', 'example/a..b', `example/${'x'.repeat(101)}`, 'https://example/project', 1, null])
    expect(api.readActivityAnalysis({ ...analysis, projects: [{ ...project, name }] })).toBeNull();
  for (const name of ['Example/Project', 'example/project.git', 'example/project.GIT', 'example/project.git.git'])
    expect(api.readActivityAnalysis({ ...analysis, projects: [{ ...project, name }] })?.projects[0]?.name).toBe(name);
});

test('rich analysis accepts exactly one selected project and no unknown features', () => {
  expect(api.readActivityAnalysis).toBeDefined();
  expect(api.readActivityAnalysis({ ...analysis, feature: 'pull-request-reviews' })?.feature).toBe('pull-request-reviews');
  expect(api.readActivityAnalysis({ ...analysis, feature: 'pull-request-reviews', total: 2,
    projects: [project, { ...project, id: 'repo_2', name: 'example/other' }] })).toBeNull();
  for (const feature of ['private-repos', '', null, ['user-activity']])
    expect(api.readActivityAnalysis({ ...analysis, feature })).toBeNull();
});

test('revision is a safe bounded integer and ETA is explicitly unknown', () => {
  expect(api.readActivityAnalysis).toBeDefined();
  for (const revision of [0, 1024]) expect(api.readActivityAnalysis({ ...analysis, revision })?.revision).toBe(revision);
  for (const revision of [-1, 1025, 0.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1, '0', null])
    expect(api.readActivityAnalysis({ ...analysis, revision })).toBeNull();
  for (const estimate_seconds of [undefined, 0, 1, 'unknown', NaN, false])
    expect(api.readActivityAnalysis({ ...analysis, estimate_seconds })).toBeNull();
});

test('range requires real inclusive UTC calendar dates strictly under ten calendar years', () => {
  expect(api.readActivityAnalysis).toBeDefined();
  for (const range of [{ since: '2024-02-29', until: '2024-02-29' },
    { since: '2016-10-19', until: '2026-10-18' }, { since: '2000-02-29', until: '2000-03-01' }])
    expect(api.readActivityAnalysis({ ...analysis, range })?.range).toEqual(range);
  for (const range of [null, [], {}, { since: '2025-12-31', until: '2025-01-01' },
    { since: '2016-10-19', until: '2026-10-19' }, { since: '2015-01-01', until: '2026-01-01' },
    ...['2025-02-29', '1900-02-29', '2024-02-30', '2025-04-31', '2025-00-01', '2025-13-01', '2025-01-00',
      '2025-01-32', '2025-1-01', '2025-01-01T00:00:00Z', ' 2025-01-01', '2025-01-01\n', 20250101, null]
      .flatMap(date => [{ since: date, until: '2025-12-31' }, { since: '2025-01-01', until: date }])])
    expect(api.readActivityAnalysis({ ...analysis, range })).toBeNull();
});

test('expiry is a strict real UTC instant without client clock or TTL extension', () => {
  expect(api.readActivityAnalysis).toBeDefined();
  for (const expires_at of ['2024-02-29T12:34:56Z', '2026-10-19T14:00:00.123Z'])
    expect(api.readActivityAnalysis({ ...analysis, expires_at })?.expires_at).toBe(expires_at);
  for (const expires_at of ['2026-02-29T00:00:00Z', '2026-02-31T00:00:00Z', '2026-10-19T24:00:00Z',
    '2026-10-19T14:00:60Z', '2026-10-19T14:00:00+00:00', '2026-10-19T14:00:00z',
    '2026-10-19T14:00:00.1Z', '2026-10-19', '2026-10-19T14:00:00Z\n', null])
    expect(api.readActivityAnalysis({ ...analysis, expires_at })).toBeNull();
});

test('analysis coverage and failure are closed primitive codes with unique bounded coverage', () => {
  expect(api.readActivityAnalysis).toBeDefined();
  expect(api.readActivityAnalysis({ ...analysis, coverage: [] })?.coverage).toEqual([]);
  expect(api.readActivityAnalysis({ ...analysis, coverage: [...protocol.ACTIVITY_COVERAGE_CODES] })?.coverage)
    .toEqual(protocol.ACTIVITY_COVERAGE_CODES);
  for (const coverage of [null, 'coverage-unknown', Array(1), ['raw-provider-error'], [['coverage-unknown']],
    ['coverage-unknown', 'coverage-unknown'], Array(12).fill('coverage-unknown')])
    expect(api.readActivityAnalysis({ ...analysis, coverage })).toBeNull();
  for (const failure of protocol.ACTIVITY_ERROR_CODES)
    expect(api.readActivityAnalysis({ ...analysis, failure })?.failure).toBe(failure);
  for (const failure of [undefined, null, 'raw-provider-error', ['coverage_incomplete']])
    expect(api.readActivityAnalysis({ ...analysis, failure })).toBeNull();
});

test('retry requires incomplete coverage and explicit confirmed new acquisition with one extra analysis job', () => {
  expect(api.readActivityAnalysis).toBeDefined();
  const failed = { ...analysis, failure: 'coverage_incomplete', retry };
  expect(api.readActivityAnalysis(failed)).toEqual(failed);
  expect(api.readActivityAnalysis({ ...analysis, failure: 'coverage_incomplete' }))
    .toEqual({ ...analysis, failure: 'coverage_incomplete' });
  for (const failure of [undefined, 'provider_unavailable', 'activity_failed'])
    expect(api.readActivityAnalysis({ ...failed, failure })).toBeNull();
  for (const change of [{ kind: 'resume' }, { kind: 'new-analysis ' }, { requires_confirmation: false },
    { requires_confirmation: undefined }, { reacquires_sources: false }, { reacquires_sources: 'true' },
    { additional_analysis_jobs: 0 }, { additional_analysis_jobs: 2 }, { additional_analysis_jobs: '1' }])
    expect(api.readActivityAnalysis({ ...failed, retry: { ...retry, ...change } })).toBeNull();
  for (const invalid of [undefined, null, [], {}])
    expect(api.readActivityAnalysis({ ...failed, retry: invalid })).toBeNull();
  for (const projects of [undefined, [], ['repo_2'], ['repo_1', 'repo_1'], 'repo_1'])
    expect(api.readActivityAnalysis({ ...failed, retry: { ...retry, projects } })).toBeNull();
});

test('partial failure offers only an explicit new analysis that excludes the failed projects', () => {
  expect(api.activityAnalysisRetry).toBeDefined();
  const projects: ActivityProjectProgress[] = [
    { id: 'repo_1', name: 'example/one', stage: 'ready' },
    { id: 'repo_2', name: 'example/two', stage: 'failed' },
    { id: 'repo_3', name: 'example/three', stage: 'queued' },
  ];
  const kept = ['repo_1', 'repo_3'];
  const partial = { ...analysis, projects, completed: 1, total: 3, failure: 'activity_failed', retry: { ...retry, projects: kept } };
  for (const failure of ['activity_failed', 'provider_unavailable', 'feature_unavailable']) {
    const result = api.readActivityAnalysis({ ...partial, failure });
    expect(result).toEqual({ ...partial, failure });
    expect(Object.isFrozen(result?.retry?.projects)).toBe(true);
    expect(api.activityAnalysisRetry(failure, projects)).toEqual({ ...retry, projects: kept });
  }
  // Session, consent, grant, quota and option failures need the viewer's action, not a new acquisition.
  for (const failure of ['auth_required', 'entitlement_required', 'consent_required', 'quota_exceeded', 'busy', 'invalid_option', 'not_found']) {
    expect(api.readActivityAnalysis({ ...partial, failure })).toBeNull();
    expect(api.readActivityAnalysis({ ...partial, failure, retry: undefined })).toBeNull();
    expect(api.activityAnalysisRetry(failure, projects)).toBeUndefined();
  }
  for (const ids of [['repo_1'], ['repo_1', 'repo_2', 'repo_3'], ['repo_3', 'repo_1'], ['repo_1', 'repo_3', 'repo_3'], ['repo_1', 'repo_9']])
    expect(api.readActivityAnalysis({ ...partial, retry: { ...retry, projects: ids } })).toBeNull();
  // Continuing needs an actually ready project and an explicit exclusion.
  const noReady = projects.map(p => p.stage === 'ready' ? { ...p, stage: 'queued' as const } : p);
  expect(api.readActivityAnalysis({ ...partial, projects: noReady, completed: 0 })).toBeNull();
  expect(api.activityAnalysisRetry('activity_failed', noReady)).toBeUndefined();
  const noFailure = projects.map(p => p.stage === 'failed' ? { ...p, stage: 'ready' as const } : p);
  expect(api.readActivityAnalysis({ ...partial, projects: noFailure, completed: 2, retry: { ...retry, projects: ['repo_1', 'repo_2', 'repo_3'] } })).toBeNull();
  expect(api.activityAnalysisRetry('activity_failed', noFailure)).toBeUndefined();
  expect(api.activityAnalysisRetry('coverage_incomplete', projects)).toEqual({ ...retry, projects: ['repo_1', 'repo_2', 'repo_3'] });
  expect(api.activityAnalysisRetry(undefined, projects)).toBeUndefined();
});

test('analysis rejects missing required fields, malformed envelopes, and throwing input', () => {
  expect(api.readActivityAnalysis).toBeDefined();
  for (const key of Object.keys(analysis)) {
    const missing: Record<string, unknown> = { ...analysis };
    delete missing[key];
    expect(api.readActivityAnalysis(missing)).toBeNull();
  }
  const throwing = new Proxy({}, { get() { throw new Error('PRIVATE'); }, has() { throw new Error('PRIVATE'); } });
  for (const invalid of [null, undefined, [], 'PRIVATE', 42, throwing]) {
    expect(api.readActivityAnalysis(invalid)).toBeNull();
    expect(api.readActivityResume(invalid)).toBeNull();
    expect(protocol.readActivityJob(invalid)).toBeNull();
  }
});

test('job analysis is safely additive while old jobs and guarded log routes remain unchanged', () => {
  expect(protocol.readActivityJob(job)).toEqual(job);
  const input = { ...job, analysis: { ...analysis, token: 'SEEDED_SECRET' }, account_id: 77 };
  expect(protocol.readActivityJob(input)).toEqual({ ...job, analysis });
  expect(Object.isFrozen(protocol.readActivityJob(input)?.analysis)).toBe(true);
  for (const status of ['queued', 'running', 'ready', 'failed', 'cancelled']) {
    const old = { ...job, status, ...(status === 'ready' ? { log_url: '/api/activity/jobs/job_1/log' } : {}) };
    expect(protocol.readActivityJob(old)).toEqual(old);
  }
  for (const invalid of [null, undefined, {}, { ...analysis, completed: 1 }])
    expect(protocol.readActivityJob({ ...job, analysis: invalid })).toBeNull();
  expect(protocol.readActivityJob({ ...job, status: 'ready', analysis })).toBeNull();
  expect(protocol.readActivityJob({ ...job, analysis, log_url: 'https://storage.invalid/log' })).toBeNull();
});

test('resume returns no work or an owned running ready or failed analysis and strips request authority', () => {
  expect(api.readActivityResume).toBeDefined();
  expect(api.readActivityResume({ analysis: null, account_id: 77, token: 'SEEDED_SECRET' })).toEqual({ analysis: null });
  expect(Object.isFrozen(api.readActivityResume({ analysis: null }))).toBe(true);
  for (const status of ['running', 'ready', 'failed']) {
    const active = { ...job, status, analysis, ...(status === 'ready' ? { log_url: '/api/activity/jobs/job_1/log' } : {}) };
    const result = api.readActivityResume({ analysis: { ...active, account_id: 77 }, account_id: 77 });
    expect(result).toEqual({ analysis: active });
    expect(Object.isFrozen(result)).toBe(true);
    expect(Object.isFrozen(result?.analysis)).toBe(true);
    expect(Object.isFrozen(result?.analysis?.analysis)).toBe(true);
  }
  for (const invalid of [{}, { analysis: undefined }, { analysis: job },
    { analysis: { ...job, analysis, status: 'queued' } }, { analysis: { ...job, analysis, status: 'cancelled' } },
    { analysis: { ...job, analysis: null } }, { analysis: { ...job, analysis, status: 'ready' } },
    { analysis: { ...job, analysis, status: 'ready', log_url: '/api/activity/jobs/other/log' } }])
    expect(api.readActivityResume(invalid)).toBeNull();
});
