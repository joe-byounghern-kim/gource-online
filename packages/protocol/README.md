# @gource-online/protocol

The shared contract of [Gource Online](https://github.com/joe-byounghern-kim/gource-online),
MIT-licensed so any client or service can use it:

- `parseRepoUrl`, `validateBranch`, `validateDate`, `validateSubdir`: which repository URLs and
  analysis options a clone service accepts, with the test vectors in `vectors/repo-urls.json`;
- the clone service API types (`/api/analyze`, `/api/jobs/:id`, `/api/features`);
- additive personal-discovery/consent/analysis and rich PR mode API types, plus a whitelisted
  final-log/index descriptor reader (no identity evidence, account IDs or private index rows);
- revisioned analysis display state (`ActivityProjectStage`, `ActivityProjectProgress`,
  `ActivityAnalysisState`, `readActivityAnalysis`), optional on `ActivityJobResponse`, and
  authenticated current-account resume (`ActivityResumeResponse`, `readActivityResume`);
- feature ids and the feature registry parser;
- frozen snapshot responses with an optional canonical `share_url`, and bounded owner-only
  `GET /api/snapshots?limit=10&cursor=...` pages. `readActivitySnapshotList` copies only creation,
  expiry, state, project count, period and byte size. A null period means older listing metadata
  is unavailable. Pages contain at most `SNAPSHOT_PAGE_SIZE` entries and an opaque `next_cursor`
  or null. Cursors are positions, not authorization. Published share links use `snapshotShareUrl`
  and always enter the guarded playback route. They never contain storage credentials and do
  not bypass source visibility revalidation;
- repository boundaries in a multi-repository activity log: one top-level directory
  `/<owner>:<repo>/` and one actor `<owner>/<repo>` per repository
  (`activityRepositoryDirectory`, `activityRepositoryActor`), and `activityRepositoryOf` /
  `activityRepositoryFilter`, which also accept the legacy `/<owner>/<repo>/` layout;
- Gource's git log command (`LOG_ARGS`, `PRETTY_AI`) that `gource-rs convert --from git` reads.

No runtime dependencies. The rest of Gource Online is GPL-3.0-or-later; see `LICENSING.md`
in the repository.

Nothing here is Gource's code. `LOG_ARGS` states the arguments of the git command Gource runs
(`formats/git.cpp`), so every producer of a log uses the same one.

## Analysis progress and resume

`readActivityJob` remains compatible with responses without `analysis`. If present, that
state must validate: 1–50 unique projects (one for rich PR analysis), exact opaque IDs,
safe canonical `owner/repo` names, closed stages/codes, ready-only completion counts,
revision 0–1024, real UTC dates strictly under ten calendar years, and the original strict
UTC `expires_at`. ETA is explicitly unknown (`estimate_seconds: null`). Readers copy and
freeze only public allowlisted fields, including nested records and arrays.

`GET /api/activity/resume` authenticates the current account without accepting request
account/job authority. `readActivityResume` accepts `{ analysis: null }` or a validated
running/ready/failed job with analysis state, never a queued/cancelled job. Existing job
SSE event names carry the same DTO with the revisioned latest state. Reconnect replays
that state, not invented stage history. Deduplicate by job ID and revision. Reads, resume,
and SSE replay never extend the original creation-based TTL or revive expired work.

`activityAnalysisRetry` defines the only valid retry receipt: `kind: 'new-analysis'`,
`requires_confirmation: true`, `reacquires_sources: true`, `additional_analysis_jobs: 1` and the
exact `projects` to keep. `coverage_incomplete` keeps every project. A partial source/provider
failure with a ready and a failed project must carry it, excluding only the failed projects
(**Continue with available projects**). Session, consent, grant and quota failures never carry it.
It means an explicitly confirmed new acquisition with a new job/idempotency key and one
additional analysis job, never an automatic retry or revival of the failed job. Current grants,
quotas, source access, and fresh consent/discovery as needed still apply. Neither resume nor
warm caches supply authority.
