// SPDX-License-Identifier: GPL-3.0-or-later

// Mock recording backend for `?mock=1&mock-record=1` in test builds (REC-93):
// a Canvas2D recorder that draws a placeholder frame at the output size, a
// sliced planner, a byte-counting muxer that writes a small box-shaped
// container, and a fake encoder with the WebCodecs queue semantics
// (asynchronous output, `dequeue` events, `encodeQueueSize`). Faults are
// injected through the test configuration (hooks.ts).
//
// Length model: every loaded commit is half a second of video, then the
// tail; `record-max-seconds` caps it. A `start-date` in the year 2100 or
// later stands for a range with no commits (record_empty_run).

import type { GourceApp } from '@gource-online/viewer';
import { normaliseHide } from '@gource-online/viewer/options';
import { testConfig } from './hooks';
import { RECORD_OPTIONS_BY_NAME } from './options';
import {
  FLAG_IN_TAIL,
  FLAG_KEY,
  FLAG_LAST,
  FLAG_LIMIT,
  type EncoderCallbacks,
  type EncoderLike,
  type MuxerLike,
  type PlannerLike,
  type RecordFactory,
  type RecordPlan,
  type RecorderLike,
} from './types';

interface MockConfig {
  fps: number;
  width: number;
  height: number;
  uiScale: number;
  matchWindow: boolean;
  tail: number;
  max: number;
  quality: string;
  bodyFrames: number;
  frames: number;
  empty: boolean;
  options: Record<string, string>;
  overrides: Record<string, string>;
}

const BITRATE: Record<string, number> = { standard: 1, high: 1.5, max: 2.5 };

function fail(code: string, text: string): Error {
  return new Error(`${code}: ${text}`);
}

function parse(app: GourceApp, json: string): MockConfig {
  let raw: Record<string, unknown>;
  try {
    raw = JSON.parse(json || '{}') as Record<string, unknown>;
  } catch {
    throw fail('record_invalid_option', 'expected a JSON object');
  }
  const options: Record<string, string> = {};
  const overrides: Record<string, string> = {};
  for (const [k, v] of Object.entries(raw)) {
    const s = String(v);
    if (k.startsWith('record-')) {
      const def = RECORD_OPTIONS_BY_NAME.get(k);
      if (!def) throw fail('record_invalid_option', `unknown recording option '${k}'`);
      if (def.choices.length > 0 && s !== '' && !def.choices.includes(s)) {
        throw fail('record_invalid_option', `invalid value '${s}' for '${k}': expected one of ${def.choices.join(', ')}`);
      }
      if (def.range && s !== '' && !def.special.includes(s)) {
        const n = Number(s);
        if (!Number.isFinite(n) || n < def.range[0] || n > def.range[1]) {
          throw fail('record_invalid_option', `invalid value '${s}' for '${k}': expected ${def.range[0]} to ${def.range[1]}`);
        }
      }
      options[k] = s;
    } else {
      overrides[k] = s;
    }
  }
  const fps = Number(options['record-framerate'] || '60');
  const viewport = options['record-viewport'] ?? '1920x1080';
  let width = 1280;
  let height = 720;
  const matchWindow = viewport === '';
  if (!matchWindow) {
    const m = /^(\d+)x(\d+)$/i.exec(viewport);
    if (!m) throw fail('record_invalid_option', `invalid value '${viewport}' for 'record-viewport': expected WIDTHxHEIGHT, such as 1920x1080`);
    width = Number(m[1]);
    height = Number(m[2]);
    if (width % 2 || height % 2 || width < 128 || height < 128 || width > 4096 || height > 4096) {
      throw fail('record_size_unsupported', `${width}x${height}: width and height must be even, 128 to 4096`);
    }
  }
  const tail = Number(options['record-tail-seconds'] || '5');
  const max = Number(options['record-max-seconds'] || '0');
  const loop = (overrides['loop'] ?? app.get_option('loop')) === 'true';
  if (loop && max <= 0) throw fail('record_unbounded', 'A looping recording needs a maximum length.');
  let commits = 0;
  try {
    commits = (JSON.parse(app.status()) as { commits_loaded: number }).commits_loaded;
  } catch {
    commits = 0;
  }
  const start = overrides['start-date'] ?? '';
  const empty = commits === 0 || /^2[1-9]\d\d/.test(start);
  const bodyFrames = Math.max(1, Math.round(commits * 0.5 * fps));
  let frames = bodyFrames + Math.round((tail < 0 ? 12 : tail) * fps);
  if (loop || max > 0) frames = max > 0 ? Math.min(loop ? Infinity : frames, Math.ceil(max * fps)) : frames;
  const ui = options['record-ui-scale'];
  const uiScale = ui && ui !== 'auto' ? Number(ui) : Math.min(width, height) / 1080;
  return {
    fps,
    width,
    height,
    uiScale,
    matchWindow,
    tail,
    max,
    quality: options['record-quality'] || 'high',
    bodyFrames,
    frames,
    empty,
    options,
    overrides,
  };
}

function mockPlan(c: MockConfig, exact: number | null): RecordPlan {
  const k = Math.ceil(60 / c.fps);
  const bitrate = Math.round(8e6 * (BITRATE[c.quality] ?? 1.5) * (c.fps > 30 ? 1.5 : 1) * ((c.width * c.height) / 2073600));
  const upper = c.frames / c.fps;
  const duration = (exact ?? c.frames) / c.fps;
  const level = c.fps > 30 ? 0x2a : 0x28;
  const codec = `avc1.6400${level.toString(16)}`;
  const candidate = (w: number, h: number, hw: HardwareAcceleration, note: string | null) => ({
    codec,
    profile: 'high',
    level_idc: level,
    width: w,
    height: h,
    bitrate,
    framerate: c.fps,
    hardware_acceleration: hw,
    avc_format: 'avc' as const,
    bitrate_mode: 'variable' as const,
    latency_mode: 'quality' as const,
    ui_scale: c.uiScale * (w / c.width),
    note,
  });
  const half = (n: number) => Math.floor(n / 4) * 2;
  return {
    fps: c.fps,
    ticks_first: 1,
    ticks_per_frame: k,
    tick_hz: k * c.fps,
    fixed_dt: 1 / (k * c.fps),
    width: c.width,
    height: c.height,
    ui_scale: c.uiScale,
    match_window: c.matchWindow,
    level_idc: level,
    codec,
    bitrate_bps: bitrate,
    keyframe_every: c.fps * 2,
    max_frames: c.max > 0 ? Math.ceil(c.max * c.fps) : null,
    tail_s: c.tail < 0 ? 18 : c.tail,
    estimate: {
      about_s: Math.round(upper * 600) / 1000,
      upper_s: upper,
      exact: exact !== null,
      frames: exact,
      duration_s: exact !== null ? duration : null,
      about_frames: Math.round(c.frames * 0.6),
      upper_frames: c.frames,
      size_upper_bytes: Math.ceil((2.5 * bitrate * duration) / 8),
      empty: c.empty,
      commits: c.empty ? 0 : c.bodyFrames,
      span_days: 10,
      clipped: c.max > 0,
    },
    layout: { seekable: 'fragmented', memory: 'faststart-deferred', max_samples: null, max_sync_samples: null },
    ladder: [
      candidate(c.width, c.height, 'no-preference', null),
      candidate(c.width, c.height, 'prefer-software', `H.264 with automatic encoder choice is not available at ${c.width}x${c.height} here; using software encoding`),
      candidate(half(c.width * 1.5), half(c.height * 1.5), 'no-preference', `H.264 is not available at ${c.width}x${c.height} here; using ${half(c.width * 1.5)}x${half(c.height * 1.5)} instead`),
    ],
    warnings: upper > 600 ? ['crash_loses_file'] : [],
    limits: { min_side: 128, max_side: 4096, max_macroblocks: 36864 },
  };
}

export class MockRecorder implements RecorderLike {
  readonly canvas: HTMLCanvasElement;
  private readonly ctx: CanvasRenderingContext2D;
  private config: MockConfig;
  private exact: number | null = null;
  private index = -1;
  private started = false;
  private previewed = 0;

  constructor(
    private readonly app: GourceApp,
    json: string,
  ) {
    this.config = parse(app, json);
    if (this.config.empty) throw fail('record_empty_run', 'The selected range has no commits.');
    this.canvas = document.createElement('canvas');
    this.ctx = this.canvas.getContext('2d')!;
    this.resize();
  }

  private resize(): void {
    this.canvas.width = this.config.width;
    this.canvas.height = this.config.height;
    this.draw(0);
  }

  reconfigure(json: string): void {
    const next = parse(this.app, json);
    if (next.empty) throw fail('record_empty_run', 'The selected range has no commits.');
    this.config = next;
    this.exact = null;
    this.started = false;
    this.index = -1;
    this.resize();
  }

  plan_json(): string {
    return JSON.stringify(mockPlan(this.config, this.exact));
  }

  set_exact_frames(frames: number): void {
    if (Number.isFinite(frames) && frames >= 1) this.exact = Math.floor(frames);
  }

  preview_for(): boolean {
    this.previewed++;
    this.draw(Math.round(5 * this.config.fps));
    return true;
  }

  redraw(): void {
    this.draw(Math.max(0, this.index));
  }

  begin_recording(): void {
    this.started = true;
    this.index = -1;
  }

  private total(): number {
    return this.exact ?? this.config.frames;
  }

  step_frame(): number {
    if (!this.started || this.index + 1 >= this.total()) throw fail('record_not_started', 'call begin_recording first');
    this.index++;
    this.draw(this.index);
    const c = this.config;
    let flags = 0;
    if (this.index % (c.fps * 2) === 0) flags |= FLAG_KEY;
    if (this.index >= c.bodyFrames) flags |= FLAG_IN_TAIL;
    if (this.index + 1 >= this.total()) {
      flags |= FLAG_LAST;
      if (c.max > 0 && this.total() >= Math.ceil(c.max * c.fps)) flags |= FLAG_LIMIT;
    }
    return flags;
  }

  private draw(n: number): void {
    const { ctx, canvas } = this;
    const w = canvas.width;
    const h = canvas.height;
    ctx.fillStyle = '#1a1a1a';
    ctx.fillRect(0, 0, w, h);
    const t = n / this.config.fps;
    ctx.fillStyle = '#62d26f';
    ctx.beginPath();
    ctx.arc(w / 2 + Math.cos(t) * w * 0.3, h / 2 + Math.sin(t) * h * 0.3, Math.min(w, h) * 0.05, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillStyle = '#ffffff';
    ctx.font = `${Math.round(24 * this.config.uiScale)}px sans-serif`;
    const title = this.config.overrides['title'] ?? this.app.get_option('title') ?? '';
    if (title) ctx.fillText(title, 10, h - 14);
    ctx.fillText(`mock recorder · frame ${n}`, 10, 30 * this.config.uiScale);
  }

  frame_index(): number {
    return this.index;
  }
  frame_count(): number {
    return this.index + 1;
  }
  pts_us(): number {
    return Math.round((Math.max(0, this.index) * 1e6) / this.config.fps);
  }
  duration_us(): number {
    const n = Math.max(0, this.index);
    return Math.round(((n + 1) * 1e6) / this.config.fps) - Math.round((n * 1e6) / this.config.fps);
  }
  progress(): number {
    return Math.min(1, (this.index + 1) / this.total());
  }
  frame_hash(): string {
    let x = 0x811c9dc5 ^ this.index;
    for (const ch of JSON.stringify(this.config.overrides)) x = Math.imul(x ^ ch.charCodeAt(0), 16777619);
    return (x >>> 0).toString(16).padStart(16, '0');
  }
  sim_date(): string {
    return `2024-01-${String(1 + (Math.max(0, this.index) % 28)).padStart(2, '0')}`;
  }
  stats_json(): string {
    return JSON.stringify({ frames: this.index + 1, draw_calls: 10, atlas_evictions: 0, preview_empty: false, previews: this.previewed });
  }
  result_json(): string {
    return JSON.stringify({ frames: this.index + 1, duration_s: (this.index + 1) / this.config.fps, exact: this.exact !== null, limit: null, atlas_evictions: 0 });
  }
  recipe_json(kind: 'embedded' | 'sidecar', context_json: string): string {
    const ctx = JSON.parse(context_json) as Record<string, unknown>;
    const source = { ...(ctx['source'] as Record<string, unknown>) };
    const isPrivate = source['private'] === true || source['kind'] !== 'repo';
    if (isPrivate && ctx['confirmed'] !== true) {
      source['path'] = '';
      source['head'] = '';
    }
    const record: Record<string, string> = { ...this.config.options };
    record['record-viewport'] = `${this.config.width}x${this.config.height}`;
    record['record-ui-scale'] = String(this.config.uiScale);
    const recipe: Record<string, unknown> = {
      v: 1,
      app: 'gource-web',
      build: ctx['build'],
      engine: 'mock',
      source,
      options: { ...this.config.overrides, 'hash-seed': '31' },
      record,
      claims: [],
      plan: { fps: this.config.fps, width: this.config.width, height: this.config.height, ui_scale: this.config.uiScale },
      redacted: [],
    };
    if (kind === 'sidecar') {
      recipe['result'] = { ...(ctx['result'] as Record<string, unknown>), ...(JSON.parse(this.result_json()) as Record<string, unknown>) };
      recipe['time_zone'] = ctx['time_zone'];
      recipe['created'] = ctx['created'];
    }
    return JSON.stringify(recipe);
  }
  free(): void {
    this.canvas.width = 0;
    this.canvas.height = 0;
  }
}

class MockPlanner implements PlannerLike {
  private slices = 0;
  private readonly config: MockConfig;
  constructor(app: GourceApp, json: string) {
    this.config = parse(app, json);
    if (this.config.empty) throw fail('record_empty_run', 'The selected range has no commits.');
  }
  run_for(): number {
    if (this.done()) return 0;
    this.slices++;
    return 100;
  }
  done(): boolean {
    return this.slices >= 5;
  }
  progress(): number {
    return Math.min(1, this.slices / 5);
  }
  result_json(): string {
    const frames = Math.round(this.config.frames * 0.8);
    return JSON.stringify({
      done: this.done(),
      frames,
      duration_s: frames / this.config.fps,
      exact: true,
      size_upper_bytes: frames * 1000,
      limit: null,
      commits: this.config.bodyFrames,
      empty: false,
      ticks: frames,
    });
  }
  free(): void {}
}

/** Writes `ftyp`, one `mdat`-like record per chunk and an end box; checks timestamps like gource-mux. */
class MockMuxer implements MuxerLike {
  private out: Uint8Array[] = [];
  private pendingBytes = 0;
  private written = 0;
  private count = 0;
  private lastPts = -1;
  private durationUs = 0;
  private finished = false;

  constructor(
    config_json: string,
    avcc: Uint8Array,
    private readonly recipe?: Uint8Array,
  ) {
    JSON.parse(config_json);
    if (avcc.byteLength === 0) throw fail('record_mux_failed', 'empty avcC');
    this.emit(this.box('ftyp', new TextEncoder().encode('isommock')));
  }

  private box(type: string, body: Uint8Array): Uint8Array {
    const b = new Uint8Array(8 + body.byteLength);
    new DataView(b.buffer).setUint32(0, b.byteLength);
    b.set(new TextEncoder().encode(type), 4);
    b.set(body, 8);
    return b;
  }

  private emit(b: Uint8Array): void {
    this.out.push(b);
    this.pendingBytes += b.byteLength;
  }

  push(data: Uint8Array, pts_us: number, duration_us: number, is_key: boolean): number {
    if (this.finished) throw fail('record_mux_failed', 'finished');
    if (this.count === 0 && !is_key) throw fail('record_mux_failed', 'the first sample must be a keyframe');
    if (pts_us <= this.lastPts) throw fail('record_mux_reorder', `timestamp ${pts_us} after ${this.lastPts}`);
    this.lastPts = pts_us;
    this.count++;
    this.durationUs = pts_us + duration_us;
    this.emit(this.box('mdat', data.slice()));
    return this.pendingBytes;
  }

  pending(): number {
    return this.pendingBytes;
  }

  take_output(dest: Uint8Array): number {
    let n = 0;
    while (this.out.length > 0) {
      const b = this.out[0]!;
      if (n + b.byteLength > dest.byteLength) {
        const k = dest.byteLength - n;
        dest.set(b.subarray(0, k), n);
        this.out[0] = b.subarray(k);
        n += k;
        break;
      }
      dest.set(b, n);
      n += b.byteLength;
      this.out.shift();
    }
    this.pendingBytes -= n;
    this.written += n;
    return n;
  }

  finish(): number {
    this.finished = true;
    const meta = this.recipe ?? new Uint8Array(0);
    this.emit(this.box('moov', meta));
    return this.pendingBytes;
  }

  take_patches(): Uint8Array {
    return new Uint8Array(0);
  }
  placement_json(): string {
    return '{"kind":"append"}';
  }
  moov_at_end(): boolean {
    return true;
  }
  recipe_dropped(): boolean {
    return false;
  }
  bytes_written(): number {
    return this.written;
  }
  frames(): number {
    return this.count;
  }
  duration_us(): number {
    return this.durationUs;
  }
  codec(): string {
    return 'avc1.64002a';
  }
  free(): void {
    this.out = [];
  }
}

/** WebCodecs-shaped fake: outputs one chunk per macrotask, fires `dequeue`, can fail on request. */
export class FakeEncoder extends EventTarget implements EncoderLike {
  state: CodecState = 'unconfigured';
  private queue: Array<{ timestamp: number; duration: number; key: boolean }> = [];
  private readonly channel = new MessageChannel();
  private config: VideoEncoderConfig | null = null;
  private encoded = 0;
  private flushWaiters: Array<{ resolve: () => void; reject: (e: unknown) => void }> = [];
  private sentConfig = false;

  constructor(private readonly callbacks: EncoderCallbacks) {
    super();
    this.channel.port1.onmessage = () => this.process();
  }

  get encodeQueueSize(): number {
    return this.queue.length;
  }

  configure(config: VideoEncoderConfig): void {
    this.config = config;
    this.state = 'configured';
  }

  encode(frame: VideoFrame, options?: VideoEncoderEncodeOptions): void {
    if (this.state !== 'configured') throw new DOMException('not configured', 'InvalidStateError');
    this.queue.push({ timestamp: frame.timestamp, duration: frame.duration ?? 0, key: options?.keyFrame === true });
    const delay = testConfig().encoderDelayMs ?? 0;
    if (delay > 0) setTimeout(() => this.process(), delay * this.queue.length);
    else this.channel.port2.postMessage(0);
  }

  private process(): void {
    const item = this.queue.shift();
    if (!item || this.state !== 'configured') return;
    const test = testConfig();
    this.encoded++;
    if (test.encoderErrorAt !== undefined && this.encoded > test.encoderErrorAt) {
      this.state = 'closed';
      this.queue = [];
      this.callbacks.error(new DOMException('Injected encoder failure', 'EncodingError'));
      for (const w of this.flushWaiters.splice(0)) w.reject(new DOMException('closed', 'AbortError'));
      return;
    }
    let timestamp = item.timestamp;
    if (test.reorderAt !== undefined && this.encoded === test.reorderAt + 1) timestamp = Math.max(0, item.timestamp - 2 * item.duration);
    const data = new Uint8Array(96);
    data[0] = item.key ? 0x65 : 0x41;
    const chunk = new EncodedVideoChunk({ type: item.key ? 'key' : 'delta', timestamp, duration: item.duration, data });
    let meta: EncodedVideoChunkMetadata | undefined;
    if (!this.sentConfig) {
      this.sentConfig = true;
      meta = {
        decoderConfig: {
          codec: this.config?.codec ?? 'avc1.64002a',
          description: new Uint8Array([1, 100, 0, 42, 0xff, 0xe1]),
          colorSpace: { primaries: 'bt709', transfer: 'iec61966-2-1', matrix: 'bt709', fullRange: true },
        },
      };
    }
    this.callbacks.output(chunk, meta);
    this.dispatchEvent(new Event('dequeue'));
    if (this.queue.length === 0) for (const w of this.flushWaiters.splice(0)) w.resolve();
  }

  flush(): Promise<void> {
    if (this.state !== 'configured') return Promise.reject(new DOMException('closed', 'InvalidStateError'));
    if (this.queue.length === 0) return Promise.resolve();
    return new Promise((resolve, reject) => this.flushWaiters.push({ resolve, reject }));
  }

  close(): void {
    this.state = 'closed';
    this.queue = [];
    this.channel.port1.close();
    for (const w of this.flushWaiters.splice(0)) w.reject(new DOMException('closed', 'AbortError'));
  }

  override addEventListener(type: 'dequeue', listener: () => void, options?: AddEventListenerOptions): void {
    super.addEventListener(type, listener, options);
  }

  override removeEventListener(type: 'dequeue', listener: () => void): void {
    super.removeEventListener(type, listener);
  }
}

export const mockRecordFactory: RecordFactory = {
  kind: 'mock',
  createRecorder: (app, json) => new MockRecorder(app, json),
  createPlanner: (app, json) => new MockPlanner(app, json),
  createMuxer: (config, avcc, recipe) => new MockMuxer(config, avcc, recipe),
  plan: (app, json) => {
    const c = parse(app, json);
    return JSON.stringify(mockPlan(c, null));
  },
  parseOptions: (json) => {
    const raw = JSON.parse(json) as Record<string, unknown>;
    const out: Record<string, string> = {};
    for (const [k, v] of Object.entries(raw)) {
      const s = String(v);
      if (k === 'hide' && s !== '' && normaliseHide(s.split(',')).length !== s.split(',').length) {
        throw fail('record_invalid_option', `invalid value '${s}' for 'hide': unknown element`);
      }
      out[k] = s;
    }
    return JSON.stringify(out);
  },
  createEncoder: (callbacks) => new FakeEncoder(callbacks),
  isConfigSupported: async (config) => !(testConfig().rejectConfig?.(config) ?? false),
};
