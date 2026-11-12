// SPDX-License-Identifier: GPL-3.0-or-later

// One recording run (recording.md sections 5.1, 5.15; IMPLEMENTATION.md
// section 1): render the whole run start to end as fast as the machine
// allows.
//
// The loop is a plain async loop: no requestAnimationFrame and no timers
// between frames. Per frame, in one task: `step_frame()` (the ticks and one
// draw), `new VideoFrame(canvas)` straight from the drawing buffer (capture
// path A, M0 spike), `encode`, `close`. It awaits only when it has to: one
// `dequeue` event while the encoder queue holds 4 frames, the sink when more
// than SINK_HIGH_WATER bytes are queued, and a MessageChannel macrotask every
// YIELD_MS when neither applied (so input and painting continue). Encoded
// chunks go into the muxer from the encoder's output callback; its output
// is taken in batches of MUX_DRAIN_BYTES and queued on the sink without
// awaiting. Progress reaches the UI at most every PROGRESS_MS, with no
// layout read. One `flush()` at the end, then `finish()` and the patches.

import { parseWasmError, type WasmError } from '../errors';
import type { RecordTestConfig } from './hooks';
import type { Placement, Sink, SinkError } from './sink';
import { candidateConfig } from './encoder';
import {
  FLAG_KEY,
  FLAG_LAST,
  FLAG_LIMIT,
  type EncoderLike,
  type LadderCandidate,
  type MuxerLike,
  type RecordFactory,
  type RecordPlan,
  type RecorderLike,
} from './types';

/** Await one `dequeue` at this encoder queue depth (M0 spike: 4 is best). */
export const QUEUE_LIMIT = 4;
/** Yield a macrotask at least this often when nothing else made the loop wait. */
export const YIELD_MS = 50;
/** Take the muxer's output once this much is pending (batched sink writes). */
export const MUX_DRAIN_BYTES = 1 << 20;
/** Await the sink when more than this is queued ... */
export const SINK_HIGH_WATER = 8 << 20;
/** ... until it is down to this. */
export const SINK_LOW_WATER = 2 << 20;
export const PROGRESS_MS = 250;
export const THUMB_MS = 500;

export type RunState = 'starting' | 'running' | 'paused' | 'stopping' | 'done' | 'failed' | 'cancelled';

/** Why the loop stopped. */
export type StopReason = 'last' | 'save' | 'cap' | 'error' | 'discard';

export interface Progress {
  frames: number;
  /** Frames the plan expects (exact after a dry run, else the upper bound). */
  expected: number;
  exact: boolean;
  /** 0..1, monotone. */
  progress: number;
  date: string;
  elapsedMs: number;
  /** Smoothed; null until 3 s and 2 % (REC-18). */
  etaMs: number | null;
  /** Video seconds per wall second. */
  speed: number;
  bytes: number;
}

export interface RunResult {
  reason: StopReason;
  frames: number;
  durationS: number;
  bytes: number;
  blob?: Blob;
  limit: boolean;
  codec: string;
  recipeDropped: boolean;
}

export interface RecordingOptions {
  factory: RecordFactory;
  recorder: RecorderLike;
  plan: RecordPlan;
  candidate: LadderCandidate;
  sink: Sink;
  /** Embedded recipe (UTF-8 JSON) or undefined (`record-metadata=none`). */
  recipe?: Uint8Array;
  comment: string;
  build: string;
  test: RecordTestConfig;
  onProgress(p: Progress): void;
  /** Called right after a draw, in the same task, at most every THUMB_MS. */
  onThumb?(canvas: HTMLCanvasElement): void;
  onState(state: RunState): void;
}

/** A macrotask without timers (MessageChannel; not throttled like setTimeout in hidden tabs). */
const channel = typeof MessageChannel === 'undefined' ? null : new MessageChannel();
const yieldQueue: Array<() => void> = [];
if (channel) channel.port1.onmessage = () => yieldQueue.shift()?.();
function macrotask(): Promise<void> {
  return new Promise((resolve) => {
    yieldQueue.push(resolve);
    channel!.port2.postMessage(0);
  });
}

export class Recording {
  state: RunState = 'starting';
  error: WasmError | null = null;
  /** The error happened after frames were encoded, so "Stop and save" can keep them. */
  canSave = false;
  readonly timing = { step: 0, capture: 0, wait: 0, mux: 0, sink: 0, finish: 0, total: 0 };
  readonly hashes: string[] = [];
  readonly captured = new Map<number, ImageData>();
  /** Largest channel difference between the canvas and its VideoFrame, per captured frame. */
  readonly captureErrors = new Map<number, number>();

  private readonly o: RecordingOptions;
  private readonly rec: RecorderLike;
  private readonly canvas: HTMLCanvasElement;
  private readonly encoder: EncoderLike;
  private mux: MuxerLike | null = null;
  private chunkBuf = new Uint8Array(1 << 20);
  private stop: StopReason | null = null;
  private wake: (() => void) | null = null;
  private gate: Promise<void> | null = null;
  private openGate: (() => void) | null = null;
  private frames = 0;
  private chunks = 0;
  private startedAt = 0;
  private limitHit = false;
  private lastProgress = 0;
  private eta: number | null = null;
  private etaAt = 0;

  constructor(o: RecordingOptions) {
    this.o = o;
    this.rec = o.recorder;
    this.canvas = o.recorder.canvas;
    this.encoder = o.factory.createEncoder({
      output: (chunk, meta) => this.onChunk(chunk, meta),
      error: (e) => this.fail({ code: 'record_encoder_failed', text: `The video encoder reported an error: ${e.name}${e.message ? ` (${e.message})` : ''}.` }),
    });
  }

  get frameCount(): number {
    return this.frames;
  }

  /** Bytes in the file so far (written or queued). */
  get bytes(): number {
    return this.o.sink.written + (this.mux ? this.mux.pending() : 0);
  }

  private setState(s: RunState): void {
    this.state = s;
    this.o.onState(s);
  }

  private fail(e: WasmError): void {
    if (this.error) return;
    this.error = e;
    this.canSave = this.chunks > 0;
    this.stop ??= 'error';
    this.wakeLoop();
  }

  private wakeLoop(): void {
    const w = this.wake;
    this.wake = null;
    w?.();
  }

  // --- encoder output -> muxer -> sink ---

  private onChunk(chunk: EncodedVideoChunk, meta?: EncodedVideoChunkMetadata): void {
    if (this.error && !this.canSave) return;
    const t0 = performance.now();
    try {
      if (!this.mux) this.mux = this.createMux(meta);
      const n = chunk.byteLength;
      if (n > this.chunkBuf.byteLength) this.chunkBuf = new Uint8Array(Math.max(n, this.chunkBuf.byteLength * 2));
      chunk.copyTo(this.chunkBuf);
      const pending = this.mux.push(this.chunkBuf.subarray(0, n), chunk.timestamp, chunk.duration ?? 0, chunk.type === 'key');
      this.chunks++;
      if (pending >= MUX_DRAIN_BYTES) this.drainMux(pending);
    } catch (e) {
      this.fail(parseWasmError(e, 'record_mux_failed'));
    }
    this.timing.mux += performance.now() - t0;
  }

  private createMux(meta?: EncodedVideoChunkMetadata): MuxerLike {
    const dc = meta?.decoderConfig;
    const desc = dc?.description;
    if (!desc) throw new Error('record_mux_failed: The encoder did not describe the stream (no avcC).');
    const avcc = ArrayBuffer.isView(desc)
      ? new Uint8Array(desc.buffer.slice(desc.byteOffset, desc.byteOffset + desc.byteLength))
      : new Uint8Array(desc.slice(0));
    const plan = this.o.plan;
    const c = this.o.candidate;
    const seekable = this.o.sink.seekable;
    const layout = seekable ? plan.layout.seekable : plan.layout.memory;
    const cs = dc.colorSpace;
    const config = {
      width: c.width,
      height: c.height,
      fps: plan.fps,
      layout,
      max_samples: layout === 'faststart' ? plan.layout.max_samples : null,
      max_sync_samples: layout === 'faststart' ? plan.layout.max_sync_samples : null,
      // The stream's own colour tag (M0 spike: 1/13/1 full range from canvas frames).
      colour: cs ? { primaries: cs.primaries ?? null, transfer: cs.transfer ?? null, matrix: cs.matrix ?? null, full_range: cs.fullRange ?? null } : null,
      created: 0,
      comment: this.o.comment,
      build: this.o.build,
      timescale: 90000,
    };
    return this.o.factory.createMuxer(JSON.stringify(config), avcc, this.o.recipe);
  }

  private drainMux(n: number): void {
    const mux = this.mux!;
    const t0 = performance.now();
    const part = new Uint8Array(n);
    const got = mux.take_output(part);
    this.o.sink.write(got === n ? part : part.subarray(0, got));
    this.timing.sink += performance.now() - t0;
  }

  // --- control ---

  /** Pause the loop (the cancel choice is open). */
  pause(): void {
    if (this.gate || this.state !== 'running') return;
    this.gate = new Promise((resolve) => (this.openGate = resolve));
    this.setState('paused');
  }

  /** "Keep recording". */
  resume(): void {
    const open = this.openGate;
    this.gate = null;
    this.openGate = null;
    if (open) {
      this.setState('running');
      open();
    }
  }

  /** "Stop and save" (finalises the frames so far) or "Discard". */
  requestStop(reason: 'save' | 'discard'): void {
    this.stop ??= reason;
    this.wakeLoop();
    this.resume();
  }

  // --- the run ---

  /** Configures the encoder, restarts the fork, runs the loop; resolves when it stopped. */
  async run(): Promise<StopReason> {
    const config = candidateConfig(this.o.candidate);
    try {
      this.encoder.configure(config);
    } catch (e) {
      this.fail({ code: 'record_encoder_failed', text: `The video encoder rejected its configuration: ${e instanceof Error ? e.message : String(e)}` });
      this.setState('failed');
      return 'error';
    }
    this.rec.begin_recording();
    this.startedAt = performance.now();
    this.setState('running');
    const reason = await this.loop();
    this.timing.total = performance.now() - this.startedAt;
    return reason;
  }

  private async loop(): Promise<StopReason> {
    const rec = this.rec;
    const enc = this.encoder;
    const canvas = this.canvas;
    const sink = this.o.sink;
    const test = this.o.test;
    const capture = test.captureFrames && test.captureFrames.length > 0 ? new Set(test.captureFrames) : null;
    const hashes = test.hashes === true;
    const timing = this.timing;
    let yieldAt = performance.now() + YIELD_MS;
    let progressAt = 0;
    let thumbAt = 0;

    while (this.stop === null) {
      const t0 = performance.now();
      let flags: number;
      try {
        flags = rec.step_frame();
      } catch (e) {
        this.fail(parseWasmError(e, 'record_gl_failed'));
        break;
      }
      const t1 = performance.now();
      // Same task as the draw: the drawing buffer is not preserved.
      const frame = new VideoFrame(canvas, { timestamp: rec.pts_us(), duration: rec.duration_us() });
      try {
        enc.encode(frame, { keyFrame: (flags & FLAG_KEY) !== 0 });
      } catch (e) {
        frame.close();
        this.fail({ code: 'record_encoder_failed', text: `The video encoder reported an error: ${e instanceof Error ? e.message : String(e)}` });
        break;
      }
      frame.close();
      this.frames++;
      const t2 = performance.now();
      timing.step += t1 - t0;
      timing.capture += t2 - t1;

      if (capture?.has(this.frames - 1)) this.capture(this.frames - 1);
      if (hashes) this.hashes.push(rec.frame_hash());
      if (this.o.onThumb && t2 >= thumbAt) {
        this.o.onThumb(canvas);
        thumbAt = t2 + THUMB_MS;
      }
      if (t2 >= progressAt) {
        this.report(t2);
        progressAt = t2 + PROGRESS_MS;
      }
      if ((flags & FLAG_LAST) !== 0) {
        this.limitHit = (flags & FLAG_LIMIT) !== 0;
        this.stop ??= 'last';
        break;
      }

      if (enc.encodeQueueSize >= QUEUE_LIMIT) {
        await this.dequeue();
        yieldAt = performance.now() + YIELD_MS;
        timing.wait += performance.now() - t2;
      } else if (t2 >= yieldAt) {
        await macrotask();
        yieldAt = performance.now() + YIELD_MS;
        timing.wait += performance.now() - t2;
      }
      if (sink.pending > SINK_HIGH_WATER) {
        const t3 = performance.now();
        try {
          await sink.drain(SINK_LOW_WATER);
        } catch (e) {
          this.failSink(e as SinkError);
        }
        timing.sink += performance.now() - t3;
      }
      // The muxer's pending output counts too (it reaches the sink in batches).
      if (sink.limit !== Infinity && this.bytes >= sink.limit) this.stop ??= 'cap';
      if (sink.error) this.failSink(sink.error);
      if (this.gate) await this.gate;
    }
    this.report(performance.now());
    return this.stop ?? 'error';
  }

  private failSink(e: SinkError): void {
    this.fail({ code: e.code, text: e.code === 'record_disk_full' ? 'The disk is full; free some space and try again.' : `The file could not be written (${e.name}).` });
  }

  private dequeue(): Promise<void> {
    return new Promise((resolve) => {
      const done = () => {
        this.encoder.removeEventListener('dequeue', done);
        this.wake = null;
        resolve();
      };
      this.encoder.addEventListener('dequeue', done);
      this.wake = done;
    });
  }

  private capture(index: number): void {
    const c = document.createElement('canvas');
    c.width = this.canvas.width;
    c.height = this.canvas.height;
    const ctx = c.getContext('2d', { willReadFrequently: true })!;
    ctx.drawImage(this.canvas, 0, 0);
    const direct = ctx.getImageData(0, 0, c.width, c.height);
    this.captured.set(index, direct);
    // Capture-only check (REC-48): the VideoFrame of the canvas, drawn back, is the canvas.
    const frame = new VideoFrame(this.canvas, { timestamp: 0 });
    ctx.drawImage(frame, 0, 0);
    frame.close();
    const back = ctx.getImageData(0, 0, c.width, c.height).data;
    let worst = 0;
    for (let i = 0; i < back.length; i++) worst = Math.max(worst, Math.abs(back[i]! - direct.data[i]!));
    this.captureErrors.set(index, worst);
  }

  private report(now: number): void {
    const plan = this.o.plan;
    const exact = plan.estimate.frames;
    const expected = exact ?? plan.estimate.upper_frames;
    let progress = exact ? Math.min(1, this.frames / exact) : this.rec.progress();
    progress = Math.max(this.lastProgress, progress);
    this.lastProgress = progress;
    const elapsed = now - this.startedAt;
    if (elapsed > 3000 && progress > 0.02) {
      const raw = elapsed / progress - elapsed;
      if (this.eta === null) {
        this.eta = raw;
        this.etaAt = now;
      } else if (now - this.etaAt >= 500) {
        this.eta = this.eta * 0.9 + raw * 0.1;
        this.etaAt = now;
      } else {
        this.eta = Math.max(0, this.eta - (now - this.etaAt));
        this.etaAt = now;
      }
    }
    this.o.onProgress({
      frames: this.frames,
      expected,
      exact: exact !== null,
      progress,
      date: this.frames > 0 ? this.rec.sim_date() : '',
      elapsedMs: elapsed,
      etaMs: this.eta,
      speed: elapsed > 0 ? this.frames / plan.fps / (elapsed / 1000) : 0,
      bytes: this.bytes,
    });
  }

  /**
   * Ends the file: flush the encoder (unless it failed), mux the rest,
   * write the tail and the patches, close. Used for the natural end, "Stop
   * and save" and the in-memory cap.
   */
  async finalize(): Promise<RunResult> {
    const t0 = performance.now();
    this.setState('stopping');
    const reason = this.stop ?? 'save';
    if (this.encoder.state === 'configured') {
      try {
        await this.encoder.flush();
      } catch (e) {
        if (!this.error) this.fail({ code: 'record_encoder_failed', text: `The video encoder failed while finishing: ${e instanceof Error ? e.message : String(e)}` });
      }
    }
    this.closeEncoder();
    const mux = this.mux;
    if (!mux || mux.frames() === 0) {
      await this.o.sink.abort();
      this.error ??= { code: 'record_mux_failed', text: 'No frame was encoded, so there is nothing to save.' };
      this.setState('failed');
      throw this.error;
    }
    let tail: Uint8Array;
    let placement: Placement;
    let patches: Uint8Array;
    try {
      if (mux.pending() > 0) this.drainMux(mux.pending());
      const n = mux.finish();
      tail = new Uint8Array(n);
      const got = mux.take_output(tail);
      tail = tail.subarray(0, got);
      placement = JSON.parse(mux.placement_json()) as Placement;
      patches = mux.take_patches();
    } catch (e) {
      this.error = parseWasmError(e, 'record_mux_failed');
      await this.o.sink.tryClose();
      this.setState('failed');
      throw this.error;
    }
    let result;
    try {
      result = await this.o.sink.finish(tail, placement, patches);
    } catch (e) {
      const se = e as SinkError;
      const closed = await this.o.sink.tryClose();
      this.error = {
        code: se.code ?? 'record_sink_failed',
        text:
          (se.code === 'record_disk_full' ? 'The disk is full.' : `The file could not be written (${se.name ?? 'error'}).`) +
          (closed ? ' What was written so far was kept, without its final index.' : ' Nothing could be saved.'),
      };
      this.setState('failed');
      throw this.error;
    }
    const out: RunResult = {
      reason,
      frames: mux.frames(),
      durationS: mux.duration_us() / 1e6,
      bytes: result.bytes,
      limit: this.limitHit,
      codec: mux.codec(),
      recipeDropped: mux.recipe_dropped(),
    };
    if (result.blob) out.blob = result.blob;
    this.freeMux();
    this.timing.finish = performance.now() - t0;
    this.setState('done');
    return out;
  }

  /** "Discard": abort the sink; nothing is kept (REC-22). */
  async discard(): Promise<void> {
    this.closeEncoder();
    this.freeMux();
    await this.o.sink.abort();
    this.setState('cancelled');
  }

  private closeEncoder(): void {
    if (this.encoder.state !== 'closed') {
      try {
        this.encoder.close();
      } catch {
        // Already closed.
      }
    }
  }

  private freeMux(): void {
    try {
      this.mux?.free();
    } catch {
      // Already freed.
    }
    this.mux = null;
  }

  /** Releases everything on any exit path (REC-54). */
  dispose(): void {
    this.closeEncoder();
    this.freeMux();
    this.resume();
  }
}
