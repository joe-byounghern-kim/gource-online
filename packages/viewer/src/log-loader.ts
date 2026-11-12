// SPDX-License-Identifier: GPL-3.0-or-later

// Streams a log through the parser worker into an engine: the first batch
// makes the run playable, `done` makes it seekable.

import type { GourceApp } from './engine/types';
import { ParserClient } from './worker/client';
import type { LogSource } from './worker/protocol';

export interface LogProgress {
  bytes: number;
  commits: number;
}

export interface LogDone extends LogProgress {
  skipped: number;
  /** `git`: the output of Gource's git log command; `custom`: a custom log. */
  format: 'custom' | 'git';
}

export interface LogError {
  code: string;
  message: string;
}

export interface LogLoaderEvents {
  /** Bytes or commits arrived. `playable` is true from the first batch with a commit in the engine. */
  progress(progress: LogProgress, playable: boolean): void;
  /** The whole log is in the engine (it has at least one commit) and the run is seekable. */
  done(done: LogDone): void;
  /** No commit could be read: the engine got nothing to finish. */
  empty(done: LogDone): void;
  error(error: LogError): void;
}

export class LogLoader {
  private parser: ParserClient | null = null;
  private stopped = false;
  private playable = false;

  constructor(private readonly engine: GourceApp, private readonly mock: boolean, private readonly events: LogLoaderEvents) {}

  start(source: LogSource): void {
    this.engine.begin_log();
    this.parser = new ParserClient(
      (e) => {
        if (this.stopped) return;
        switch (e.type) {
          case 'batch':
            try {
              this.engine.append_batch(new Uint8Array(e.batch));
            } catch (err) {
              this.fail({ code: 'parse_failed', message: err instanceof Error ? err.message : String(err) });
              return;
            }
            if (e.commits > 0) this.playable = true;
            this.events.progress({ bytes: e.bytes, commits: e.commits }, this.playable);
            return;
          case 'progress':
            this.events.progress({ bytes: e.bytes, commits: e.commits }, this.playable);
            return;
          case 'done': {
            this.close();
            const done = { bytes: e.bytes, commits: e.commits, skipped: e.skipped, format: e.format };
            if (e.commits === 0) {
              this.events.empty(done);
              return;
            }
            this.engine.finish_log(e.bytes);
            this.events.done(done);
            return;
          }
          case 'error':
            this.fail({ code: e.code, message: e.message });
            return;
          case 'cancelled':
            return;
        }
      },
      (message) => this.fail({ code: 'parse_failed', message }),
    );
    this.parser.start(source, this.mock);
  }

  /** Stops the download and the worker; the engine keeps what arrived. */
  cancel(): void {
    this.stopped = true;
    this.parser?.cancel();
    this.close();
  }

  private fail(error: LogError): void {
    this.cancel();
    this.events.error(error);
  }

  private close(): void {
    this.parser?.terminate();
    this.parser = null;
  }
}
