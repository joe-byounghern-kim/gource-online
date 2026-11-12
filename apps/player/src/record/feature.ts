// SPDX-License-Identifier: GPL-3.0-or-later

// The Record entry point in the viewer (REC-1 to REC-4, REC-20, REC-86):
// the toolbar button with its disabled reasons, Alt+R, `record=1`, the
// "recording in progress" banner and the handshake with the viewer that
// pauses the live simulation while a recording runs. The dialog itself is
// loaded and built on first use.

import { h } from '../dom';
import type { EngineStatus, GourceApp } from '@gource-online/viewer';
import { playerHost } from '../host';
import type { PlaybackState } from '../playback';
import { recordingSupported } from './encoder';
import { hooksActive, mockRecordEnabled } from './hooks';
import { takeInitialRecordParams, type RecordUrlParams } from './options';
import { applySession } from './source';
import type { RecordSource, SnapshotRecordSettings } from './types';
import type { RecordDialog } from './dialog';
import './record.css';

/** What the dialog needs from the viewer. */
export interface RecordHost {
  engine(): GourceApp | null;
  /** Live option values (Gource names), as the panel and the URL hold them. */
  values(): ReadonlyMap<string, string>;
  status(): EngineStatus | null;
  /** Pauses the live simulation and locks restart-class controls (REC-20), or restores them. */
  setRecording(active: boolean): void;
  /** Where the dialog and the banner are attached. */
  root: HTMLElement;
  display: string;
  source: RecordSource;
  /** The page runs the mock engine (`?mock=1`). */
  mock: boolean;
}

export function recordIcon(): SVGSVGElement {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('class', 'icon icon-record');
  svg.setAttribute('aria-hidden', 'true');
  const ring = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
  ring.setAttribute('cx', '12');
  ring.setAttribute('cy', '12');
  ring.setAttribute('r', '8');
  const dot = document.createElementNS('http://www.w3.org/2000/svg', 'circle');
  dot.setAttribute('cx', '12');
  dot.setAttribute('cy', '12');
  dot.setAttribute('r', '3.5');
  dot.setAttribute('fill', 'currentColor');
  dot.setAttribute('stroke', 'none');
  svg.append(ring, dot);
  return svg;
}

export class RecordFeature {
  readonly button: HTMLButtonElement;
  readonly banner: HTMLElement;
  private readonly host: RecordHost;
  private dialog: RecordDialog | null = null;
  private dialogLoading: Promise<RecordDialog> | null = null;
  private logComplete = false;
  private loadPercent: number | null = 0;
  private head = '';
  private urlParams: RecordUrlParams | null;
  private readonly onKey = (e: KeyboardEvent) => this.keyDown(e);
  private readonly coarse: MediaQueryList | null;
  private disposed = false;
  /** Unregisters the F2 hook that stops and saves before a private run ends. */
  private readonly unhookPrivate: () => void;

  constructor(host: RecordHost, params?: RecordUrlParams) {
    this.host = host;
    // Consume initial URL settings even when an immutable snapshot overrides
    // them, so they cannot leak into a later viewer in the same page.
    const initial = takeInitialRecordParams();
    this.urlParams = params ?? initial;
    this.button = h('button.icon-btn.record-btn', { type: 'button', 'aria-label': 'Record video', 'aria-keyshortcuts': 'Alt+R', onclick: () => void this.open() }, recordIcon());
    this.banner = h(
      'div.record-banner',
      { role: 'status', hidden: true },
      recordIcon(),
      h('span', {}, 'Recording in progress: the live view is paused; changes here do not affect the video.'),
      h('button.btn.btn-small', { type: 'button', onclick: () => void this.open() }, 'Show progress'),
    );
    window.addEventListener('keydown', this.onKey);
    this.coarse = typeof window.matchMedia === 'function' ? window.matchMedia('(pointer: coarse)') : null;
    this.coarse?.addEventListener('change', () => this.refresh());
    window.addEventListener('resize', this.onResize);
    // Sign-out or a lost session ends a private run (F2 PR-121): a recording
    // in progress is stopped and saved first, then the run is closed (REC-20).
    this.unhookPrivate = playerHost.onRunEnding(() => this.stopAndSave());
    this.refresh();
  }

  /** Stops a running recording and keeps the frames so far; resolves once the file is finished. */
  async stopAndSave(): Promise<void> {
    if (this.dialog?.running) await this.dialog.stopAndSaveNow();
  }

  private readonly onResize = () => this.refresh();

  /** Why Record cannot be used right now, or null (REC-4). */
  disabledReason(): string | null {
    if (!recordingSupported()) return 'This browser cannot encode video here. Chrome and Edge, opened over https or localhost, are tested.';
    if (this.host.mock && !mockRecordEnabled()) return 'Recording needs the real engine (this page runs the mock engine).';
    if (!this.logComplete) {
      return this.loadPercent === null ? 'Waiting for the full history' : `Waiting for the full history, ${Math.floor(this.loadPercent)} %`;
    }
    return null;
  }

  /** Touch-first devices do not get Record (REC-86). */
  private touchFirst(): boolean {
    return (this.coarse?.matches ?? false) && window.innerWidth < 768;
  }

  private refresh(): void {
    if (this.disposed) return;
    const reason = this.disabledReason();
    this.button.hidden = this.touchFirst();
    // aria-disabled keeps the button focusable so the reason stays reachable.
    this.button.setAttribute('aria-disabled', String(reason !== null));
    this.button.classList.toggle('is-disabled', reason !== null);
    this.button.title = reason ?? 'Record video (Alt+R)';
    this.button.dataset.reason = reason ?? '';
    if (reason === null && this.urlParams?.open) {
      const params = this.urlParams;
      this.urlParams = null;
      void this.open(params);
    }
  }

  /** The viewer's loading updates: Record unlocks once the whole log is in (US-9). */
  updateLoading(state: PlaybackState): void {
    applySession(this.host.source, state);
    const l = state.loading;
    const complete = l.finished && !state.error;
    const total = state.done?.bytes ?? l.totalBytes;
    this.loadPercent = l.finished ? 100 : total ? Math.min(100, (l.bytes / total) * 100) : null;
    if (state.done?.head) this.head = state.done.head;
    if (complete !== this.logComplete || !complete) {
      this.logComplete = complete;
      this.refresh();
    }
  }

  get headSha(): string {
    return this.head;
  }

  get running(): boolean {
    return this.dialog?.running ?? false;
  }

  /** Capture the effective F3 choices without opening a dialog or starting an encoder. */
  async captureSnapshotSettings(): Promise<SnapshotRecordSettings> {
    if (this.disposed || !this.logComplete || this.running) throw new Error('Snapshot recording settings are unavailable.');
    const dialog = await this.loadDialog();
    const settings = await dialog.captureSnapshotSettings(this.urlParams ?? undefined);
    if (this.disposed) throw new DOMException('Cancelled', 'AbortError');
    return settings;
  }

  private keyDown(e: KeyboardEvent): void {
    if (!e.altKey || e.ctrlKey || e.metaKey || e.code !== 'KeyR') return;
    if (this.host.root.hidden || this.button.hidden) return;
    e.preventDefault();
    void this.open();
  }

  /** Opens the dialog (or brings a running recording's progress forward, REC-89). */
  async open(params?: RecordUrlParams): Promise<void> {
    if (this.disposed) return;
    if (this.dialog?.running) {
      this.dialog.focusProgress();
      return;
    }
    if (this.disabledReason() !== null) {
      this.button.focus();
      return;
    }
    const dialog = await this.loadDialog();
    if (this.disposed) return;
    await dialog.open(params ?? (this.urlParams && !this.urlParams.open ? this.takeOptionParams() : undefined));
  }

  /** `record-*` URL values without `record=1` still prefill the first dialog (REC-3). */
  private takeOptionParams(): RecordUrlParams | undefined {
    const p = this.urlParams;
    this.urlParams = null;
    return p ?? undefined;
  }

  private loadDialog(): Promise<RecordDialog> {
    if (this.dialog) return Promise.resolve(this.dialog);
    this.dialogLoading ??= import('./dialog').then(({ RecordDialog: Dialog }) => {
      const d = new Dialog(this.host, {
        head: () => this.head,
        returnFocus: () => this.button.focus({ preventScroll: true }),
        runningChanged: (running) => {
          this.banner.hidden = !running;
          this.host.setRecording(running);
        },
      });
      this.host.root.append(d.el);
      this.dialog = d;
      return d;
    });
    return this.dialogLoading;
  }

  /** Test hook builds: whether the hooks are live on this page. */
  static testMode(): boolean {
    return hooksActive();
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.unhookPrivate();
    window.removeEventListener('keydown', this.onKey);
    window.removeEventListener('resize', this.onResize);
    this.dialog?.dispose();
    this.dialog = null;
  }
}
