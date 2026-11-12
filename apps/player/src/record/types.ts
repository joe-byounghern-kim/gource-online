// SPDX-License-Identifier: GPL-3.0-or-later

// The recording API as the page sees it (recording.md section 5.15). The
// controller and the dialog depend only on these interfaces, so the mock
// suite (`?mock=1&mock-record=1`) runs them without wasm or WebCodecs
// (REC-93). The wasm classes are GourceRecorder, RecordPlanner and
// Mp4Muxer in crates/gource-web/src/record/wasm.rs; every error they throw
// is "<code>: <text>" (see ../errors.ts).

import type { GourceApp } from '@gource-online/viewer';

/** Validated snapshot settings, without an output filename or embedded metadata. */
export interface SnapshotRecordSettings {
  options: Record<string, string>;
  record: Record<string, string>;
  plan: { fps: number; width: number; height: number; ui_scale: number };
}

/** `step_frame()` flags. */
export const FLAG_LAST = 1;
export const FLAG_KEY = 2;
export const FLAG_IN_TAIL = 4;
export const FLAG_LIMIT = 8;
export const FLAG_HARD_CAP = 16;

/** A fork of the live engine on its own canvas, stepped frame by frame. */
export interface RecorderLike {
  readonly canvas: HTMLCanvasElement;
  /** New options: restarts the run, resizes the output, redraws. */
  reconfigure(options_json: string): void;
  plan_json(): string;
  set_exact_frames(frames: number): void;
  /** Sliced preview: true once the frame `sim_seconds` after the start is drawn. */
  preview_for(sim_seconds: number, ms: number): boolean;
  /** Draws the current state again, for a still read in the same task. */
  redraw(): void;
  begin_recording(): void;
  /** Ticks of the next frame, one draw; returns the FLAG_* bits. */
  step_frame(): number;
  frame_index(): number;
  frame_count(): number;
  pts_us(): number;
  duration_us(): number;
  progress(): number;
  frame_hash(): string;
  sim_date(): string;
  stats_json(): string;
  result_json(): string;
  recipe_json(kind: 'embedded' | 'sidecar', context_json: string): string;
  free(): void;
}

/** The exact dry run (REC-67). */
export interface PlannerLike {
  /** Runs about `ms` of work; returns the ticks run, 0 once done. */
  run_for(ms: number): number;
  done(): boolean;
  progress(): number;
  result_json(): string;
  free(): void;
}

/** gource-mux behind wasm-bindgen. */
export interface MuxerLike {
  push(data: Uint8Array, pts_us: number, duration_us: number, is_key: boolean): number;
  pending(): number;
  take_output(dest: Uint8Array): number;
  finish(): number;
  take_patches(): Uint8Array;
  placement_json(): string;
  moov_at_end(): boolean;
  recipe_dropped(): boolean;
  bytes_written(): number;
  frames(): number;
  duration_us(): number;
  codec(): string;
  free(): void;
}

/** The part of `VideoEncoder` the controller uses (a fake one in the mock suite). */
export interface EncoderLike {
  readonly encodeQueueSize: number;
  readonly state: CodecState;
  configure(config: VideoEncoderConfig): void;
  encode(frame: VideoFrame, options?: VideoEncoderEncodeOptions): void;
  flush(): Promise<void>;
  close(): void;
  addEventListener(type: 'dequeue', listener: () => void, options?: AddEventListenerOptions): void;
  removeEventListener(type: 'dequeue', listener: () => void): void;
}

export interface EncoderCallbacks {
  output(chunk: EncodedVideoChunk, meta?: EncodedVideoChunkMetadata): void;
  error(e: DOMException): void;
}

/** Recording entry points, real (wasm + WebCodecs) or mock. */
export interface RecordFactory {
  readonly kind: 'wasm' | 'mock';
  createRecorder(app: GourceApp, options_json: string): RecorderLike;
  createPlanner(app: GourceApp, options_json: string): PlannerLike;
  createMuxer(config_json: string, avcc: Uint8Array, recipe?: Uint8Array): MuxerLike;
  /** `record_plan`: the plan without forking; an empty range is reported, not thrown. */
  plan(app: GourceApp, options_json: string): string;
  /** `parse_record_options`: validated, normalised `{name: value}`. */
  parseOptions(pairs_json: string): string;
  createEncoder(callbacks: EncoderCallbacks): EncoderLike;
  isConfigSupported(config: VideoEncoderConfig): Promise<boolean>;
}

/** One rung of the encoder support ladder (REC-52). */
export interface LadderCandidate {
  codec: string;
  profile: string;
  level_idc: number;
  width: number;
  height: number;
  bitrate: number;
  framerate: number;
  hardware_acceleration: HardwareAcceleration;
  avc_format: 'avc';
  bitrate_mode: VideoEncoderBitrateMode;
  latency_mode: LatencyMode;
  ui_scale: number;
  note: string | null;
}

/** `plan_json()` / `record_plan()` (crates/gource-web/src/record/setup.rs). */
export interface RecordPlan {
  fps: number;
  ticks_first: number;
  ticks_per_frame: number;
  tick_hz: number;
  fixed_dt: number;
  width: number;
  height: number;
  ui_scale: number;
  match_window: boolean;
  level_idc: number;
  codec: string;
  bitrate_bps: number;
  keyframe_every: number;
  max_frames: number | null;
  tail_s: number;
  estimate: {
    about_s: number;
    upper_s: number;
    exact: boolean;
    frames: number | null;
    duration_s: number | null;
    about_frames: number;
    upper_frames: number;
    size_upper_bytes: number;
    empty: boolean;
    commits: number;
    span_days: number;
    clipped: boolean;
  };
  layout: {
    seekable: 'fragmented' | 'faststart';
    memory: 'fragmented' | 'faststart-deferred';
    max_samples: number | null;
    max_sync_samples: number | null;
  };
  ladder: LadderCandidate[];
  warnings: string[];
  limits: { min_side: number; max_side: number; max_macroblocks: number };
}

/** `RecordPlanner.result_json()`. */
export interface PlannerResult {
  done: boolean;
  frames: number;
  duration_s: number;
  exact: boolean;
  size_upper_bytes: number;
  limit: string | null;
  commits: number;
  empty: boolean;
  ticks: number;
}

/** Where the recording's options and source came from, for the recipe (REC-70, REC-103). */
export interface RecordSource {
  kind: 'repo' | 'file' | 'url';
  /** `github.com`, `gitlab.com`, or '' for local sources. */
  host: string;
  /** Repository path (`owner/repo`), file name or log URL. */
  path: string;
  branch: string;
  since: string;
  until: string;
  subdir: string;
  head: string;
  /** F2: a private repository (or unknown). */
  private: boolean;
}

export function parsePlan(json: string): RecordPlan {
  return JSON.parse(json) as RecordPlan;
}
