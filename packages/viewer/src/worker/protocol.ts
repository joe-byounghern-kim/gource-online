// SPDX-License-Identifier: GPL-3.0-or-later

// Messages between the page and parser.worker.ts.

/** Where the log (a custom log, or git log output) comes from. */
export type LogSource =
  /**
   * Absolute URL (`/api/jobs/:id/log`, a `?log=` deep link), or `mock:` for a fabricated log.
   * `fetch` narrows the request: the player passes the handoff protocol's `PLAYER_LOG_FETCH`.
   */
  | { kind: 'url'; url: string; fetch?: LogFetch }
  /** A file the user opened or dropped on the page. */
  | { kind: 'file'; file: Blob; name: string };

/** The request settings a page may set for a log URL (structured-cloneable, unlike `RequestInit`). */
export interface LogFetch {
  credentials?: RequestCredentials;
  referrerPolicy?: ReferrerPolicy;
  redirect?: RequestRedirect;
}

export type ToWorker =
  | {
      type: 'start';
      source: LogSource;
      /** Mock mode: no wasm; batches are whole custom-log lines. */
      mock: boolean;
    }
  | { type: 'cancel' };

export type FromWorker =
  /** An encoded batch for GourceApp.append_batch; `batch` is transferred. */
  | { type: 'batch'; batch: ArrayBuffer; commits: number; bytes: number }
  /** Bytes arrived without completing a batch. */
  | { type: 'progress'; commits: number; bytes: number }
  | {
      type: 'done';
      commits: number;
      bytes: number;
      skipped: number;
      /** `git`: the output of Gource's git log command; `custom`: a custom log. */
      format: 'custom' | 'git';
    }
  | { type: 'error'; code: 'engine_missing' | 'log_failed' | 'parse_failed'; message: string }
  | { type: 'cancelled' };
