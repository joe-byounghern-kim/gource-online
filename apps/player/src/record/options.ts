// SPDX-License-Identifier: GPL-3.0-or-later

// Recording options (recording.md section 5.2). They are not Gource
// options: they never reach `set_option` and are validated in Rust
// (`RecordOptions`, crates/gource-web/src/record/options.rs). This table
// mirrors `RECORD_OPTION_DEFS` there; tests/unit/record-options.test.ts
// checks they agree (names, defaults, choices, ranges).
//
// Also here: the page-URL form (`record=1` and `record-*` parameters,
// REC-3), the remembered values (REC-88) and named presets (US-15), all in
// try/catch so the dialog works without storage.

import { readSetting, writeSetting } from '../saved-settings';

export interface RecordOptionDef {
  name: string;
  default: string;
  choices: readonly string[];
  range: readonly [number, number] | null;
  special: readonly string[];
}

export const RECORD_OPTION_DEFS: readonly RecordOptionDef[] = [
  { name: 'record-framerate', default: '60', choices: ['24', '25', '30', '60'], range: null, special: [] },
  { name: 'record-viewport', default: '1920x1080', choices: [], range: null, special: [''] },
  { name: 'record-ui-scale', default: 'auto', choices: [], range: [0.25, 4], special: ['auto'] },
  { name: 'record-format', default: 'mp4', choices: ['mp4'], range: null, special: [] },
  { name: 'record-layout', default: 'auto', choices: ['auto', 'fragmented', 'faststart'], range: null, special: [] },
  { name: 'record-quality', default: 'high', choices: ['standard', 'high', 'max'], range: null, special: [] },
  { name: 'record-bitrate', default: '0', choices: [], range: [0.5, 100], special: ['0'] },
  { name: 'record-keyframe-seconds', default: '2', choices: [], range: [0.5, 10], special: [] },
  { name: 'record-hardware', default: 'auto', choices: ['auto', 'prefer-hardware', 'prefer-software'], range: null, special: [] },
  { name: 'record-tail-seconds', default: '5', choices: [], range: [0, 120], special: ['-1'] },
  { name: 'record-max-seconds', default: '0', choices: [], range: [1, 14400], special: ['0'] },
  { name: 'record-metadata', default: 'embed', choices: ['embed', 'none'], range: null, special: [] },
  { name: 'record-name', default: '', choices: [], range: null, special: [] },
];

export const RECORD_OPTIONS_BY_NAME: ReadonlyMap<string, RecordOptionDef> = new Map(RECORD_OPTION_DEFS.map((d) => [d.name, d]));

/** Size presets (REC-6); "match window" and "custom" are handled by the dialog. */
export const SIZE_PRESETS: ReadonlyArray<{ value: string; label: string; note?: string }> = [
  { value: '1280x720', label: '1280x720', note: '720p' },
  { value: '1920x1080', label: '1920x1080', note: '1080p' },
  { value: '2560x1440', label: '2560x1440', note: '1440p' },
  { value: '3840x2160', label: '3840x2160', note: '4K' },
  { value: '1080x1080', label: '1080x1080', note: 'square' },
  { value: '1080x1920', label: '1080x1920', note: 'vertical 9:16' },
];

export const FRAME_RATES = ['24', '25', '30', '60'] as const;

/** Largest Blob the in-memory sink builds (REC-63). */
export const BLOB_CAP = 1024 * 1024 * 1024;

/** Recording options read from the page URL at load time (REC-3). */
export interface RecordUrlParams {
  /** `record=1`: open the dialog once the history is complete. */
  open: boolean;
  /** `record-*` values as written in the URL. */
  options: Record<string, string>;
}

/** `record=1` and the `record-*` parameters of a query string. */
export function parseRecordParams(search: string): RecordUrlParams {
  const params = new URLSearchParams(search.startsWith('?') ? search.slice(1) : search);
  const options: Record<string, string> = {};
  for (const [key, value] of params) {
    if (RECORD_OPTIONS_BY_NAME.has(key)) options[key] = value;
  }
  const open = params.get('record') === '1';
  return { open, options };
}

// The parameters are read when this module is first evaluated, before the
// page router rewrites the URL (main.ts drops parameters it does not know).
let initial: RecordUrlParams | null = parseRecordParams(typeof window === 'undefined' ? '' : window.location.search);

/** The page's initial `record` parameters, once; later calls get nothing. */
export function takeInitialRecordParams(): RecordUrlParams | null {
  const p = initial;
  initial = null;
  return p && (p.open || Object.keys(p.options).length > 0) ? p : null;
}

// --- storage ---

const LAST_KEY = 'gource.record.last';
const PRESETS_KEY = 'gource.record.presets';

/** The values remembered between recordings: size, frame rate, quality, format, end (REC-88). */
export const REMEMBERED = [
  'record-viewport',
  'record-framerate',
  'record-quality',
  'record-format',
  'record-tail-seconds',
  'record-max-seconds',
] as const;

function readJson(key: string): unknown {
  try {
    const text = readSetting(key);
    return text ? (JSON.parse(text) as unknown) : null;
  } catch {
    return null;
  }
}

function writeJson(key: string, value: unknown): boolean {
  try {
    writeSetting(key, JSON.stringify(value));
    return true;
  } catch {
    return false;
  }
}

function stringRecord(v: unknown): Record<string, string> {
  const out: Record<string, string> = {};
  if (v && typeof v === 'object' && !Array.isArray(v)) {
    for (const [k, s] of Object.entries(v)) if (typeof s === 'string') out[k] = s;
  }
  return out;
}

export function loadRemembered(): Record<string, string> {
  const all = stringRecord(readJson(LAST_KEY));
  const out: Record<string, string> = {};
  for (const name of REMEMBERED) if (all[name] !== undefined) out[name] = all[name]!;
  return out;
}

export function saveRemembered(options: Record<string, string>): void {
  const out: Record<string, string> = {};
  for (const name of REMEMBERED) if (options[name] !== undefined) out[name] = options[name]!;
  writeJson(LAST_KEY, out);
}

/** Named presets: dialog values by name (title and range are never stored). */
export function loadPresets(): Map<string, Record<string, string>> {
  const raw = readJson(PRESETS_KEY);
  const presets = new Map<string, Record<string, string>>();
  if (raw && typeof raw === 'object' && !Array.isArray(raw)) {
    for (const [name, values] of Object.entries(raw)) presets.set(name, stringRecord(values));
  }
  return presets;
}

export function savePresets(presets: Map<string, Record<string, string>>): boolean {
  return writeJson(PRESETS_KEY, Object.fromEntries(presets));
}

// --- file names (REC-62) ---

/** Characters outside `[A-Za-z0-9._-]` become `-`; at most 80 characters. */
export function sanitiseStem(s: string): string {
  return s.trim().replace(/[^A-Za-z0-9._-]/g, '-').slice(0, 80);
}

/** `<stem>_<W>x<H>_<fps>fps_<YYYY-MM-DD>` (without extension). */
export function fileStem(stem: string, width: number, height: number, fps: number, date: Date): string {
  const pad = (n: number) => String(n).padStart(2, '0');
  const day = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
  return `${sanitiseStem(stem) || 'gource'}_${width}x${height}_${fps}fps_${day}`;
}
