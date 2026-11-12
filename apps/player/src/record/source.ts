// SPDX-License-Identifier: GPL-3.0-or-later

// The recording's source identity (recipe `source`, REC-70, REC-103). The
// page gives the first one; a host's session keeps it up to date
// (`applySession`): F2's `repo.visibility` marks a private
// repository (US-11), and the service's canonical name replaces the one typed
// in the URL. Unknown-provenance sources (a local file, a log URL) are
// treated as private for titles, names and the recipe.

import type { PlaybackState } from '../playback';
import type { RecordSource } from './types';

/** A log from a file or a URL: no repository identity. */
export function logSource(kind: 'file' | 'url', path: string): RecordSource {
  return { kind, host: '', path, branch: '', since: '', until: '', subdir: '', head: '', private: false };
}

/**
 * Follows the run's session (F2 PR-93): a private repository records with
 * no default title, the stem `gource`, no embedded recipe and a redacted
 * sidecar (REC-11, REC-62, REC-71, REC-103). Privacy only ever turns on:
 * a run that was private stays private.
 */
export function applySession(source: RecordSource, state: PlaybackState): void {
  if (source.kind !== 'repo' || state.source !== 'server') return;
  if (state.private) source.private = true;
  // The analyze answer's display name is the canonical `owner/repo`.
  if (state.job && state.display.includes('/')) source.path = state.display;
}
