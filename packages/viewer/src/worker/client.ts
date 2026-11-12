// SPDX-License-Identifier: GPL-3.0-or-later

// Page side of parser.worker.ts.

import type { FromWorker, LogSource, ToWorker } from './protocol';

export type ParserEvent = FromWorker;

export class ParserClient {
  private worker: Worker | null;

  constructor(onMessage: (event: ParserEvent) => void, onFailure: (message: string) => void) {
    this.worker = new Worker(new URL('./parser.worker.ts', import.meta.url), { type: 'module', name: 'gource-parser' });
    this.worker.addEventListener('message', (e: MessageEvent<FromWorker>) => onMessage(e.data));
    this.worker.addEventListener('error', (e) => {
      e.preventDefault();
      onFailure(e.message || 'The log parser worker failed to start.');
    });
  }

  start(source: LogSource, mock: boolean): void {
    this.send({ type: 'start', source, mock });
  }

  cancel(): void {
    this.send({ type: 'cancel' });
  }

  terminate(): void {
    this.worker?.terminate();
    this.worker = null;
  }

  private send(message: ToWorker): void {
    this.worker?.postMessage(message);
  }
}
