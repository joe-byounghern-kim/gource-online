// SPDX-License-Identifier: GPL-3.0-or-later

// The Record dialog (recording.md section 4): one modal with a screen per
// controller state. `configuring` edits a copy of the live settings (REC-5)
// and shows the plan, the encoder ladder result, a preview and the exact
// dry run; Record picks the file in the click handler (REC-15) and starts
// the export loop (controller.ts); `running` shows progress and the cancel
// choice; `done`, `failed` and `cancelled` end it.

import { SNAPSHOT_OPTION_KEYS, SNAPSHOT_RECORD_KEYS } from '@gource-online/protocol';
import { h } from '../dom';
import { playerHost } from '../host';
import type { GourceApp } from '@gource-online/viewer';
import { ApiError, parseWasmError, type WasmError } from '../errors';
import { formatBytes, formatCount, formatElapsed } from '../format';
import { HIDE_ELEMENTS } from '@gource-online/viewer/options';
import { Recording, type Progress, type RunResult } from './controller';
import { chooseCandidate, hardwareWords, profileName, type LadderChoice } from './encoder';
import { loadRecordFactory } from './factory';
import type { RecordHost } from './feature';
import { recordIcon } from './feature';
import { hooksActive, testConfig } from './hooks';
import {
  BLOB_CAP,
  FRAME_RATES,
  RECORD_OPTIONS_BY_NAME,
  RECORD_OPTION_DEFS,
  SIZE_PRESETS,
  fileStem,
  loadPresets,
  loadRemembered,
  saveRemembered,
  savePresets,
  type RecordUrlParams,
} from './options';
import { FileSink, MemorySink, sinkError, type FileHandleLike, type Sink } from './sink';
import { validSnapshotSetting } from './snapshot';
import { parsePlan, type PlannerResult, type RecordFactory, type RecordPlan, type RecorderLike, type SnapshotRecordSettings } from './types';

/** Build identity written into the recipe and the file (REC-70). */
const BUILD = (import.meta.env.VITE_BUILD_ID as string | undefined) || 'dev';
/** Preview: the run this many simulated seconds after the start (REC-7). */
const PREVIEW_SECONDS = 5;
const PREVIEW_DELAY_MS = 400;
const PLAN_DELAY_MS = 120;
const SLICE_MS = 8;
/** REC-124 thresholds. */
const CONFIRM_BYTES = 2e9;
const CONFIRM_SECONDS = 3600;

const HIDE_LABELS: Record<string, string> = {
  date: 'Date',
  users: 'Users',
  tree: 'Tree',
  files: 'Files',
  usernames: 'User names',
  filenames: 'File names',
  dirnames: 'Directory names',
  bloom: 'Bloom',
  root: 'Root directory',
  decorations: 'AI decorations',
  'pull-requests': 'Pull requests',
};

/** Hide elements the recording never shows anyway (REC-11). */
const NOT_OFFERED = new Set(['mouse', 'progress']);

const WARNINGS: Record<string, string> = {
  long_render: 'The video can be over an hour long; rendering it can take about as long.',
  crash_loses_file: 'A crash or closing the tab loses the whole file.',
  speed_very_fast: 'The speed is very fast (under 0.05 seconds per day).',
  speed_very_slow: 'The speed is very slow (over 30 seconds per day).',
  software_encoder_slow: 'Software encoders may be slow at this size and frame rate.',
  tail_may_cut_users: 'Below time scale 1 a fixed tail may cut fading users off; use a longer tail or "until every user has left".',
};

const ERROR_TEXT: Record<string, string> = {
  record_unsupported_browser: 'This browser cannot encode video here. Chrome and Edge, opened over https or localhost, are tested.',
  record_empty_run: 'No activity in this range',
  record_log_incomplete: 'The full history has not arrived yet.',
  record_unbounded: 'A looping recording needs a maximum length.',
  record_no_encoder: 'This browser cannot encode video at any size we tried. Try Chrome or Edge, or lower the resolution.',
  record_gl_failed: 'The graphics context was lost; the recording stopped.',
  record_mux_failed: 'The video file could not be assembled.',
  record_mux_reorder: 'The video file could not be assembled (the encoder reordered frames).',
};

function errorText(e: WasmError): string {
  if (e.code === 'record_invalid_option' || e.code === 'record_size_unsupported' || e.code === 'record_encoder_failed') return e.text;
  if (e.code === 'record_disk_full' || e.code === 'record_sink_failed') return e.text;
  return ERROR_TEXT[e.code] ?? e.text;
}

/** `mm:ss` or `h:mm:ss` of a duration in seconds. */
function clock(seconds: number): string {
  return formatElapsed(Math.max(0, seconds) * 1000);
}

type Screen = 'config' | 'confirm' | 'progress' | 'done' | 'failed' | 'cancelled';
type Range = 'all' | 'current' | 'dates';
type Ends = 'tail' | 'until-left' | 'loop';

/** Everything the configuration screen edits. */
interface Form {
  size: string;
  custom: string;
  fps: string;
  quality: string;
  spd: string;
  timeScale: string;
  autoSkip: string;
  range: Range;
  startDate: string;
  stopDate: string;
  ends: Ends;
  tail: string;
  max: string;
  hidden: Set<string>;
  key: boolean;
  agentKey: boolean;
  titleOn: boolean;
  title: string;
  name: string;
  metadata: string;
  hardware: string;
  bitrate: string;
  keyframe: string;
  layout: string;
  /** Gource options from an imported recipe that the dialog has no field for. */
  extra: Record<string, string>;
}

interface DialogCallbacks {
  head(): string;
  returnFocus(): void;
  runningChanged(running: boolean): void;
}

interface FieldRefs {
  [key: string]: HTMLInputElement | HTMLSelectElement;
}

export class RecordDialog {
  readonly el: HTMLElement;
  private readonly host: RecordHost;
  private readonly cb: DialogCallbacks;
  private readonly panel: HTMLElement;
  private readonly titleEl: HTMLElement;
  private readonly body: HTMLElement;
  private readonly foot: HTMLElement;
  private readonly live: HTMLElement;
  private factory: RecordFactory | null = null;
  private recorder: RecorderLike | null = null;
  private recorderJson = '';
  private form!: Form;
  private refs: FieldRefs = {};
  private plan: RecordPlan | null = null;
  private planJson = '';
  private planError: WasmError | null = null;
  private choice: LadderChoice | null = null;
  private choicePromise: Promise<LadderChoice | null> | null = null;
  private choiceKey = '';
  private choiceDone = false;
  private exact: { json: string; result: PlannerResult } | null = null;
  private dryRunToken = 0;
  private previewToken = 0;
  private planTimer: number | undefined;
  private previewTimer: number | undefined;
  /** Options that came from the URL or a recipe and were not typed (REC-124). */
  private supplied = new Set<string>();
  private confirmed = false;
  private screen: Screen = 'config';
  private recording: Recording | null = null;
  private progressEls: Record<string, HTMLElement> = {};
  private thumbCtx: CanvasRenderingContext2D | null = null;
  private lastAnnounced = -1;
  private result: RunResult | null = null;
  private sink: Sink | null = null;
  private runError: WasmError | null = null;
  private outputUrl = '';
  private wakeLock: { release(): Promise<void> } | null = null;
  private titleBefore = '';
  private preview: HTMLCanvasElement | null = null;
  private opener: Element | null = null;
  private disposed = false;
  /** `stopAndSaveNow` callers waiting for the run to end. */
  private readonly runEndWaiters: Array<() => void> = [];
  private sourceConfirmed = false;
  private recipeNote = '';
  private finishedBlob: Blob | null = null;

  constructor(host: RecordHost, cb: DialogCallbacks) {
    this.host = host;
    this.cb = cb;
    this.titleEl = h('h2.rec-title#rec-title', {}, 'Record video');
    this.body = h('div.rec-body');
    this.foot = h('div.rec-foot');
    this.live = h('div.rec-live', { 'aria-live': 'polite', role: 'status' });
    const close = h('button.icon-btn.rec-close', { type: 'button', 'aria-label': 'Close', onclick: () => this.escape() }, closeIcon());
    this.panel = h(
      'section.rec-dialog',
      { role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': 'rec-title', tabindex: -1 },
      h('header.rec-head', {}, recordIcon(), this.titleEl, close),
      hooksActive() ? h('p.rec-testmode', {}, 'Test mode: nothing will be saved.') : null,
      this.body,
      this.foot,
      this.live,
    );
    this.el = h('div.rec-backdrop', { hidden: true }, this.panel);
    this.panel.addEventListener('keydown', (e) => this.keyDown(e));
    this.panel.addEventListener('dragover', (e) => {
      if (this.screen !== 'config') return;
      e.preventDefault();
      this.panel.classList.add('is-dropping');
    });
    this.panel.addEventListener('dragleave', () => this.panel.classList.remove('is-dropping'));
    this.panel.addEventListener('drop', (e) => {
      this.panel.classList.remove('is-dropping');
      if (this.screen !== 'config') return;
      e.preventDefault();
      const file = e.dataTransfer?.files[0];
      if (file) void this.importRecipe(file);
    });
    if (hooksActive()) this.installHook();
  }

  get running(): boolean {
    return this.recording !== null && (this.screen === 'progress' || this.screen === 'failed');
  }

  get isOpen(): boolean {
    return !this.el.hidden;
  }

  /** The same effective form, validator and plan used by F3, with no recorder or encoder. */
  async captureSnapshotSettings(params?: RecordUrlParams): Promise<SnapshotRecordSettings> {
    this.factory ??= await loadRecordFactory(this.host.mock);
    if (this.disposed) throw new DOMException('Cancelled', 'AbortError');
    if (!this.form) this.form = this.initialForm(params);
    const live: Record<string, string> = {};
    for (const name of SNAPSHOT_OPTION_KEYS) live[name] = this.live_(name);
    const request: Record<string, string> = {
      ...Object.fromEntries(RECORD_OPTION_DEFS.map((def) => [def.name, def.default])),
      ...live, ...this.options(), 'record-name': '', 'record-metadata': 'none',
    };
    if (this.form.range === 'current' || request['start-position'] || request['stop-position']) {
      throw new ApiError('invalid_option', 'Snapshots cannot freeze position ranges. Choose Whole history or Dates in Record video before saving.');
    }
    const normalized = JSON.parse(this.factory.parseOptions(JSON.stringify(request))) as Record<string, string>;
    const plan = parsePlan(this.factory.plan(this.engine(), JSON.stringify(normalized)));
    if (![plan.fps, plan.width, plan.height].every(Number.isSafeInteger) ||
      plan.fps <= 0 || plan.fps > 60 || plan.width <= 0 || plan.width > 7680 || plan.height <= 0 || plan.height > 4320 ||
      !Number.isFinite(plan.ui_scale) || plan.ui_scale <= 0 || plan.ui_scale > 4) throw new Error('The recording settings exceed snapshot bounds.');
    normalized['record-viewport'] = `${plan.width}x${plan.height}`;
    normalized['record-ui-scale'] = String(plan.ui_scale);
    normalized['record-framerate'] = String(plan.fps);
    const project = (keys: readonly string[]) => {
      const result: Record<string, string> = {};
      for (const name of keys) {
        const value = normalized[name];
        if (value === undefined || value === '') continue;
        if (!validSnapshotSetting(value)) throw new Error('The snapshot settings are invalid.');
        result[name] = value;
      }
      return result;
    };
    const settings = { options: project(SNAPSHOT_OPTION_KEYS), record: project(SNAPSHOT_RECORD_KEYS),
      plan: { fps: plan.fps, width: plan.width, height: plan.height, ui_scale: plan.ui_scale } };
    // Reserve one KiB for build identity and the server-injected snapshot reference
    // within the service's 16 KiB immutable recipe cap.
    if (new TextEncoder().encode(JSON.stringify(settings)).byteLength > 15 * 1024) throw new Error('The snapshot settings are too large.');
    return settings;
  }

  // --- open / close ---

  async open(params?: RecordUrlParams): Promise<void> {
    if (this.isOpen) {
      this.panel.focus();
      return;
    }
    this.opener = document.activeElement;
    this.factory = await loadRecordFactory(this.host.mock);
    this.form = this.initialForm(params);
    this.exact = null;
    this.confirmed = false;
    this.result = null;
    this.runError = null;
    this.el.hidden = false;
    this.showConfig();
    this.panel.focus();
    this.schedulePlan(0);
  }

  focusProgress(): void {
    this.el.hidden = false;
    this.panel.focus();
  }

  /** Esc and the close button (REC-94). */
  private escape(): void {
    switch (this.screen) {
      case 'progress':
        if (this.body.querySelector('.rec-cancel-choice')) this.keepRecording();
        else this.askCancel();
        return;
      case 'failed':
        if (this.recording) return;
        this.close();
        return;
      case 'confirm':
        this.showConfig();
        return;
      default:
        this.close();
    }
  }

  close(): void {
    if (this.recording) return;
    this.dryRunToken++;
    this.previewToken++;
    window.clearTimeout(this.planTimer);
    window.clearTimeout(this.previewTimer);
    this.releaseRecorder();
    if (this.outputUrl) URL.revokeObjectURL(this.outputUrl);
    this.outputUrl = '';
    this.finishedBlob = null;
    this.el.hidden = true;
    this.body.replaceChildren(...kids());
    this.foot.replaceChildren(...kids());
    this.screen = 'config';
    if (this.opener instanceof HTMLElement && this.opener.isConnected) this.opener.focus({ preventScroll: true });
    else this.cb.returnFocus();
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    if (this.recording) {
      this.recording.requestStop('discard');
      void this.recording.discard();
      this.endRun();
    }
    // A file saved just before (a private run ended by sign-out stops and
    // saves first, F2 PR-121) may still be downloading: revoke its URL later.
    const url = this.outputUrl;
    const blob = this.finishedBlob;
    this.outputUrl = '';
    if (url) window.setTimeout(() => URL.revokeObjectURL(url), 60_000);
    this.close();
    if (hooksActive()) this.finishedBlob = blob;
    this.el.remove();
  }

  /** REC-118: free, lose the context, drop the drawing buffer. */
  private releaseRecorder(): void {
    const rec = this.recorder;
    this.recorder = null;
    this.recorderJson = '';
    if (!rec) return;
    let canvas: HTMLCanvasElement | null = null;
    try {
      canvas = rec.canvas;
    } catch {
      canvas = null;
    }
    try {
      rec.free();
    } catch {
      // Already freed.
    }
    if (canvas) {
      try {
        (canvas.getContext('webgl2') as WebGL2RenderingContext | null)?.getExtension('WEBGL_lose_context')?.loseContext();
      } catch {
        // A 2D (mock) canvas.
      }
      canvas.width = 0;
      canvas.height = 0;
    }
  }

  private keyDown(e: KeyboardEvent): void {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      this.escape();
      return;
    }
    if (e.key !== 'Tab') return;
    // Focus trap (REC-94).
    const focusable = [...this.panel.querySelectorAll<HTMLElement>('button, input, select, textarea, a[href], summary, [tabindex="0"]')].filter(
      (el) => !el.hasAttribute('disabled') && el.offsetParent !== null,
    );
    if (focusable.length === 0) return;
    const first = focusable[0]!;
    const last = focusable[focusable.length - 1]!;
    if (e.shiftKey && (document.activeElement === first || document.activeElement === this.panel)) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }

  private announce(text: string): void {
    this.live.textContent = text;
  }

  // --- the form ---

  private engine(): GourceApp {
    const e = this.host.engine();
    if (!e) throw new Error('record_log_incomplete: The engine is not running.');
    return e;
  }

  private live_(name: string): string {
    try {
      return this.host.engine()?.get_option(name) ?? this.host.values().get(name) ?? '';
    } catch {
      return this.host.values().get(name) ?? '';
    }
  }

  private publicSource(): boolean {
    return this.host.source.kind === 'repo' && !this.host.source.private;
  }

  private initialForm(params?: RecordUrlParams): Form {
    const remembered = loadRemembered();
    const url = params?.options ?? {};
    const rec = (name: string) => url[name] ?? remembered[name] ?? RECORD_OPTIONS_BY_NAME.get(name)!.default;
    this.supplied = new Set(Object.keys(url));
    const viewport = rec('record-viewport');
    const presets = SIZE_PRESETS.map((p) => p.value);
    const size = viewport === '' ? 'match' : presets.includes(viewport) ? viewport : 'custom';
    const tail = rec('record-tail-seconds');
    const max = rec('record-max-seconds');
    const loop = this.live_('loop') === 'true';
    const start = this.live_('start-date');
    const stop = this.live_('stop-date');
    const liveTitle = this.live_('title');
    const hidden = new Set(this.live_('hide').split(',').filter((s) => s !== '' && !NOT_OFFERED.has(s)));
    return {
      size,
      custom: size === 'custom' ? viewport : '',
      fps: rec('record-framerate'),
      quality: rec('record-quality'),
      spd: this.live_('seconds-per-day') || '10',
      timeScale: this.live_('time-scale') || '1',
      autoSkip: this.live_('auto-skip-seconds') || '3',
      range: start || stop ? 'dates' : 'all',
      startDate: start,
      stopDate: stop,
      ends: loop ? 'loop' : tail === '-1' ? 'until-left' : 'tail',
      tail: tail === '-1' ? '5' : tail,
      max: max === '0' && loop ? '60' : max === '0' ? '' : max,
      hidden,
      key: this.live_('key') === 'true',
      agentKey: this.live_('agent-key') === 'true',
      titleOn: true,
      title: liveTitle || (this.publicSource() ? this.host.display : ''),
      name: url['record-name'] ?? '',
      metadata: url['record-metadata'] ?? (this.publicSource() ? 'embed' : 'none'),
      hardware: rec('record-hardware'),
      bitrate: rec('record-bitrate'),
      keyframe: rec('record-keyframe-seconds'),
      layout: rec('record-layout'),
      extra: { 'record-ui-scale': rec('record-ui-scale'), 'record-format': rec('record-format') },
    };
  }

  /** The hide elements the engine accepts, in Gource's order, plus F1's `decorations`. */
  private hideElements(): string[] {
    const candidates = [...new Set<string>([...HIDE_ELEMENTS, 'decorations'])].filter((e) => !NOT_OFFERED.has(e));
    const f = this.factory!;
    return candidates.filter((e) => {
      try {
        f.parseOptions(JSON.stringify({ hide: e }));
        return true;
      } catch {
        return false;
      }
    });
  }

  private supportsAgentKey(): boolean {
    try {
      return this.host.engine()?.get_option('agent-key') !== undefined;
    } catch {
      return false;
    }
  }

  /** Viewport value of the form: `''` for match window. */
  private viewport(): string {
    const f = this.form;
    if (f.size === 'match') return '';
    if (f.size === 'custom') return f.custom.trim();
    return f.size;
  }

  /** The dialog's request: Gource overrides plus `record-*` options (recording.md section 5.2). */
  private options(): Record<string, string> {
    const f = this.form;
    const o: Record<string, string> = { ...f.extra };
    o['seconds-per-day'] = f.spd.trim();
    o['time-scale'] = f.timeScale.trim();
    o['auto-skip-seconds'] = f.autoSkip.trim();
    o['hide'] = [...f.hidden].join(',');
    o['title'] = f.titleOn ? f.title : '';
    o['key'] = String(f.key);
    if (this.supportsAgentKey()) o['agent-key'] = String(f.agentKey);
    o['start-position'] = '';
    o['start-date'] = '';
    o['stop-date'] = '';
    if (f.range === 'current') {
      const fraction = this.host.status()?.fraction ?? 0;
      if (fraction > 0 && fraction < 1) o['start-position'] = String(Math.round(fraction * 1e6) / 1e6);
    } else if (f.range === 'dates') {
      o['start-date'] = f.startDate.trim();
      o['stop-date'] = f.stopDate.trim();
    }
    o['loop'] = String(f.ends === 'loop');
    o['record-framerate'] = f.fps;
    o['record-viewport'] = this.viewport();
    o['record-quality'] = f.quality;
    o['record-tail-seconds'] = f.ends === 'until-left' ? '-1' : f.tail.trim() || '5';
    o['record-max-seconds'] = f.max.trim() || '0';
    o['record-metadata'] = f.metadata;
    o['record-name'] = f.name.trim();
    o['record-hardware'] = f.hardware;
    o['record-bitrate'] = f.bitrate.trim() || '0';
    o['record-keyframe-seconds'] = f.keyframe.trim() || '2';
    o['record-layout'] = f.layout;
    return o;
  }

  private optionsJson(): string {
    return JSON.stringify(this.options());
  }

  /** The record-* subset as the dialog would put it in a link or remember it. */
  private recordOptions(): Record<string, string> {
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(this.options())) if (k.startsWith('record-')) out[k] = v;
    return out;
  }

  // --- configuration screen ---

  private showConfig(): void {
    this.screen = 'config';
    this.titleEl.textContent = 'Record video';
    const f = this.form;
    this.refs = {};
    const changed = (key: string) => {
      this.supplied.delete(key);
      this.confirmed = false;
      this.schedulePlan(PLAN_DELAY_MS);
    };
    const radios = <T extends string>(name: string, legend: string, items: Array<{ value: T; label: string; note?: string; disabled?: boolean }>, get: () => T, set: (v: T) => void, key: string) =>
      h(
        'fieldset.rec-field',
        {},
        h('legend', {}, legend),
        h(
          'div.rec-choices',
          {},
          ...items.map((it) => {
            const input = h('input', { type: 'radio', name: `rec-${name}`, value: it.value, checked: get() === it.value, disabled: it.disabled === true });
            input.addEventListener('change', () => {
              if (!input.checked) return;
              set(it.value);
              changed(key);
              if (name === 'size' || name === 'range' || name === 'ends') this.syncVisibility();
            });
            this.refs[`${name}:${it.value}`] = input;
            return h('label.rec-choice', {}, input, h('span', {}, it.label), it.note ? h('small', {}, it.note) : null);
          }),
        ),
      );
    const text = (key: keyof Form & string, label: string, attrs: Record<string, string | number | boolean> = {}, optKey: string = key) => {
      const id = `rec-in-${key}`;
      const input = h('input.rec-input', { id, type: 'text', value: String(f[key] ?? ''), autocomplete: 'off', spellcheck: 'false', ...attrs });
      input.addEventListener('input', () => {
        (f as unknown as Record<string, string>)[key] = input.value;
        changed(optKey);
      });
      this.refs[key] = input;
      return h('label.rec-inline', { for: id }, h('span', {}, label), input);
    };

    const match = this.matchLabel();
    const sizeField = radios(
      'size',
      'Size',
      [
        { value: 'match', label: 'Match window', note: match },
        ...SIZE_PRESETS.map((p) => ({ value: p.value, label: p.label, note: p.note })),
        { value: 'custom', label: 'Custom' },
      ],
      () => f.size,
      (v) => (f.size = v),
      'record-viewport',
    );
    const customInput = text('custom', 'Width x height', { placeholder: '1600x900', 'aria-describedby': 'rec-err-viewport' }, 'record-viewport');
    customInput.classList.add('rec-custom');
    sizeField.append(customInput, h('p.rec-error#rec-err-viewport', { 'data-for': 'record-viewport' }));
    sizeField.append(h('p.rec-note', {}, 'Camera: automatic. Your zoom and rotation are not recorded.'));

    const fpsField = radios(
      'fps',
      'Frame rate',
      FRAME_RATES.map((v) => ({ value: v, label: `${v} fps` })),
      () => f.fps,
      (v) => (f.fps = v),
      'record-framerate',
    );
    const qualityField = radios(
      'quality',
      'Quality',
      [
        { value: 'standard', label: 'Standard' },
        { value: 'high', label: 'High' },
        { value: 'max', label: 'Max' },
      ],
      () => f.quality,
      (v) => (f.quality = v),
      'record-quality',
    );

    const fit = h('input.rec-input.rec-short', { id: 'rec-fit', type: 'number', min: 1, step: 1, placeholder: 'seconds', 'aria-label': 'Target video length in seconds' });
    const fitBtn = h('button.btn.btn-small', { type: 'button', onclick: () => this.fitTo(Number(fit.value)) }, 'Fit');
    const speedField = h(
      'fieldset.rec-field',
      {},
      h('legend', {}, 'Speed'),
      h(
        'div.rec-row',
        {},
        text('spd', 'Seconds per day', { inputmode: 'decimal' }, 'seconds-per-day'),
        text('timeScale', 'Time scale', { inputmode: 'decimal' }, 'time-scale'),
        text('autoSkip', 'Auto-skip after (s)', { inputmode: 'decimal' }, 'auto-skip-seconds'),
      ),
      h('div.rec-row', {}, h('label.rec-inline', { for: 'rec-fit' }, h('span', {}, 'Fit to'), fit), fitBtn),
      h('p.rec-note', {}, 'Fit sets seconds per day from the history span; auto-skip can make the video shorter.'),
      h('p.rec-error', { 'data-for': 'seconds-per-day time-scale auto-skip-seconds' }),
    );

    const fraction = this.host.status()?.fraction ?? 0;
    const rangeField = radios(
      'range',
      'Range',
      [
        { value: 'all', label: 'Whole history' },
        { value: 'current', label: 'From current position', disabled: !(fraction > 0 && fraction < 1) },
        { value: 'dates', label: 'Dates' },
      ],
      () => f.range,
      (v) => (f.range = v),
      'start-date',
    );
    const dates = h('div.rec-row.rec-dates', {}, text('startDate', 'From', { placeholder: 'YYYY-MM-DD' }, 'start-date'), text('stopDate', 'To', { placeholder: 'YYYY-MM-DD' }, 'stop-date'));
    rangeField.append(dates, h('p.rec-note.rec-current-note', {}, 'Starting mid-history shows only the files changed from there on.'), h('p.rec-error', { 'data-for': 'start-date stop-date start-position' }));

    const endsField = radios(
      'ends',
      'Ends',
      [
        { value: 'tail', label: 'After the last commit' },
        { value: 'until-left', label: 'Until every user has left (about 12 s after the last commit)' },
        { value: 'loop', label: 'Loop' },
      ],
      () => f.ends,
      (v) => {
        f.ends = v;
        if (v === 'loop' && !f.max.trim()) {
          f.max = '60';
          const m = this.refs['max'] as HTMLInputElement | undefined;
          if (m) m.value = '60';
        }
      },
      'record-tail-seconds',
    );
    endsField.append(
      h('div.rec-row', {}, text('tail', 'Tail (s)', { inputmode: 'decimal' }, 'record-tail-seconds'), text('max', 'Maximum length (s)', { inputmode: 'decimal', placeholder: 'none' }, 'record-max-seconds')),
      h('p.rec-error', { 'data-for': 'record-tail-seconds record-max-seconds loop' }),
    );

    const show = h('div.rec-show');
    for (const el of this.hideElements()) {
      const input = h('input', { type: 'checkbox', checked: !f.hidden.has(el) });
      input.addEventListener('change', () => {
        if (input.checked) f.hidden.delete(el);
        else f.hidden.add(el);
        changed('hide');
      });
      this.refs[`show:${el}`] = input;
      show.append(h('label.rec-check', {}, input, h('span', {}, HIDE_LABELS[el] ?? el[0]!.toUpperCase() + el.slice(1))));
    }
    const check = (key: 'key' | 'agentKey' | 'titleOn', label: string, optKey: string) => {
      const input = h('input', { type: 'checkbox', checked: f[key] });
      input.addEventListener('change', () => {
        f[key] = input.checked;
        changed(optKey);
        this.syncVisibility();
      });
      this.refs[key] = input;
      return h('label.rec-check', {}, input, h('span', {}, label));
    };
    show.append(check('key', 'File extension key', 'key'));
    if (this.supportsAgentKey()) show.append(check('agentKey', 'Agent key', 'agent-key'));
    const titleRow = h('div.rec-row', {}, check('titleOn', 'Title', 'title'), text('title', 'Title text', { placeholder: 'No title' }, 'title'));
    const showField = h('fieldset.rec-field', {}, h('legend', {}, 'Show'), show, titleRow, h('p.rec-note', {}, 'The video can contain flashing (beams and bloom); untick Bloom to soften it.'));
    if (this.live_('ai-filter') && this.live_('ai-filter') !== 'all') showField.append(h('p.rec-note', {}, 'The video will show the AI filter line.'));
    // A repository is known to be private; a log a page handed over only may be.
    if (this.host.source.private) showField.append(h('p.rec-warn', {}, this.host.source.kind === 'repo'
      ? 'The video shows names and paths from a private repository.'
      : 'This log may come from a private repository: the video shows its names and paths.'));

    // Preview, estimate and encoder.
    this.preview = h('canvas.rec-preview', { width: 320, height: 180, 'aria-label': 'Preview of the recording' });
    const previewBox = h(
      'div.rec-preview-box',
      {},
      this.preview,
      h('button.btn.btn-small', { type: 'button', onclick: () => this.schedulePreview(0) }, 'Refresh preview'),
      h('span.rec-preview-state', {}),
    );
    const summary = h(
      'div.rec-summary',
      {},
      h('p.rec-estimate', { 'aria-live': 'polite' }),
      h('button.btn.btn-small.rec-widen', { type: 'button', hidden: true, onclick: () => this.useEntireHistory() }, 'Use entire history'),
      h('div.rec-dry', {}, h('button.btn.btn-small.rec-dry-btn', { type: 'button', onclick: () => void this.dryRun() }, 'Calculate exact length'), h('span.rec-dry-state', {})),
      h('p.rec-encoder', {}),
      h('ul.rec-warnings', {}),
    );

    const details = this.detailsBox();
    const presets = this.presetsBox();

    this.body.replaceChildren(...kids(
      h('div.rec-top', {}, previewBox, summary),
      h('div.rec-grid', {}, sizeField, h('div.rec-col', {}, fpsField, qualityField), speedField, rangeField, endsField, showField),
      details,
      presets,
      this.recipeNote ? h('p.rec-note', {}, this.recipeNote) : null,
      this.supplied.size > 0 ? h('p.rec-warn.rec-supplied', {}, 'Some recording options came from the link or a recipe; you will be asked to confirm them.') : null,
    ));

    const still = h('button.btn.rec-still', { type: 'button', onclick: () => this.saveStill() }, 'Save frame as PNG');
    const recordBtn = h('button.btn.btn-primary.rec-start', { type: 'button', onclick: () => this.recordClicked() }, recordIcon(), 'Record…');
    this.foot.replaceChildren(...kids(still, h('span.rec-spacer'), h('button.btn', { type: 'button', onclick: () => this.close() }, 'Cancel'), recordBtn));
    this.syncVisibility();
    this.renderPlan();
  }

  private matchLabel(): string {
    const p = this.plan;
    if (p && p.match_window) return `${p.width}x${p.height}`;
    const s = this.host.status();
    const w = s?.width;
    const hh = s?.height;
    return w && hh ? `${w - (w % 2)}x${hh - (hh % 2)}` : 'the window';
  }

  private syncVisibility(): void {
    const f = this.form;
    const q = (sel: string) => this.body.querySelector<HTMLElement>(sel);
    const custom = q('.rec-custom');
    if (custom) custom.hidden = f.size !== 'custom';
    const dates = q('.rec-dates');
    if (dates) dates.hidden = f.range !== 'dates';
    const note = q('.rec-current-note');
    if (note) note.hidden = f.range !== 'current';
    const tail = this.refs['tail'] as HTMLInputElement | undefined;
    if (tail) tail.disabled = f.ends !== 'tail';
    const title = this.refs['title'] as HTMLInputElement | undefined;
    if (title) title.disabled = !f.titleOn;
  }

  private detailsBox(): HTMLElement {
    const f = this.form;
    const select = (key: 'hardware' | 'layout' | 'metadata', label: string, optKey: string, choices: Array<[string, string]>) => {
      const id = `rec-sel-${key}`;
      const sel = h('select.rec-input', { id }, ...choices.map(([v, l]) => h('option', { value: v, selected: f[key] === v }, l)));
      sel.addEventListener('change', () => {
        f[key] = sel.value;
        this.supplied.delete(optKey);
        this.schedulePlan(PLAN_DELAY_MS);
      });
      this.refs[key] = sel;
      return h('label.rec-inline', { for: id }, h('span', {}, label), sel);
    };
    const input = (key: 'bitrate' | 'keyframe' | 'name', label: string, optKey: string, placeholder: string) => {
      const id = `rec-in-${key}`;
      const el = h('input.rec-input', { id, type: 'text', value: f[key], placeholder, autocomplete: 'off' });
      el.addEventListener('input', () => {
        f[key] = el.value;
        this.supplied.delete(optKey);
        this.schedulePlan(PLAN_DELAY_MS);
      });
      this.refs[key] = el;
      return h('label.rec-inline', { for: id }, h('span', {}, label), el);
    };
    return h(
      'details.rec-details',
      {},
      h('summary', {}, 'Details'),
      h('dl.rec-facts', {}),
      h(
        'div.rec-row.rec-advanced',
        {},
        select('hardware', 'Encoder preference', 'record-hardware', [
          ['auto', 'Automatic'],
          ['prefer-hardware', 'Prefer hardware'],
          ['prefer-software', 'Prefer software'],
        ]),
        input('bitrate', 'Bitrate (Mbps, 0 = quality)', 'record-bitrate', '0'),
        input('keyframe', 'Keyframe every (s)', 'record-keyframe-seconds', '2'),
        select('layout', 'MP4 layout', 'record-layout', [
          ['auto', 'Automatic'],
          ['fragmented', 'Fragmented'],
          ['faststart', 'Faststart'],
        ]),
        select('metadata', 'Recipe in the file', 'record-metadata', [
          ['embed', 'Embed'],
          ['none', 'None'],
        ]),
        input('name', 'File name', 'record-name', 'derived'),
      ),
      h('p.rec-error', { 'data-for': 'record-hardware record-bitrate record-keyframe-seconds record-layout record-metadata record-name' }),
    );
  }

  /** The preset chosen in the list: applying it draws the dialog again, and the list must still show it. */
  private chosenPreset = '';

  private presetsBox(): HTMLElement {
    const presets = loadPresets();
    if (!presets.has(this.chosenPreset)) this.chosenPreset = '';
    const select = h(
      'select.rec-input',
      { id: 'rec-presets', 'aria-label': 'Saved presets' },
      h('option', { value: '' }, presets.size ? 'Load a preset…' : 'No saved presets'),
      ...[...presets.keys()].map((name) => h('option', { value: name }, name)),
    );
    select.value = this.chosenPreset;
    const del = h('button.btn.btn-small', { type: 'button', disabled: this.chosenPreset === '' }, 'Delete');
    select.addEventListener('change', () => {
      this.chosenPreset = select.value;
      del.disabled = select.value === '';
      const values = presets.get(select.value);
      if (values) this.applyValues(values, false);
    });
    // Writes start from storage, not from the list shown: another tab or a
    // sign-out (F2 clears localStorage, PR-22) may have changed it since.
    del.addEventListener('click', () => {
      const stored = loadPresets();
      stored.delete(select.value);
      savePresets(stored);
      this.chosenPreset = '';
      this.showConfig();
    });
    const name = h('input.rec-input', { type: 'text', placeholder: 'Preset name', 'aria-label': 'Preset name', autocomplete: 'off' });
    const save = h('button.btn.btn-small', { type: 'button' }, 'Save preset');
    const status = h('span.rec-preset-state', { role: 'status' });
    save.addEventListener('click', () => {
      const n = name.value.trim();
      if (!n) {
        name.focus();
        return;
      }
      const values = this.presetValues();
      const stored = loadPresets();
      stored.set(n, values);
      presets.set(n, values);
      status.textContent = savePresets(stored) ? `Saved "${n}".` : 'This browser does not allow saving presets.';
      select.append(h('option', { value: n }, n));
    });
    const file = h('input', { type: 'file', accept: '.json,application/json', hidden: true });
    file.addEventListener('change', () => {
      const f = file.files?.[0];
      if (f) void this.importRecipe(f);
      file.value = '';
    });
    const load = h('button.btn.btn-small', { type: 'button', onclick: () => file.click() }, 'Load recipe (.json)…');
    return h(
      'div.rec-presets',
      {},
      h('div.rec-row', {}, select, del),
      h('div.rec-row', {}, name, save, status),
      h('div.rec-row', {}, load, file, h('span.rec-note', {}, 'or drop a recipe file on this dialog')),
    );
  }

  /** Dialog values a preset keeps (no title and no range: they belong to the run). */
  private presetValues(): Record<string, string> {
    const o = this.options();
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(o)) {
      if (['title', 'start-date', 'stop-date', 'start-position', 'record-name'].includes(k)) continue;
      out[k] = v;
    }
    return out;
  }

  /** Applies option values (a preset, or a recipe's `options` + `record`) to the form. */
  private applyValues(values: Record<string, string>, supplied: boolean): void {
    const f = this.form;
    const known = new Set<string>();
    const take = (name: string, fn: (v: string) => void) => {
      const v = values[name];
      if (v === undefined) return;
      fn(v);
      known.add(name);
      if (supplied && name.startsWith('record-')) this.supplied.add(name);
    };
    take('record-viewport', (v) => {
      if (v === '') f.size = 'match';
      else if (SIZE_PRESETS.some((p) => p.value === v)) f.size = v;
      else {
        f.size = 'custom';
        f.custom = v;
      }
    });
    take('record-framerate', (v) => (f.fps = v));
    take('record-quality', (v) => (f.quality = v));
    take('record-tail-seconds', (v) => {
      if (v === '-1') f.ends = f.ends === 'loop' ? 'loop' : 'until-left';
      else f.tail = v;
    });
    take('record-max-seconds', (v) => (f.max = v === '0' ? '' : v));
    take('record-metadata', (v) => (f.metadata = v));
    take('record-name', (v) => (f.name = v));
    take('record-hardware', (v) => (f.hardware = v));
    take('record-bitrate', (v) => (f.bitrate = v));
    take('record-keyframe-seconds', (v) => (f.keyframe = v));
    take('record-layout', (v) => (f.layout = v));
    take('record-ui-scale', (v) => (f.extra['record-ui-scale'] = v));
    take('record-format', (v) => (f.extra['record-format'] = v));
    take('seconds-per-day', (v) => (f.spd = v));
    take('time-scale', (v) => (f.timeScale = v));
    take('auto-skip-seconds', (v) => (f.autoSkip = v));
    take('hide', (v) => (f.hidden = new Set(v.split(',').filter((s) => s !== '' && !NOT_OFFERED.has(s)))));
    take('title', (v) => {
      f.title = v;
      f.titleOn = v !== '';
    });
    take('key', (v) => (f.key = v === 'true'));
    take('agent-key', (v) => (f.agentKey = v === 'true'));
    take('loop', (v) => {
      if (v === 'true') f.ends = 'loop';
      else if (f.ends === 'loop') f.ends = 'tail';
    });
    take('start-position', () => undefined);
    take('start-date', (v) => {
      f.startDate = v;
      if (v) f.range = 'dates';
    });
    take('stop-date', (v) => {
      f.stopDate = v;
      if (v) f.range = 'dates';
    });
    if (values['record-tail-seconds'] === '-1' && f.ends !== 'loop') f.ends = 'until-left';
    if (supplied) {
      f.extra = { 'record-ui-scale': f.extra['record-ui-scale'] ?? 'auto', 'record-format': f.extra['record-format'] ?? 'mp4' };
      for (const [k, v] of Object.entries(values)) if (!known.has(k) && !k.startsWith('record-')) f.extra[k] = v;
    }
    this.exact = null;
    this.showConfig();
    this.schedulePlan(0);
  }

  /** A recipe `.json` dropped on the dialog or chosen with the button (US-15). */
  private async importRecipe(file: File): Promise<void> {
    let recipe: Record<string, unknown>;
    try {
      if (file.size > 1 << 20) throw new Error('too large');
      recipe = JSON.parse(await file.text()) as Record<string, unknown>;
      if (recipe['v'] !== 1 || typeof recipe['options'] !== 'object' || typeof recipe['record'] !== 'object') throw new Error('not a recipe');
    } catch {
      this.recipeNote = `${file.name} is not a Gource recording recipe.`;
      this.showConfig();
      return;
    }
    const values: Record<string, string> = {};
    for (const part of [recipe['options'], recipe['record']]) {
      for (const [k, v] of Object.entries(part as Record<string, unknown>)) {
        if (typeof v === 'string') values[k] = v;
      }
    }
    // Rust validates every value (REC-105); an invalid one rejects the recipe.
    try {
      this.factory!.parseOptions(JSON.stringify(values));
    } catch (e) {
      this.recipeNote = `The recipe has an invalid value: ${parseWasmError(e, 'internal').text}`;
      this.showConfig();
      return;
    }
    const source = recipe['source'] as { path?: string } | undefined;
    const sameSource = !source?.path || source.path === this.host.source.path;
    this.recipeNote = `Loaded the recipe ${file.name}.` + (sameSource ? '' : ` It was made for ${source!.path}.`);
    if (recipe['build'] && recipe['build'] !== BUILD) this.recipeNote += ` Made with build ${String(recipe['build'])}; frames can differ.`;
    this.applyValues(values, true);
  }

  private fitTo(seconds: number): void {
    const span = this.plan?.estimate.span_days ?? 0;
    if (!(seconds > 0) || !(span > 0)) return;
    const timeScale = Number(this.form.timeScale) || 1;
    const spd = Math.max(0.001, (seconds * timeScale) / span);
    this.form.spd = String(Number(spd.toPrecision(3)));
    const input = this.refs['spd'] as HTMLInputElement | undefined;
    if (input) input.value = this.form.spd;
    this.supplied.delete('seconds-per-day');
    this.schedulePlan(0);
  }

  /** RPA-4: explicitly widen the recording copy, never the live view or an export. */
  private useEntireHistory(): void {
    this.form.range = 'all';
    this.form.startDate = '';
    this.form.stopDate = '';
    this.form.extra['stop-position'] = '';
    for (const key of ['start-date', 'stop-date', 'start-position', 'stop-position']) this.supplied.delete(key);
    this.confirmed = false;
    this.exact = null;
    this.planJson = '';
    this.showConfig();
    this.refs['range:all']?.focus();
    this.schedulePlan(0);
  }

  // --- plan, ladder, preview, dry run ---

  private schedulePlan(delay: number): void {
    window.clearTimeout(this.planTimer);
    this.planTimer = window.setTimeout(() => this.updatePlan(), delay);
  }

  /** Applies a pending (debounced) form change now, so actions never see a stale plan. */
  private flushPlan(): void {
    window.clearTimeout(this.planTimer);
    this.updatePlan();
  }

  private updatePlan(): void {
    if (!this.factory || this.screen !== 'config') return;
    const json = this.optionsJson();
    if (json === this.planJson && this.plan) return;
    this.planJson = json;
    this.dryRunToken++;
    if (this.exact && this.exact.json !== json) this.exact = null;
    try {
      this.plan = parsePlan(this.factory.plan(this.engine(), json));
      this.planError = null;
    } catch (e) {
      this.plan = null;
      this.planError = parseWasmError(e, 'record_invalid_option');
    }
    if (this.plan && this.exact) this.applyExact();
    this.renderPlan();
    if (this.plan && !this.plan.estimate.empty) {
      void this.probeLadder(this.plan);
      this.schedulePreview(PREVIEW_DELAY_MS);
    }
  }

  private applyExact(): void {
    const p = this.plan;
    const ex = this.exact?.result;
    if (!p || !ex) return;
    const fps = p.fps;
    p.estimate.exact = true;
    p.estimate.frames = ex.frames;
    p.estimate.duration_s = ex.frames / fps;
    p.estimate.size_upper_bytes = ex.size_upper_bytes;
  }

  private probeLadder(plan: RecordPlan): Promise<LadderChoice | null> {
    const key = JSON.stringify(plan.ladder);
    if (key === this.choiceKey && this.choicePromise) return this.choicePromise;
    this.choiceKey = key;
    this.choice = null;
    this.choiceDone = false;
    const promise = chooseCandidate(plan.ladder, (c) => this.factory!.isConfigSupported(c));
    this.choicePromise = promise;
    void promise.then((choice) => {
      if (this.choicePromise !== promise) return;
      this.choice = choice;
      this.choiceDone = true;
      this.renderPlan();
    });
    return promise;
  }

  private renderPlan(): void {
    if (this.screen !== 'config') return;
    const q = (sel: string) => this.body.querySelector<HTMLElement>(sel);
    for (const el of this.body.querySelectorAll<HTMLElement>('.rec-error')) el.textContent = '';
    const est = q('.rec-estimate');
    const enc = q('.rec-encoder');
    const warn = q('.rec-warnings');
    const start = this.foot.querySelector<HTMLButtonElement>('.rec-start');
    const dryBtn = q('.rec-dry-btn') as HTMLButtonElement | null;
    const still = this.foot.querySelector<HTMLButtonElement>('.rec-still');
    if (!est || !enc || !warn || !start) return;
    warn.replaceChildren(...kids());
    const p = this.plan;
    const emptyRange = p?.estimate.empty === true || (!p && this.planError?.code === 'record_empty_run');
    const widen = q('.rec-widen');
    if (widen) widen.hidden = !emptyRange;
    let blocked: string | null = null;
    if (!p) {
      const e = this.planError ?? { code: 'record_invalid_option', text: '' };
      const text = errorText(e);
      const target = emptyRange ? null : this.errorTarget(e.text);
      if (target) target.textContent = text;
      est.textContent = target ? 'Fix the highlighted field to see the estimate.' : text;
      enc.textContent = '';
      blocked = text;
    } else if (p.estimate.empty) {
      est.textContent = ERROR_TEXT['record_empty_run']!;
      enc.textContent = '';
      blocked = ERROR_TEXT['record_empty_run']!;
    } else {
      const e = p.estimate;
      const size = formatBytes(e.size_upper_bytes);
      est.textContent = e.exact && e.frames !== null
        ? `Exactly ${clock(e.frames / p.fps)} (${formatCount(e.frames)} frames) · file up to ${size}`
        : `About ${clock(e.about_s)}, up to ${clock(e.upper_s)} · file up to ${size}`;
      const c = this.choice;
      const probed = this.choiceDone && this.choiceKey === JSON.stringify(p.ladder);
      if (c && probed) {
        const cand = c.candidate;
        enc.textContent = `Encoder: ${profileName(cand.codec)}, ${hardwareWords(cand.hardware_acceleration)}, ${cand.width}x${cand.height}`;
        if (cand.note) enc.append(h('span.rec-note', {}, ` (${cand.note})`));
      } else if (probed) {
        enc.textContent = errorText({ code: 'record_no_encoder', text: '' });
        blocked = enc.textContent;
      } else {
        enc.textContent = 'Encoder: checking…';
      }
      for (const w of p.warnings) {
        const text = w === 'long_render' ? `The video can be up to ${Math.ceil(e.upper_s / 60)} minutes long; rendering it can take about as long.` : WARNINGS[w];
        if (text) warn.append(h('li', {}, text));
      }
      if (!this.canPickFile()) {
        const minutes = Math.floor((BLOB_CAP * 8) / (p.bitrate_bps * 2.5) / 60);
        warn.append(h('li', {}, `This browser builds the video in memory and downloads it at the end, up to ${formatBytes(BLOB_CAP)} (about ${minutes} minutes at this size).`));
      }
      if (dryBtn) {
        dryBtn.hidden = e.exact;
        dryBtn.classList.toggle('btn-primary', e.upper_s > 600);
      }
    }
    start.disabled = blocked !== null;
    start.title = blocked ?? '';
    if (still) still.disabled = blocked !== null;
    this.renderFacts();
  }

  /** The `.rec-error` slot of the field an error message names, if any. */
  private errorTarget(text: string): HTMLElement | null {
    const m = /'(record-[a-z-]+|[a-z-]+)'/.exec(text);
    const name = m?.[1];
    if (!name) return null;
    for (const el of this.body.querySelectorAll<HTMLElement>('.rec-error')) {
      if ((el.dataset.for ?? '').split(' ').includes(name)) return el;
    }
    return null;
  }

  private renderFacts(): void {
    const dl = this.body.querySelector('.rec-facts');
    const p = this.plan;
    if (!dl || !p) return;
    const c = this.choice?.candidate;
    const rows: Array<[string, string]> = [
      ['Codec', `${p.codec} (level ${(p.level_idc / 10).toFixed(1)})`],
      ['Frames', `${p.fps} fps; ${p.ticks_per_frame} tick${p.ticks_per_frame > 1 ? 's' : ''} of 1/${p.tick_hz} s per frame`],
      ['Bitrate', `${(p.bitrate_bps / 1e6).toFixed(1)} Mbps target, variable`],
      ['Keyframes', `every ${p.keyframe_every} frames`],
      ['Encoder', c ? `${hardwareWords(c.hardware_acceleration)}; candidate ${c.codec} at ${c.width}x${c.height}` : 'not chosen yet'],
      ['Estimated frames', p.estimate.exact && p.estimate.frames !== null ? formatCount(p.estimate.frames) : `about ${formatCount(p.estimate.about_frames)}, up to ${formatCount(p.estimate.upper_frames)}`],
      ['Container', this.canPickFile() ? `MP4, ${p.layout.seekable}` : `MP4, ${p.layout.memory} (in memory)`],
      ['Saved to', this.canPickFile() ? 'a file you choose on this computer' : 'memory, then downloaded'],
      ['Embedded in the file', this.form.metadata === 'embed' ? 'the recipe (options and plan; no time zone, no names)' : 'nothing'],
      ['Not recorded', 'camera gestures (zoom, drag, rotate), gravity keys'],
    ];
    dl.replaceChildren(...kids(...rows.flatMap(([k, v]) => [h('dt', {}, k), h('dd', {}, v)])));
  }

  private canPickFile(): boolean {
    if (hooksActive()) return window.__recordSink !== undefined && testConfig().sink !== 'blob';
    return typeof (window as unknown as { showSaveFilePicker?: unknown }).showSaveFilePicker === 'function';
  }

  /** Creates or reconfigures the fork for `json` (REC-89). */
  private ensureRecorder(json: string): RecorderLike {
    if (this.recorder && this.recorderJson === json) return this.recorder;
    if (this.recorder) {
      this.recorder.reconfigure(json);
    } else {
      this.recorder = this.factory!.createRecorder(this.engine(), json);
    }
    this.recorderJson = json;
    if (this.exact && this.exact.json === json) this.recorder.set_exact_frames(this.exact.result.frames);
    return this.recorder;
  }

  private schedulePreview(delay: number): void {
    window.clearTimeout(this.previewTimer);
    this.previewTimer = window.setTimeout(() => void this.renderPreview(), delay);
  }

  /** Sliced preview (REC-7, REC-114): `preview_for` at most SLICE_MS per macrotask. */
  private async renderPreview(): Promise<void> {
    if (this.screen !== 'config' || !this.plan || this.plan.estimate.empty) return;
    const token = ++this.previewToken;
    const state = this.body.querySelector('.rec-preview-state');
    const json = this.optionsJson();
    let rec: RecorderLike;
    try {
      rec = this.ensureRecorder(json);
    } catch (e) {
      this.planError = parseWasmError(e, 'internal');
      this.plan = null;
      this.renderPlan();
      return;
    }
    if (state) state.textContent = 'Rendering preview…';
    try {
      for (;;) {
        if (token !== this.previewToken || this.recorder !== rec) return;
        if (rec.preview_for(PREVIEW_SECONDS, SLICE_MS)) break;
        await nextTask();
      }
      const canvas = this.preview;
      if (canvas && token === this.previewToken) {
        const p = this.plan;
        const w = 320;
        const hh = p ? Math.max(1, Math.round((w * p.height) / p.width)) : 180;
        canvas.width = w;
        canvas.height = hh;
        canvas.getContext('2d')?.drawImage(rec.canvas, 0, 0, w, hh);
      }
      if (state) state.textContent = '';
    } catch (e) {
      if (state) state.textContent = `No preview: ${errorText(parseWasmError(e, 'internal'))}`;
    }
  }

  /** "Calculate exact length" (REC-12, REC-67): the sliced dry run. */
  private async dryRun(): Promise<void> {
    this.flushPlan();
    if (!this.plan || this.plan.estimate.empty) return;
    const token = ++this.dryRunToken;
    const json = this.optionsJson();
    const state = this.body.querySelector('.rec-dry-state');
    let planner;
    try {
      planner = this.factory!.createPlanner(this.engine(), json);
    } catch (e) {
      if (state) state.textContent = errorText(parseWasmError(e, 'internal'));
      return;
    }
    const cancel = h('button.btn.btn-small', { type: 'button', onclick: () => this.dryRunToken++ }, 'Stop');
    try {
      while (!planner.done()) {
        if (token !== this.dryRunToken || this.screen !== 'config') {
          if (state) state.textContent = '';
          return;
        }
        planner.run_for(SLICE_MS);
        if (state) state.replaceChildren(...kids(`Calculating… ${Math.floor(planner.progress() * 100)} % `, cancel));
        await nextTask();
      }
      const result = JSON.parse(planner.result_json()) as PlannerResult;
      this.exact = { json, result };
      if (this.recorder && this.recorderJson === json) this.recorder.set_exact_frames(result.frames);
      if (state) state.textContent = '';
      this.applyExact();
      this.renderPlan();
    } finally {
      planner.free();
    }
  }

  /** "Save frame as PNG" at the output size (US-13). */
  private saveStill(): void {
    this.flushPlan();
    const p = this.plan;
    if (!p) return;
    let rec: RecorderLike;
    try {
      rec = this.ensureRecorder(this.optionsJson());
      rec.redraw();
    } catch (e) {
      this.announce(errorText(parseWasmError(e, 'internal')));
      return;
    }
    const name = `${fileStem(this.stem(), p.width, p.height, p.fps, new Date())}.png`;
    // toBlob snapshots the drawing buffer now, in the same task as the draw.
    rec.canvas.toBlob((blob) => {
      if (!blob) return;
      download(blob, name);
      this.announce(`Saved ${name}.`);
    }, 'image/png');
  }

  private stem(): string {
    const name = this.form.name.trim();
    if (name) return name;
    return this.publicSource() ? this.host.source.path.replace(/\//g, '-') : 'gource';
  }

  // --- start ---

  /** Needs the blocking confirmation of REC-124. */
  private needsConfirm(): boolean {
    const p = this.plan;
    if (!p) return false;
    return p.estimate.size_upper_bytes > CONFIRM_BYTES || p.estimate.upper_s > CONFIRM_SECONDS || this.supplied.size > 0;
  }

  /** Click handler of Record… and of the confirmation's Start: the picker must come before any await. */
  private recordClicked(): void {
    if (this.screen === 'config') this.flushPlan();
    const p = this.plan;
    if (!p || p.estimate.empty || this.recording) return;
    if (this.needsConfirm() && !this.confirmed) {
      this.showConfirm();
      return;
    }
    const name = `${fileStem(this.stem(), this.choice?.candidate.width ?? p.width, this.choice?.candidate.height ?? p.height, p.fps, new Date())}.mp4`;
    let picked: Promise<FileHandleLike> | null = null;
    const test = testConfig();
    const picker = hooksActive() ? window.__recordSink : (window as unknown as { showSaveFilePicker?: (o: unknown) => Promise<FileHandleLike> }).showSaveFilePicker;
    if (picker && test.sink !== 'blob') {
      try {
        picked = picker.call(window, { suggestedName: name, types: [{ description: 'MP4 video', accept: { 'video/mp4': ['.mp4'] } }] });
      } catch (e) {
        picked = Promise.reject(e);
      }
    }
    void this.start(name, picked);
  }

  private showConfirm(): void {
    const p = this.plan!;
    this.screen = 'confirm';
    this.titleEl.textContent = 'Confirm recording';
    const o = this.options();
    const rows: Array<[string, string]> = [
      ['Size', `${p.width}x${p.height}`],
      ['Frame rate', `${p.fps} fps`],
      ['Quality', `${o['record-quality']}${o['record-bitrate'] !== '0' ? `, ${o['record-bitrate']} Mbps` : ''}`],
      ['Length', `up to ${clock(p.estimate.upper_s)}`],
      ['File', `up to ${formatBytes(p.estimate.size_upper_bytes)}`],
    ];
    if (o['loop'] === 'true') rows.push(['Loop', `yes, at most ${o['record-max-seconds']} s`]);
    const supplied = [...this.supplied].map((k) => `${k}=${o[k] ?? ''}`);
    this.body.replaceChildren(...kids(
      h('p', {}, 'Please check these values before the recording starts.'),
      h('dl.rec-facts', {}, ...rows.flatMap(([k, v]) => [h('dt', {}, k), h('dd', {}, v)])),
      supplied.length ? h('div', {}, h('p', {}, 'Set by the link or a recipe, not typed here:'), h('ul.rec-supplied-list', {}, ...supplied.map((s) => h('li', {}, h('code', {}, s))))) : null,
    ));
    const go = h('button.btn.btn-primary.rec-confirm', { type: 'button' }, recordIcon(), 'Start recording…');
    go.addEventListener('click', () => {
      this.confirmed = true;
      this.screen = 'config';
      this.recordClicked();
    });
    this.foot.replaceChildren(...kids(h('span.rec-spacer'), h('button.btn', { type: 'button', onclick: () => this.showConfig() }, 'Back'), go));
    go.focus();
  }

  private async start(name: string, picked: Promise<FileHandleLike> | null): Promise<void> {
    const startBtn = this.foot.querySelector<HTMLButtonElement>('.rec-start, .rec-confirm');
    if (startBtn) startBtn.disabled = true;
    let sink: Sink;
    try {
      sink = picked ? await FileSink.open(await picked) : new MemorySink(name, testConfig().blobCap ?? BLOB_CAP);
    } catch (e) {
      // A dismissed picker leaves the dialog open (REC-16).
      if (e instanceof DOMException && e.name === 'AbortError') {
        this.showConfig();
        return;
      }
      this.showConfig();
      const se = sinkError(e);
      this.announce(`The file could not be opened (${se.name}).`);
      const est = this.body.querySelector('.rec-estimate');
      if (est) est.textContent = `The file could not be opened (${se.name}).`;
      return;
    }
    try {
      await this.startRecording(sink);
    } catch (e) {
      await sink.abort();
      this.runError = parseWasmError(e, 'record_encoder_failed');
      this.showFailed(false);
      this.endRun();
    }
  }

  private async startRecording(sink: Sink): Promise<void> {
    const factory = this.factory!;
    let plan = this.plan!;
    const choice = this.choice ?? (await this.probeLadder(plan));
    if (!choice) throw new Error(`record_no_encoder: ${ERROR_TEXT['record_no_encoder']}`);
    const cand = choice.candidate;
    let json = this.optionsJson();
    if (cand.width !== plan.width || cand.height !== plan.height) {
      // The accepted ladder step is smaller (REC-52): record at that size.
      const o = this.options();
      o['record-viewport'] = `${cand.width}x${cand.height}`;
      o['record-ui-scale'] = String(cand.ui_scale);
      json = JSON.stringify(o);
    }
    const rec = this.ensureRecorder(json);
    plan = parsePlan(rec.plan_json());
    this.dryRunToken++;
    this.previewToken++;
    saveRemembered(this.recordOptions());

    const recipeBytes = this.form.metadata === 'embed' ? new TextEncoder().encode(rec.recipe_json('embedded', this.recipeContext(null))) : undefined;
    this.sink = sink;
    const recording = new Recording({
      factory,
      recorder: rec,
      plan,
      candidate: cand,
      sink,
      ...(recipeBytes ? { recipe: recipeBytes } : {}),
      comment: this.publicSource() ? `Made with gource-web: ${this.host.source.host}/${this.host.source.path}` : 'Made with gource-web',
      build: BUILD,
      test: testConfig(),
      onProgress: (p) => this.onProgress(p),
      ...(reducedMotion() ? {} : { onThumb: (c: HTMLCanvasElement) => this.thumbCtx?.drawImage(c, 0, 0, this.thumbCtx.canvas.width, this.thumbCtx.canvas.height) }),
      onState: () => undefined,
    });
    this.recording = recording;
    this.plan = plan;
    playerHost.track('recording_started', planProps(plan));
    this.showProgress(plan, cand.codec);
    this.cb.runningChanged(true);
    window.addEventListener('beforeunload', onBeforeUnload);
    await this.acquireWakeLock();
    document.addEventListener('visibilitychange', this.onVisibility);
    this.announce('Recording started.');
    const reason = await recording.run();
    await this.afterLoop(reason);
  }

  private async afterLoop(reason: string): Promise<void> {
    const recording = this.recording!;
    if (reason === 'discard') {
      await recording.discard();
      this.showCancelled();
      this.endRun();
      return;
    }
    if (reason === 'error') {
      this.runError = recording.error;
      this.showFailed(recording.canSave);
      return;
    }
    await this.finalize();
  }

  private async finalize(): Promise<void> {
    const recording = this.recording!;
    this.setProgressNote('Finishing the file…');
    try {
      this.result = await recording.finalize();
      this.showDone();
    } catch (e) {
      this.runError = (e as WasmError).code ? (e as WasmError) : parseWasmError(e, 'internal');
      this.showFailed(false);
    }
    this.endRun();
  }

  private endRun(): void {
    if (!this.recording) return;
    for (const resolve of this.runEndWaiters.splice(0)) resolve();
    this.lastRun = this.recording;
    this.recording.dispose();
    this.recording = null;
    window.removeEventListener('beforeunload', onBeforeUnload);
    document.removeEventListener('visibilitychange', this.onVisibility);
    void this.releaseWakeLock();
    this.releaseRecorder();
    this.cb.runningChanged(false);
  }

  // --- wake lock and tab title (REC-123, REC-24) ---

  private async acquireWakeLock(): Promise<void> {
    try {
      const wl = (navigator as unknown as { wakeLock?: { request(type: 'screen'): Promise<{ release(): Promise<void> }> } }).wakeLock;
      if (wl && document.visibilityState === 'visible') this.wakeLock = await wl.request('screen');
    } catch {
      this.wakeLock = null;
    }
  }

  private async releaseWakeLock(): Promise<void> {
    const lock = this.wakeLock;
    this.wakeLock = null;
    try {
      await lock?.release();
    } catch {
      // Already released.
    }
  }

  private readonly onVisibility = () => {
    if (document.visibilityState === 'visible' && this.recording && !this.wakeLock) void this.acquireWakeLock();
  };

  private markDoneTitle(): void {
    if (document.visibilityState !== 'hidden') return;
    this.titleBefore = document.title;
    document.title = `[Done] ${document.title}`;
    const reset = () => {
      if (document.visibilityState !== 'visible') return;
      if (document.title.startsWith('[Done] ')) document.title = this.titleBefore;
      document.removeEventListener('visibilitychange', reset);
      window.removeEventListener('focus', reset);
    };
    document.addEventListener('visibilitychange', reset);
    window.addEventListener('focus', reset);
  }

  // --- progress ---

  private showProgress(plan: RecordPlan, codec: string): void {
    this.screen = 'progress';
    this.titleEl.textContent = 'Recording';
    const thumb = h('canvas.rec-thumb', { width: 192, height: Math.max(1, Math.round((192 * plan.height) / plan.width)), 'aria-hidden': 'true' });
    this.thumbCtx = thumb.getContext('2d');
    const bar = h('div.rec-bar', { role: 'progressbar', 'aria-valuemin': 0, 'aria-valuemax': 100, 'aria-valuenow': 0, 'aria-label': 'Recording progress' }, h('span.rec-bar-fill'));
    const el = (cls: string) => h(`span.${cls}` as `span.${string}`, {});
    this.progressEls = {
      bar,
      fill: bar.firstElementChild as HTMLElement,
      percent: el('rec-percent'),
      date: el('rec-date'),
      frames: el('rec-frames'),
      elapsed: el('rec-elapsed'),
      eta: el('rec-eta'),
      speed: el('rec-speed'),
      bytes: el('rec-bytes'),
      note: h('p.rec-note.rec-progress-note', {}, 'Keep this tab open. You can keep using the viewer.'),
    };
    const e = this.progressEls;
    const where = this.host.source.kind === 'repo' && !this.host.source.private ? this.host.source.path : 'this run';
    this.lastAnnounced = -1;
    this.body.replaceChildren(...kids(
      h('p.rec-progress-head', {}, `${where} · ${plan.width}x${plan.height} · ${plan.fps} fps · ${profileName(codec)}`),
      h(
        'div.rec-progress',
        {},
        thumb,
        h(
          'div.rec-progress-main',
          {},
          h('div.rec-bar-row', {}, bar, e.percent!),
          h('p', {}, e.date!, ' · ', e.frames!),
          h('p', {}, e.elapsed!, ' · ', e.eta!, ' · ', e.speed!),
          h('p', {}, e.bytes!, ` · Encoder: ${hardwareWords(this.choice?.candidate.hardware_acceleration ?? 'no-preference')}`),
        ),
      ),
      e.note!,
    ));
    const cancel = h('button.btn.rec-cancel', { type: 'button', onclick: () => this.askCancel() }, 'Cancel');
    this.foot.replaceChildren(...kids(h('span.rec-spacer'), cancel));
    this.panel.focus();
  }

  private setProgressNote(text: string): void {
    const n = this.progressEls['note'];
    if (n) n.textContent = text;
  }

  private onProgress(p: Progress): void {
    const e = this.progressEls;
    const pct = Math.floor(p.progress * 100);
    e['fill']!.style.width = `${(p.progress * 100).toFixed(1)}%`;
    e['bar']!.setAttribute('aria-valuenow', String(pct));
    e['percent']!.textContent = `${pct} %`;
    e['date']!.textContent = p.date || '—';
    e['frames']!.textContent = `frame ${formatCount(p.frames)} of ${p.exact ? '' : 'about '}${formatCount(p.expected)}`;
    e['elapsed']!.textContent = `Elapsed ${formatElapsed(p.elapsedMs)}`;
    e['eta']!.textContent = p.etaMs === null ? 'estimating time left' : `about ${formatElapsed(p.etaMs)} left`;
    e['speed']!.textContent = `${p.speed.toFixed(1)}x real time`;
    e['bytes']!.textContent = `File so far ${formatBytes(p.bytes)}`;
    const step = Math.floor(pct / 10);
    if (step !== this.lastAnnounced && step > 0 && step < 10) {
      this.lastAnnounced = step;
      this.announce(`Recording ${step * 10} %.`);
    }
  }

  /** Cancel: Keep recording / Stop and save / Discard (REC-21). */
  private askCancel(): void {
    const r = this.recording;
    if (!r || this.screen !== 'progress' || this.body.querySelector('.rec-cancel-choice')) return;
    r.pause();
    const fps = this.plan?.fps ?? 60;
    const expected = this.plan?.estimate.frames ?? this.plan?.estimate.upper_frames ?? 0;
    const keep = h('button.btn.btn-primary', { type: 'button', onclick: () => this.keepRecording() }, 'Keep recording');
    const box = h(
      'div.rec-cancel-choice',
      { role: 'group', 'aria-labelledby': 'rec-cancel-title' },
      h('p#rec-cancel-title', {}, h('strong', {}, 'Stop recording? '), `${clock(r.frameCount / fps)} of ${this.plan?.estimate.exact ? '' : 'about '}${clock(expected / fps)} is recorded (${formatBytes(r.bytes)}).`),
      h(
        'div.rec-row',
        {},
        keep,
        h('button.btn', { type: 'button', onclick: () => this.stopAndSave() }, 'Stop and save'),
        h('button.btn.rec-discard', { type: 'button', onclick: () => this.discard() }, 'Discard'),
      ),
    );
    this.body.append(box);
    keep.focus();
  }

  private keepRecording(): void {
    this.body.querySelector('.rec-cancel-choice')?.remove();
    this.recording?.resume();
    this.panel.focus();
  }

  private stopAndSave(): void {
    const r = this.recording;
    if (!r) return;
    this.body.querySelector('.rec-cancel-choice')?.remove();
    if (r.state === 'failed' || this.screen === 'failed') {
      this.screen = 'progress';
      this.showProgress(this.plan!, this.choice?.candidate.codec ?? '');
      void this.finalize();
      return;
    }
    r.requestStop('save');
  }

  /**
   * "Stop and save" from outside the dialog (F2 PR-121: sign-out or a lost
   * session ends a private run): resolves once the run has ended, saved or
   * failed, so the engine can be dropped afterwards.
   */
  stopAndSaveNow(): Promise<void> {
    if (!this.recording) return Promise.resolve();
    const ended = new Promise<void>((resolve) => this.runEndWaiters.push(resolve));
    this.stopAndSave();
    return ended;
  }

  private discard(): void {
    const r = this.recording;
    if (!r) return;
    this.body.querySelector('.rec-cancel-choice')?.remove();
    if (this.screen === 'failed') {
      void r.discard().then(() => {
        this.showCancelled();
        this.endRun();
      });
      return;
    }
    r.requestStop('discard');
  }

  // --- end screens ---

  private recipeContext(result: Record<string, unknown> | null): string {
    const s = this.host.source;
    const head = s.head || this.cb.head();
    return JSON.stringify({
      build: BUILD,
      source: { ...s, head },
      confirmed: this.sourceConfirmed,
      time_zone: Intl.DateTimeFormat().resolvedOptions().timeZone,
      created: new Date().toISOString(),
      result: result ?? {},
    });
  }

  /** Both sidecar renderings, built at the end while the fork still exists (REC-72). */
  private sidecars: { plain: string; confirmed: string } | null = null;

  private buildSidecars(): void {
    const rec = this.recorder;
    const r = this.result;
    const c = this.choice?.candidate;
    if (!rec) return;
    const result = {
      codec: r?.codec ?? c?.codec ?? '',
      bitrate_bps: c?.bitrate ?? this.plan?.bitrate_bps ?? 0,
      encoder: { requested: this.form.hardware === 'auto' ? 'no-preference' : this.form.hardware, accepted: c?.codec ?? '' },
      container_frames: r?.frames ?? 0,
      bytes: r?.bytes ?? 0,
    };
    const was = this.sourceConfirmed;
    try {
      this.sourceConfirmed = false;
      const plain = rec.recipe_json('sidecar', this.recipeContext(result));
      this.sourceConfirmed = true;
      const confirmed = rec.recipe_json('sidecar', this.recipeContext(result));
      this.sidecars = { plain, confirmed };
    } catch {
      this.sidecars = null;
    } finally {
      this.sourceConfirmed = was;
    }
  }

  private showDone(): void {
    const r = this.result!;
    const p = this.plan!;
    const c = this.choice?.candidate;
    this.screen = 'done';
    playerHost.track('recording_finished', planProps(p));
    this.titleEl.textContent = 'Saved';
    const name = this.sink?.name ?? '';
    this.sourceConfirmed = false;
    this.buildSidecars();
    let stats: { atlas_evictions?: number } = {};
    try {
      stats = JSON.parse(this.recorder?.stats_json() ?? '{}') as typeof stats;
    } catch {
      stats = {};
    }
    const notes: Array<string> = [];
    if (r.reason === 'cap') notes.push(`Stopped at the ${formatBytes(BLOB_CAP)} in-memory limit (${clock(r.durationS)} of about ${clock((p.estimate.frames ?? p.estimate.upper_frames) / p.fps)} recorded); use Chrome or Edge to save to disk.`);
    if (r.reason === 'save') notes.push('Stopped early: the file holds the frames recorded so far.');
    if (r.limit) notes.push('The video reached its maximum length.');
    if ((stats.atlas_evictions ?? 0) > 0) notes.push('Some labels were drawn from a smaller font cache.');
    if (r.recipeDropped) notes.push('The recipe was too large to embed; use the recipe file.');
    const blob = r.blob ?? null;
    this.finishedBlob = blob;
    if (blob && !hooksActive()) {
      this.outputUrl = URL.createObjectURL(blob);
      download(blob, name, this.outputUrl);
    }
    const recipeBtn = h('button.btn', { type: 'button', onclick: () => this.downloadRecipe(name) }, 'Download recipe (.json)');
    const privateSource = this.host.source.kind !== 'repo' || this.host.source.private;
    const confirm = privateSource
      ? (() => {
          const input = h('input', { type: 'checkbox' });
          input.addEventListener('change', () => (this.sourceConfirmed = input.checked));
          return h('label.rec-check', {}, input, h('span', {}, `Include the source (${this.host.source.path}) and its commit in the recipe`));
        })()
      : null;
    this.body.replaceChildren(...kids(
      h('p.rec-saved-name', {}, h('strong', {}, name)),
      h('p', {}, `${clock(r.durationS)}, ${formatCount(r.frames)} frames, ${formatBytes(r.bytes)}, ${p.width}x${p.height}, ${p.fps} fps, ${profileName(r.codec)} (${r.codec})${c ? `, encoder ${hardwareWords(c.hardware_acceleration)}` : ''}`),
      h('p', {}, this.sink?.kind === 'file' ? 'Stored on this computer where you chose. Nothing was uploaded.' : 'Stored on this computer (downloaded). Nothing was uploaded.'),
      ...notes.map((n) => h('p.rec-note', {}, n)),
      confirm,
    ));
    const again = h('button.btn', { type: 'button', onclick: () => this.recordAgain() }, 'Record again');
    const open = blob && this.outputUrl ? h('a.btn', { href: this.outputUrl, download: name }, 'Download video') : null;
    this.foot.replaceChildren(...kids(open, recipeBtn, h('span.rec-spacer'), again, h('button.btn.btn-primary', { type: 'button', onclick: () => this.close() }, 'Close')));
    this.announce(`Recording finished: ${name}.`);
    this.markDoneTitle();
    (open ?? again).focus();
  }

  private downloadRecipe(videoName: string): void {
    const text = this.sidecarText();
    const stem = videoName.replace(/\.mp4$/, '') || 'gource';
    download(new Blob([text], { type: 'application/json' }), `${stem}.recipe.json`);
  }

  private sidecarText(): string {
    const s = this.sidecars;
    if (!s) return '{}';
    return this.sourceConfirmed ? s.confirmed : s.plain;
  }

  private recordAgain(): void {
    if (this.outputUrl) URL.revokeObjectURL(this.outputUrl);
    this.outputUrl = '';
    this.result = null;
    this.sidecars = null;
    this.confirmed = false;
    this.planJson = '';
    this.showConfig();
    this.panel.focus();
    this.schedulePlan(0);
  }

  private showFailed(canSave: boolean): void {
    const e = this.runError ?? { code: 'internal', text: 'Unknown error' };
    this.screen = 'failed';
    playerHost.track('recording_failed', { ...(this.plan ? planProps(this.plan) : {}), reason: e.code });
    this.titleEl.textContent = 'Recording stopped';
    this.body.replaceChildren(...kids(
      h('p.rec-error-box', { role: 'alert', 'data-code': e.code }, errorText(e)),
      canSave ? h('p', {}, 'The frames recorded so far can still be saved.') : null,
    ));
    if (canSave) {
      this.foot.replaceChildren(...kids(
        h('span.rec-spacer'),
        h('button.btn.rec-discard', { type: 'button', onclick: () => this.discard() }, 'Discard'),
        h('button.btn.btn-primary', { type: 'button', onclick: () => this.stopAndSave() }, 'Stop and save'),
      ));
    } else {
      this.foot.replaceChildren(...kids(h('span.rec-spacer'), h('button.btn.btn-primary', { type: 'button', onclick: () => this.close() }, 'Close')));
    }
    this.announce(`Recording stopped: ${errorText(e)}`);
    this.panel.focus();
  }

  private showCancelled(): void {
    this.screen = 'cancelled';
    this.titleEl.textContent = 'Recording discarded';
    const sink = this.sink;
    this.body.replaceChildren(...kids(
      h(
        'p',
        {},
        sink?.kind === 'file'
          ? `Discarded. An empty file named ${sink.name} was left where you saved it; delete it if you like.`
          : 'Discarded. Nothing was saved.',
      ),
    ));
    this.foot.replaceChildren(...kids(h('span.rec-spacer'), h('button.btn.btn-primary', { type: 'button', onclick: () => this.close() }, 'Close')));
    this.announce('Recording discarded.');
    this.panel.focus();
  }

  // --- test hook (REC-95) ---

  private installHook(): void {
    window.__record = {
      config: testConfig(),
      state: () => (this.el.hidden ? 'closed' : this.screen === 'failed' ? 'failed' : this.recording ? this.recording.state : this.screen === 'config' ? 'configuring' : this.screen),
      frames: () => this.recording?.frameCount ?? this.result?.frames ?? 0,
      cancel: (choice) => {
        if (choice === 'keep') this.keepRecording();
        else if (choice === 'save') this.stopAndSave();
        else this.discard();
      },
      sinkBytes: () => this.sink?.written ?? 0,
      plan: () => {
        if (this.screen === 'config') this.flushPlan();
        return this.plan;
      },
      frameHash: () => this.recorder?.frame_hash() ?? '',
      output: () => this.finishedBlob,
      captured: () => this.lastRun?.captured ?? new Map(),
      captureErrors: () => this.lastRun?.captureErrors ?? new Map(),
      hashes: () => this.lastRun?.hashes ?? [],
      timing: () => {
        const run = this.recording ?? this.lastRun;
        return { ...(run?.timing ?? {}), frames: run?.frameCount ?? 0 };
      },
      error: () => this.runError,
    };
  }

  private lastRun: Recording | null = null;
}

/** Children for `replaceChildren`, without the null placeholders of conditional parts. */
function kids(...items: Array<Node | string | null | undefined | false>): Array<Node | string> {
  return items.filter((x): x is Node | string => x !== null && x !== undefined && x !== false);
}

function closeIcon(): SVGSVGElement {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('class', 'icon icon-close');
  svg.setAttribute('aria-hidden', 'true');
  const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  path.setAttribute('d', 'M6 6l12 12M18 6L6 18');
  svg.append(path);
  return svg;
}

function onBeforeUnload(e: BeforeUnloadEvent): void {
  e.preventDefault();
  e.returnValue = '';
}

function reducedMotion(): boolean {
  return typeof window.matchMedia === 'function' && window.matchMedia('(prefers-reduced-motion: reduce)').matches;
}

/** One macrotask (MessageChannel), for sliced work. */
function nextTask(): Promise<void> {
  return new Promise((resolve) => {
    const c = new MessageChannel();
    c.port1.onmessage = () => resolve();
    c.port2.postMessage(0);
  });
}

function download(blob: Blob, name: string, existingUrl?: string): void {
  const url = existingUrl ?? URL.createObjectURL(blob);
  const a = h('a', { href: url, download: name, hidden: true });
  document.body.append(a);
  a.click();
  a.remove();
  if (!existingUrl) window.setTimeout(() => URL.revokeObjectURL(url), 1000);
}

/** The recording's shape for analytics (no names, no paths). */
function planProps(p: RecordPlan): { resolution: string; fps: number; duration_s?: number } {
  const out: { resolution: string; fps: number; duration_s?: number } = { resolution: `${p.width}x${p.height}`, fps: p.fps };
  if (p.max_frames) out.duration_s = Math.round(p.max_frames / p.fps);
  return out;
}
