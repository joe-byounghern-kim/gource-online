// SPDX-License-Identifier: GPL-3.0-or-later

// TypeScript view of the gource-web wasm-bindgen API (PLAN section 4).

/** Main-thread engine: simulation, frame building and WebGL2 rendering. */
export interface GourceApp {
  free(): void;

  // events
  /** New repository: drop events and entities. */
  begin_log(): void;
  /** An encoded batch from `LogParser.push`/`finish`, in order. */
  append_batch(batch: Uint8Array): void;
  /** The log is complete: the run becomes seekable. */
  finish_log(total_bytes: number): void;

  // loop
  /** Advance and render; called from requestAnimationFrame. In fixed-dt mode it only redraws. */
  frame(now_ms: number): void;
  /** Deterministic mode: seconds of simulation per tick, advanced only by `step`; 0 = real time. */
  set_fixed_dt(seconds: number): void;
  /** Runs exactly `ticks` simulation ticks of the fixed dt (Gource's exporter), then renders one frame. */
  step(ticks: number): void;
  resize(css_width: number, css_height: number, device_pixel_ratio: number): void;

  // input: CSS pixels relative to the canvas top-left
  pointer_move(x: number, y: number, buttons: number): void;
  pointer_down(x: number, y: number, button: number): void;
  pointer_up(x: number, y: number, button: number): void;
  pointer_leave(): void;
  wheel(delta_y: number): void;
  /** DOM `KeyboardEvent.key`; returns true when the engine handled it. */
  key(key: string, pressed: boolean, shift: boolean, ctrl: boolean): boolean;

  // settings: Gource option names and value syntax
  /** Returns "live" or "restart"; throws on invalid input. */
  set_option(name: string, value: string): string;
  get_option(name: string): string | undefined;
  /** Viewer state, not an option: the overview camera frames the dirs under `dir` (`/owner:repo/`); '' frames the whole tree. */
  set_camera_focus(dir: string): void;
  camera_focus(): string;
  /** Rebuild the simulation from the event log after "restart" options. */
  restart(): void;
  /** F1: validates `user-style` rules without applying them: `{"ok":true}` or `{"ok":false,"code","line","message"}`. */
  check_style(text: string): string;
  /** F1: JSON AI attribution numbers of the loaded log and the played commits (see ai/stats.ts). */
  ai_stats(): string;
  /** Frozen full PR hover details for the DOM title fallback. Older/mock engines may omit it. */
  pr_hover?(): string;

  // playback
  set_paused(paused: boolean): void;
  seek(fraction: number): void;
  date_at(fraction: number): string;
  /** JSON-encoded {@link EngineStatus}. */
  status(): string;
}

/** Worker-side incremental parser of the Gource custom log. */
export interface LogParser {
  free(): void;
  /** Feeds raw log bytes; returns an encoded batch (may be empty). */
  push(chunk: Uint8Array): Uint8Array;
  /** Flushes the last partial line and commit. */
  finish(): Uint8Array;
  commits(): number;
  bytes(): number;
  skipped_lines(): number;
  /** `custom`, `git` (the output of Gource's git log command) or `unknown` before a whole line arrived. */
  format(): string;
}

export interface EngineStatus {
  date: string;
  time: number;
  fraction: number;
  seekable: boolean;
  paused: boolean;
  finished: boolean;
  commits_loaded: number;
  commits_played: number;
  files: number;
  users: number;
  dirs: number;
  fps: number;
  frame_ms: number;
  sim_ms: number;
  build_ms: number;
  gpu_ms: number;
  draw_calls: number;
  vertices: number;
  // Additions of the wasm engine (absent in the mock).
  uploaded_bytes?: number;
  /** Seconds per tick in fixed-dt mode; 0 = real time. */
  fixed_dt?: number;
  /** Simulation ticks run so far. */
  ticks?: number;
  /** Drawable size in device pixels. */
  width?: number;
  height?: number;
  /** Size of the WebAssembly linear memory in bytes. */
  memory_bytes?: number;
}

export type OptionEffect = 'live' | 'restart';

/** What `scripts/build-web.sh` generates into `src/wasm/gource_web.js` (wasm-bindgen `--target web`). */
export interface GourceWasmModule {
  /** Instantiates the module; takes the `.wasm` URL (or `{ module_or_path }`). */
  default: (init?: unknown) => Promise<unknown>;
  GourceApp: new (canvas: HTMLCanvasElement) => GourceApp;
  LogParser: new () => LogParser;
}

/** Engine entry points the page uses, real or mock. */
export interface EngineFactory {
  readonly kind: 'wasm' | 'mock';
  createApp(canvas: HTMLCanvasElement): GourceApp;
}

export function parseStatus(json: string): EngineStatus | null {
  try {
    return JSON.parse(json) as EngineStatus;
  } catch {
    return null;
  }
}
