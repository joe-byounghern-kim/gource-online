// SPDX-License-Identifier: GPL-3.0-or-later

// Viewer screen: full-viewport canvas driven by the engine, playback bar,
// status line, loading indicator, control panel and debug overlay.

import '../styles/theme.css';
import '../styles/viewer.css';
import { append, h, icon } from '../dom';
import { parseStatus, type EngineStatus, type GourceApp, type GourceViewerElement } from '@gource-online/viewer';
import { formatBytes, formatCount } from '../format';
import { OPTION_DEFS, OPTIONS_BY_NAME, RESTART_OPTIONS, normaliseValue } from '@gource-online/viewer/options';
import { prRecordingLocked } from '../pr-mode';
import { hostLabel, loadingFraction, type PlaybackState } from '../playback';
import { ControlPanel } from '../ui/panel';
import { AiSection } from '../ai/section';
import { aiStartOptions, loadStoredAi, prefersReducedMotion } from '../ai/stored';
// F3 recording (./record/).
import { RecordFeature } from '../record/feature';
import type { RecordUrlParams } from '../record/options';
import type { RecordSource } from '../record/types';

const SPEED_PRESETS = [0.05, 0.1, 0.25, 0.5, 1, 2, 5, 10, 30];
const STATUS_INTERVAL_MS = 250;
const SEEK_STEPS = 10000;
const RECORDING_LOCKED = 'Recording in progress: the live view is paused; this changes nothing in the video.';

/** Options Gource's keys change inside the engine (`Gource::keyPress`); read back after a key press. */
const KEY_OPTIONS = [
  'seconds-per-day',
  'time-scale',
  'camera-mode',
  'key',
  'hide',
  'highlight-users',
  'highlight-dirs',
  'file-extensions',
  'hash-seed',
];

/** `0.33333334` → `0.333`: speed labels stay short whatever the engine computed. */
function speedLabel(n: number): string {
  return `${Number(n.toPrecision(3))} s/day`;
}

export interface ViewerOptions {
  display: string;
  host: string;
  /** Option values from the page URL (Gource names). */
  options: Record<string, string>;
  /** Frozen recording settings use F3, never the live engine option API. */
  recordParams?: RecordUrlParams;
  /** Whether "Copy link" is offered (unset: yes). */
  shareable?: boolean;
  /** The recording's first source identity (gource-player/record/source). */
  recordSource: RecordSource;
  /** The placeholder engine runs (`?mock=1`): the recording uses its mock encoder. */
  mock: boolean;
  /** Shown at the end of the settings panel: the licence and the source, for a person who only ever sees the viewer. */
  about?: HTMLElement;
  /** The length of the link to this view with these option values; absent where there is no such link. */
  linkLength?(values: ReadonlyMap<string, string>): number | null;
  /** Called when the set of option values changed (for the page URL). */
  onOptionsChanged(values: Map<string, string>): void;
  onBack(): void;
  /** The back button's label. */
  backLabel?: string;
  /** Starts the run again (after the history failed to load completely). */
  onRetry(): void;
}

export class ViewerScreen {
  readonly el: HTMLElement;
  /** The canvas, the engine on it, its input and the render loop. */
  readonly view: GourceViewerElement;
  readonly canvas: HTMLCanvasElement;
  private engine: GourceApp | null = null;
  private readonly opts: ViewerOptions;
  private readonly values = new Map<string, string>();
  private readonly panel: ControlPanel;
  /** F1: panel section "Contributors & AI" and its URL notices. */
  private readonly ai: AiSection;
  private readonly panelToggle: HTMLButtonElement;
  private readonly playBtn: HTMLButtonElement;
  private readonly seek: HTMLInputElement;
  private readonly seekTip: HTMLElement;
  private readonly seekWrap: HTMLElement;
  private readonly speed: HTMLSelectElement;
  private readonly statusLine: HTMLElement;
  private readonly loadingBox: HTMLElement;
  private readonly loadingFill: HTMLElement;
  private readonly loadingText: HTMLElement;
  private readonly incompleteNote: HTMLElement;
  private readonly debugBox: HTMLElement;
  private readonly debugBtn: HTMLButtonElement;
  private readonly banner: HTMLElement;
  private readonly repoName: HTMLElement;
  private readonly tools: HTMLElement;
  private readonly shareBtn: HTMLButtonElement;
  private readonly cleanups: Array<() => void> = [];
  private statusTimer: number | undefined;
  private seeking = false;
  private lastStatus: EngineStatus | null = null;
  private disposed = false;
  /** The history stopped loading part-way (the session failed after the viewer appeared). */
  private incomplete = false;
  // F3 recording: the Record button, and the live view's pause while a recording runs (REC-20).
  private readonly record: RecordFeature;

  private recording = false;
  private pausedBeforeRecording = false;

  constructor(opts: ViewerOptions) {
    this.opts = opts;
    // F1: stored AI options fill in what the URL leaves out (the URL wins).
    const aiStart = aiStartOptions(opts.options, loadStoredAi(), prefersReducedMotion());
    for (const [k, v] of Object.entries(aiStart.options)) this.values.set(k, v);
    this.ai = new AiSection({
      setOption: (name, value) => this.setOption(name, value),
      ...(opts.linkLength ? { linkLength: opts.linkLength } : {}),
    }, this.values, aiStart);

    this.view = document.createElement('gource-viewer');
    this.canvas = this.view.canvas;
    this.canvas.setAttribute('aria-label', `Gource visualisation of ${opts.display}`);
    this.canvas.setAttribute('aria-describedby', 'canvas-keys');
    // Shown while the canvas has keyboard focus: Tab belongs to Gource there.
    const keysHint = h(
      'p.canvas-keys#canvas-keys',
      {},
      h('kbd', {}, 'Tab'),
      ' next user · ',
      h('kbd', {}, 'Esc'),
      ' or ',
      h('kbd', {}, 'Shift+Tab'),
      ' leave the visualisation',
    );

    this.panel = new ControlPanel(
      {
        setOption: (name, value) => this.setOption(name, value),
        applyFilters: (values) => this.applyFilters(values),
        section: (id) => (id === 'style' ? this.ai.el : null),
      },
      this.values,
      () => this.togglePanel(false),
    );

    this.playBtn = h('button.icon-btn.play-btn', { type: 'button', 'aria-label': 'Pause', onclick: () => this.togglePause() }, icon('pause'));
    this.seek = h('input.seek', {
      type: 'range',
      min: 0,
      max: SEEK_STEPS,
      step: 1,
      value: 0,
      disabled: true,
      'aria-label': 'Position in history',
      title: 'Seeking unlocks once the whole history has loaded',
    });
    this.seekTip = h('span.seek-tip', { hidden: true, role: 'tooltip' });
    this.speed = h(
      'select.speed',
      { 'aria-label': 'Seconds per day', title: 'Seconds per day (seconds-per-day)' },
      ...SPEED_PRESETS.map((v) => h('option', { value: String(v) }, speedLabel(v))),
    );
    this.statusLine = h('div.status-line', { 'aria-live': 'off' });
    this.loadingFill = h('span.mini-progress-fill');
    this.loadingText = h('span.mini-progress-text');
    this.loadingBox = h(
      'div.mini-progress',
      { role: 'status' },
      h('span.mini-progress-bar', {}, this.loadingFill),
      this.loadingText,
    );
    this.incompleteNote = h('span.history-incomplete', { hidden: true });
    this.debugBox = h('pre.debug-overlay', { hidden: true, 'aria-label': 'Performance' });
    this.debugBtn = h('button.icon-btn', { type: 'button', 'aria-label': 'Toggle performance overlay', 'aria-pressed': 'false', onclick: () => this.toggleDebug() }, icon('gauge'));
    this.panelToggle = h(
      'button.icon-btn',
      { type: 'button', 'aria-label': 'Settings', 'aria-controls': 'control-panel', 'aria-expanded': 'false', onclick: () => this.togglePanel() },
      icon('sliders'),
    );
    this.banner = h('div.viewer-banner', { role: 'alert', hidden: true });

    this.seekWrap = h('div.seek-wrap', {}, this.seek, this.seekTip);
    const viewer = this;
    this.record = new RecordFeature({
      engine: () => this.engine,
      values: () => this.values,
      status: () => this.lastStatus,
      setRecording: (active) => this.setRecording(active),
      get root() {
        return viewer.el;
      },
      display: opts.display,
      source: opts.recordSource,
      mock: opts.mock,
    }, opts.recordParams);
    const shareBtn = h('button.icon-btn', { type: 'button', 'aria-label': 'Copy link to this view', title: 'Copy link', onclick: () => this.copyLink(shareBtn) }, icon('share'));
    this.shareBtn = shareBtn;
    shareBtn.hidden = opts.shareable === false;
    this.repoName = h('span.viewer-name', {}, opts.display);
    this.tools = h('div.viewer-tools', {}, shareBtn, this.record.button, this.debugBtn, this.panelToggle);

    this.el = h(
      'main.screen.screen-viewer',
      { 'data-screen': 'viewer' },
      h(
        'div.viewer-top',
        {},
        h(
          'div.viewer-title',
          {},
          h('button.icon-btn', { type: 'button', 'aria-label': opts.backLabel ?? 'Back to repository selection', onclick: () => opts.onBack() }, icon('back')),
          h('div.viewer-repo', {}, h('span.viewer-host', {}, hostLabel(opts.host)), this.repoName),
          this.loadingBox,
          this.incompleteNote,
        ),
        this.tools,
      ),
      // After the top bar in the tab order, so Shift+Tab from the canvas
      // reaches the toolbar and Esc moves on to the playback bar.
      h('div.stage-area', {}, this.view, keysHint),
      this.debugBox,
      this.banner,
      this.ai.notices,
      this.record.banner,
      h(
        'div.playback',
        {},
        h('div.playback-row', {}, this.playBtn, this.seekWrap, this.speed),
        this.statusLine,
      ),
      this.panel.el,
    );

    if (opts.about) this.panel.el.append(opts.about);

    this.speed.addEventListener('change', () => {
      const v = this.speed.value;
      if (this.setOption('seconds-per-day', v) === null) this.panel.setValue('seconds-per-day', v);
    });
    this.syncSpeed();
    this.bindSeek();
    this.bindInput();
    this.view.keyScope = this.el;
  }

  // --- engine ---

  /** Capture the current F3 settings without changing live playback. */
  captureSnapshotSettings() { return this.record.captureSnapshotSettings(); }

  /** Creates the engine on the canvas and applies the URL options. */
  attach(engine: GourceApp): void {
    this.engine = engine;
    this.view.attach(engine);
    for (const def of OPTION_DEFS) {
      const value = this.values.get(def.name);
      if (value === undefined) continue;
      try {
        engine.set_option(def.name, value);
      } catch (err) {
        console.warn(`Ignoring option ${def.name}=${value}:`, err);
        this.values.delete(def.name);
        this.panel.setValue(def.name, def.default);
        this.ai.setValue(def.name, def.default);
      }
    }
    this.ai.attach(engine);
    this.applyHide();
    this.notifyOptions();
  }

  get attached(): GourceApp | null {
    return this.engine;
  }

  private setOption(name: string, value: string): string | null {
    const engine = this.engine;
    if (!engine) return 'The engine is not running yet.';
    if (this.recording && (RESTART_OPTIONS.has(name)
      || prRecordingLocked(name, value, engine.get_option('hide') ?? ''))) return RECORDING_LOCKED;
    let effect: string;
    try {
      effect = engine.set_option(name, value);
    } catch (err) {
      return err instanceof Error ? err.message : String(err);
    }
    this.values.set(name, value);
    if (effect === 'restart' && OPTIONS_BY_NAME.get(name)?.section !== 'filters') engine.restart();
    if (this.recording) engine.set_paused(true);
    if (name === 'seconds-per-day') this.syncSpeed();
    if (name === 'hide' || name === 'hide-pull-requests') {
      const hide = engine.get_option('hide') ?? '';
      const hidden = engine.get_option('hide-pull-requests') ?? 'false';
      this.values.set('hide', hide);
      this.values.set('hide-pull-requests', hidden);
      this.applyHide();
      this.panel.setValue('hide', hide);
      this.panel.setValue('hide-pull-requests', hidden);
    }
    this.ai.optionApplied(name, value);
    this.notifyOptions();
    return null;
  }

  /**
   * Gource's keys (+/-, ./, v, k, u, d, f, t, g, r, m, s) change settings in
   * the engine; mirror them into the panel, the speed menu and the page URL
   * (from every status poll, and right after a handled key).
   */
  private syncFromEngine(): void {
    const engine = this.engine;
    if (!engine) return;
    let changed = false;
    for (const name of KEY_OPTIONS) {
      let value: string | undefined;
      try {
        value = engine.get_option(name);
      } catch {
        continue;
      }
      if (value === undefined) continue;
      const current = this.values.get(name) ?? OPTIONS_BY_NAME.get(name)?.default ?? '';
      if (normaliseValue(name, value) === normaliseValue(name, current)) continue;
      this.values.set(name, value);
      this.panel.setValue(name, value);
      this.ai.setValue(name, value);
      changed = true;
    }
    if (!changed) return;
    this.syncSpeed();
    this.applyHide();
    this.notifyOptions();
  }

  /**
   * Gource's progress slider is the page's seek bar here: `hide=progress`
   * (also implied by `mouse`) hides it, and `hide=mouse` hides the cursor
   * over the canvas.
   */
  private applyHide(): void {
    let hide = '';
    try {
      hide = this.engine?.get_option('hide') ?? this.values.get('hide') ?? '';
    } catch {
      hide = this.values.get('hide') ?? '';
    }
    const items = new Set(hide.split(','));
    const prHidden = String(items.has('pull-requests'));
    this.values.set('hide-pull-requests', prHidden);
    this.panel.setValue('hide-pull-requests', prHidden);
    this.seekWrap.classList.toggle('is-concealed', items.has('progress') || items.has('mouse'));
    this.canvas.classList.toggle('hide-cursor', items.has('mouse'));
  }

  private applyFilters(values: Array<[string, string]>): { name: string; message: string } | null {
    const engine = this.engine;
    if (!engine) return { name: values[0]?.[0] ?? '', message: 'The engine is not running yet.' };
    if (this.recording) return { name: values[0]?.[0] ?? '', message: RECORDING_LOCKED };
    // All or nothing: a rejected filter puts back the ones set before it, so
    // the engine, the panel and the page URL keep agreeing.
    const applied: Array<[string, string]> = [];
    for (const [name, value] of values) {
      let previous: string | undefined;
      try {
        previous = engine.get_option(name);
      } catch {
        previous = undefined;
      }
      try {
        engine.set_option(name, value);
      } catch (err) {
        for (const [n, v] of applied.reverse()) {
          try {
            engine.set_option(n, v);
          } catch (restoreErr) {
            console.warn(`Could not restore ${n}:`, restoreErr);
          }
        }
        return { name, message: err instanceof Error ? err.message : String(err) };
      }
      applied.push([name, previous ?? this.values.get(name) ?? OPTIONS_BY_NAME.get(name)?.default ?? '']);
    }
    for (const [name, value] of values) this.values.set(name, value);
    engine.restart();
    this.notifyOptions();
    return null;
  }

  private notifyOptions(): void {
    this.opts.onOptionsChanged(new Map(this.values));
  }

  // --- lifecycle ---

  /** Starts playing; `focus` moves keyboard focus to the canvas (not when the page is embedded in another). */
  show(focus = true): void {
    if (this.view.playing || this.disposed) return;
    this.statusTimer = window.setInterval(() => this.pollStatus(), STATUS_INTERVAL_MS);
    this.view.start(focus);
    this.pollStatus();
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    window.clearInterval(this.statusTimer);
    this.record.dispose();
    for (const fn of this.cleanups) fn();
    this.panel.dispose();
    this.ai.dispose();
    this.view.dispose();
    this.engine = null;
  }

  // --- loading / errors ---

  updateLoading(state: PlaybackState): void {
    this.record.updateLoading(state);
    const l = state.loading;
    // Before the viewer is ready an error belongs to the analysis screen.
    if (state.error && !state.ready) return;
    if (state.error && !l.finished) {
      this.failLoading(state);
      return;
    }
    this.ai.setSource({ aiDetect: state.aiDetect, finished: l.finished });
    const frac = loadingFraction(state);
    this.loadingBox.hidden = l.finished;
    this.loadingBox.classList.toggle('is-indeterminate', frac === null);
    this.loadingFill.style.width = `${((frac ?? 0) * 100).toFixed(1)}%`;
    const size = state.done?.bytes ?? l.totalBytes;
    this.loadingText.textContent =
      `Loading history · ${formatCount(l.commits)} commits · ` +
      (size ? `${formatBytes(l.bytes)} of ${formatBytes(size)}` : formatBytes(l.bytes));
    if (state.error) this.showBanner(state.error.message || 'Something went wrong.');
  }

  /**
   * The download or the job failed after the viewer appeared: the engine
   * keeps playing what arrived, the loading indicator gives way to a
   * persistent "incomplete" note, and the banner offers a retry.
   */
  private failLoading(state: PlaybackState): void {
    if (this.incomplete) return;
    this.incomplete = true;
    this.loadingBox.hidden = true;
    const commits = formatCount(state.loading.commits);
    this.incompleteNote.replaceChildren(
      `History incomplete · ${commits} commits loaded`,
      h('button.link-btn', { type: 'button', onclick: () => this.opts.onRetry() }, 'Retry'),
    );
    this.incompleteNote.hidden = false;
    this.showBanner(
      `The history stopped loading after ${commits} commits: ${state.error?.message || 'the download failed.'}`,
      { label: 'Retry', run: () => this.opts.onRetry() },
    );
    this.pollStatus();
  }

  showBanner(message: string, action?: { label: string; run: () => void }): void {
    this.banner.hidden = false;
    this.banner.replaceChildren();
    append(
      this.banner,
      icon('alert'),
      h('span.viewer-banner-text', {}, message),
      action && h('button.btn.btn-small.btn-primary', { type: 'button', onclick: () => action.run() }, icon('refresh'), action.label),
      h('button.icon-btn', { type: 'button', 'aria-label': 'Dismiss', onclick: () => (this.banner.hidden = true) }, icon('close')),
    );
  }

  // --- status / playback ---

  private pollStatus(): void {
    const engine = this.engine;
    if (!engine) return;
    let status: EngineStatus | null;
    try {
      status = parseStatus(engine.status());
    } catch {
      status = null;
    }
    if (!status) return;
    this.updatePrHoverTitle(engine);
    this.lastStatus = status;
    if (this.recording && !status.paused) this.engine?.set_paused(true);
    // Some key effects land on the next tick (the `s` recolour picks a new seed then).
    this.syncFromEngine();
    this.playBtn.replaceChildren(icon(status.paused ? 'play' : 'pause'));
    this.playBtn.setAttribute('aria-label', status.paused ? 'Play' : 'Pause');
    this.playBtn.dataset.paused = String(status.paused);
    this.seek.disabled = !status.seekable || this.recording;
    this.seek.title = this.recording
      ? RECORDING_LOCKED
      : status.seekable
      ? 'Seek'
      : this.incomplete
        ? 'Seeking is unavailable: the history did not load completely'
        : 'Seeking unlocks once the whole history has loaded';
    if (!this.seeking) this.seek.value = String(Math.round(status.fraction * SEEK_STEPS));
    this.seek.style.setProperty('--fill', `${(status.fraction * 100).toFixed(2)}%`);

    const played = status.commits_loaded > 0 ? `${formatCount(status.commits_played)} / ${formatCount(status.commits_loaded)} commits` : 'No commits yet';
    this.statusLine.replaceChildren(
      h('span.status-date', {}, status.date || '—'),
      h('span.status-sep', { 'aria-hidden': 'true' }),
      h('span', { 'data-stat': 'commits' }, played),
      h('span', { 'data-stat': 'files' }, `${formatCount(status.files)} files`),
      h('span', { 'data-stat': 'users' }, `${formatCount(status.users)} users`),
    );

    if (!this.debugBox.hidden) {
      const ms = (v: number) => v.toFixed(2).padStart(6);
      this.debugBox.textContent = [
        `fps        ${status.fps.toFixed(1)}`,
        `frame   ${ms(status.frame_ms)} ms`,
        `sim     ${ms(status.sim_ms)} ms`,
        `build   ${ms(status.build_ms)} ms`,
        `gpu     ${ms(status.gpu_ms)} ms`,
        `draws      ${status.draw_calls}`,
        `vertices   ${formatCount(status.vertices)}`,
        `dirs       ${formatCount(status.dirs)}`,
      ].join('\n');
    }
  }

  private updatePrHoverTitle(engine: GourceApp): void {
    let title = '';
    try {
      const value: unknown = engine.pr_hover ? JSON.parse(engine.pr_hover()) : null;
      if (value && typeof value === 'object' && !Array.isArray(value)) {
        const hover = value as Record<string, unknown>;
        if (['title', 'owner', 'actor', 'action', 'age_days'].every((key) => typeof hover[key] === 'string')) {
          title = `${hover.title}\nOwner: ${hover.owner}\nState: ${hover.action}\nLast actor: ${hover.actor}\nAge: ${hover.age_days} days`;
          if (typeof hover.target === 'string') title += `\nRequested: ${hover.target}`;
        }
      }
    } catch {
      // Optional read-only fallback must never interrupt playback.
    }
    // Property assignment is text, not HTML. Non-Latin and markup-like titles
    // remain complete and escaped without a remote font or DOM insertion.
    if (this.canvas.title !== title) this.canvas.title = title;
  }

  private togglePause(): void {
    const engine = this.engine;
    if (!engine || this.recording) return;
    const paused = !(this.lastStatus?.paused ?? false);
    engine.set_paused(paused);
    this.pollStatus();
  }

  private syncSpeed(): void {
    const value = this.values.get('seconds-per-day') ?? OPTIONS_BY_NAME.get('seconds-per-day')!.default;
    const n = Number(value);
    if (!SPEED_PRESETS.includes(n) && ![...this.speed.options].some((o) => o.value === String(n))) {
      const custom = h('option', { value: String(n), 'data-custom': 'true' }, speedLabel(n));
      this.speed.querySelector('[data-custom]')?.remove();
      const after = [...this.speed.options].find((o) => Number(o.value) > n);
      this.speed.insertBefore(custom, after ?? null);
    }
    this.speed.value = String(n);
  }

  private bindSeek(): void {
    const seek = this.seek;
    const fractionAt = (clientX: number) => {
      const rect = seek.getBoundingClientRect();
      return Math.min(1, Math.max(0, (clientX - rect.left) / Math.max(1, rect.width)));
    };
    seek.addEventListener('pointerdown', () => (this.seeking = true));
    const end = () => (this.seeking = false);
    seek.addEventListener('pointerup', end);
    seek.addEventListener('pointercancel', end);
    seek.addEventListener('change', end);
    seek.addEventListener('input', () => {
      if (!this.engine || seek.disabled || this.recording) return;
      const fraction = Number(seek.value) / SEEK_STEPS;
      this.engine.seek(fraction);
      seek.style.setProperty('--fill', `${(fraction * 100).toFixed(2)}%`);
    });
    const wrap = seek.parentElement ?? seek;
    seek.addEventListener('pointermove', (e) => {
      if (!this.engine || !this.lastStatus || this.lastStatus.commits_loaded === 0) return;
      const f = fractionAt(e.clientX);
      const date = this.engine.date_at(f);
      if (!date) return;
      this.seekTip.textContent = date;
      this.seekTip.hidden = false;
      const rect = seek.getBoundingClientRect();
      const wrapRect = wrap instanceof HTMLElement ? wrap.getBoundingClientRect() : rect;
      this.seekTip.style.left = `${e.clientX - wrapRect.left}px`;
    });
    seek.addEventListener('pointerleave', () => (this.seekTip.hidden = true));
  }

  // --- input ---

  /**
   * The element forwards pointers and keys to the engine. Here: re-read the
   * options after a key Gource handled, show a render failure, and Escape,
   * which closes the panel or leaves the canvas for the playback bar.
   */
  private bindInput(): void {
    const on = <K extends keyof HTMLElementEventMap>(target: HTMLElement | Window, type: K, fn: (e: HTMLElementEventMap[K]) => void) => {
      target.addEventListener(type, fn as EventListener);
      this.cleanups.push(() => target.removeEventListener(type, fn as EventListener));
    };
    this.view.addEventListener('gource-key', () => this.pollStatus());
    this.view.addEventListener('gource-render-error', (e) => this.showBanner(`Rendering stopped: ${e.detail.message}`));
    on(window, 'keydown', (e) => {
      if (e.key !== 'Escape' || !this.view.playing || !this.engine || e.metaKey || e.altKey) return;
      const target = e.target as HTMLElement | null;
      if (target !== this.canvas && target !== document.body && target !== this.el) return;
      if (this.panel.el.classList.contains('is-open')) this.togglePanel(false);
      else if (target === this.canvas) this.playBtn.focus();
      else return;
      e.preventDefault();
    });
  }

  /**
   * F3 recording (REC-20): pause the live simulation while a recording runs
   * and restore the previous state afterwards; restart-class options and
   * seeking are locked meanwhile, because a restart would end the pause.
   */
  private setRecording(active: boolean): void {
    const engine = this.engine;
    if (active === this.recording) return;
    this.recording = active;
    if (active) {
      this.pausedBeforeRecording = this.lastStatus?.paused ?? false;
      engine?.set_paused(true);
    } else {
      engine?.set_paused(this.pausedBeforeRecording);
    }
    this.playBtn.disabled = active;
    this.pollStatus();
  }

  // --- chrome ---

  togglePanel(open?: boolean): void {
    const next = open ?? !this.panel.el.classList.contains('is-open');
    this.panel.el.classList.toggle('is-open', next);
    this.el.classList.toggle('panel-open', next);
    this.panelToggle.setAttribute('aria-expanded', String(next));
    if (!next) this.canvas.focus({ preventScroll: true });
  }

  private toggleDebug(): void {
    const show = this.debugBox.hidden;
    this.debugBox.hidden = !show;
    this.debugBtn.setAttribute('aria-pressed', String(show));
    this.pollStatus();
  }

  // --- private repository (F2 section 4.6) ---

  /** Puts the lock badge after the repository name and `controls` (Sign out) first in the toolbar. */
  setPrivate(badge: HTMLElement, controls: HTMLElement[]): void {
    const row = h('span.viewer-name-row');
    this.repoName.replaceWith(row);
    row.append(this.repoName, badge);
    this.tools.prepend(...controls);
  }

  /** Tooltip of "Copy link" (F2 PR-91). */
  setLinkTitle(title: string): void {
    this.shareBtn.title = title;
  }

  private async copyLink(btn: HTMLElement): Promise<void> {
    try {
      await navigator.clipboard.writeText(window.location.href);
      btn.classList.add('is-done');
      window.setTimeout(() => btn.classList.remove('is-done'), 1200);
    } catch {
      window.prompt('Copy this link', window.location.href);
    }
  }
}
