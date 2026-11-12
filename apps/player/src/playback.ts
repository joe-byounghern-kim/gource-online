// SPDX-License-Identifier: GPL-3.0-or-later

// The loading state of one log, as the viewer screen and the recording read
// it.

import type { DoneEvent, ProgressEvent, ServerStage } from '@gource-online/protocol';

export interface PlaybackError {
  code: string;
  message: string;
}

export interface PlaybackState {
  display: string;
  host: string;
  /** `local`: a log file or URL loaded without the clone service. */
  source: 'server' | 'local';
  /** The server's job id, once `analyze` answered. */
  job?: string;
  /** The repository is private (`repo.visibility`, F2 PR-93). */
  private: boolean;
  /** Latest progress per server stage. */
  progress: Partial<Record<ServerStage, ProgressEvent>>;
  /** `totalBytes` is known for local files; `format` once the log is read. */
  loading: {
    started: boolean;
    bytes: number;
    commits: number;
    finished: boolean;
    totalBytes?: number;
    format?: 'custom' | 'git';
  };
  done?: DoneEvent;
  error?: PlaybackError;
  ready: boolean;
  /** F1 AI-29: the server's `ai_detect` (absent for local logs). */
  aiDetect?: boolean;
}

/**
 * Share of the log loaded into the engine, or null when unknown. Bytes are
 * exact once the size is known (local files; the server's `done`, at once
 * on a cache hit); while git is still writing the log, the server's history
 * progress stands in. Commit counts cannot: `commits_total` counts every git
 * commit (merges included) and the custom log groups lines by time and user.
 */
export function loadingFraction(state: PlaybackState): number | null {
  const l = state.loading;
  if (l.finished) return 1;
  const total = state.done?.bytes ?? l.totalBytes;
  if (total) return Math.min(1, l.bytes / total);
  const h = state.progress.history;
  if (h?.current !== undefined && h.total) return Math.min(1, h.current / h.total);
  return null;
}

/** The label of a run's host: `local` for a log file, `log` for a log the player was handed. */
export function hostLabel(host: string): string {
  return host === 'gitlab.com' ? 'GitLab' : host === 'local' ? 'Local log' : host === 'log' ? 'Log' : 'GitHub';
}
