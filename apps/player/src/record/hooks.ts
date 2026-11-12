// SPDX-License-Identifier: GPL-3.0-or-later

// Test hooks (REC-95): `window.__record` and the `window.__recordSink`
// override of `showSaveFilePicker` exist only in builds made with
// VITE_E2E_HOOKS=1 (the e2e builds; never production or the Docker image),
// and then only on pages opened with `?fixed-dt` or `?mock-record=1`. While
// they are active a banner says nothing will be saved.

import type { FileHandleLike } from './sink';

/** Fault injection and sink overrides a test can set before pressing Record. */
export interface RecordTestConfig {
  /** `'blob'` forces the in-memory sink (headless Chrome has no picker). */
  sink?: 'blob';
  /** In-memory cap in bytes (REC-63 test: 1 MiB). */
  blobCap?: number;
  /** The (fake) encoder reports an error after this many frames. */
  encoderErrorAt?: number;
  /** Mock suite: the fake encoder takes this long per frame (keeps a run alive for UI tests). */
  encoderDelayMs?: number;
  /** The fake encoder swaps two output timestamps after this many frames. */
  reorderAt?: number;
  /** Reject ladder candidates whose codec, hardware or size matches. */
  rejectConfig?: (config: VideoEncoderConfig) => boolean;
  /** Frame indices whose recorder canvas is captured (ImageData) during the run. */
  captureFrames?: number[];
  /** Record per-frame pre-encode hashes. */
  hashes?: boolean;
}

export interface RecordTestHook {
  config: RecordTestConfig;
  state(): string;
  frames(): number;
  cancel(choice: 'keep' | 'save' | 'discard'): void;
  sinkBytes(): number;
  plan(): unknown;
  frameHash(): string;
  /** The finished file of the in-memory sink. */
  output(): Blob | null;
  /** Frames captured for `config.captureFrames`, by index. */
  captured(): Map<number, ImageData>;
  /** Capture-only check per captured frame: largest channel error of VideoFrame(canvas). */
  captureErrors(): Map<number, number>;
  hashes(): string[];
  /** Current or most recent completed run timing (ms). Current counters may be partial. */
  timing(): Record<string, number>;
  error(): { code: string; text: string } | null;
}

declare global {
  interface Window {
    __record?: RecordTestHook;
    /** Replaces `showSaveFilePicker` for tests (REC-92). */
    __recordSink?: (options: unknown) => Promise<FileHandleLike>;
    __recordTest?: RecordTestConfig;
  }
}

/** Build-time flag: false in production builds, so the hooks are dead code there. */
export const HOOKS_BUILT = import.meta.env.VITE_E2E_HOOKS === '1';

// Read once at load: the page router rewrites the URL without `mock-record`.
const initial = new URLSearchParams(HOOKS_BUILT && typeof window !== 'undefined' ? window.location.search : '');
const MOCK_RECORD = HOOKS_BUILT && initial.get('mock-record') === '1';
const ACTIVE = HOOKS_BUILT && (initial.has('fixed-dt') || MOCK_RECORD);

export function hooksActive(): boolean {
  return ACTIVE;
}

/** `?mock-record=1` in a test build: the mock recorder with the mock engine (REC-93). */
export function mockRecordEnabled(): boolean {
  return MOCK_RECORD;
}

export function testConfig(): RecordTestConfig {
  if (!hooksActive()) return {};
  window.__recordTest ??= {};
  return window.__recordTest;
}
