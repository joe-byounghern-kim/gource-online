// SPDX-License-Identifier: GPL-3.0-or-later

// The Gource options the control panel exposes (PLAN section 4). Names and
// value syntax are Gource's (`gource_settings.cpp`): booleans are
// "true"/"false", colours RRGGBB, multi-value options newline-separated.
// Defaults follow `GourceSettings::setGourceDefaults()` as ported to
// `gource-core/src/settings.rs`.

export type SectionId = 'look' | 'playback' | 'visibility' | 'filters' | 'style';

export const SECTIONS: ReadonlyArray<{ id: SectionId; title: string; note?: string }> = [
  { id: 'look', title: 'Colours & bloom' },
  { id: 'playback', title: 'Playback & camera' },
  { id: 'visibility', title: 'Visibility & labels' },
  {
    id: 'filters',
    title: 'Filters',
    note: 'Re-simulates from the downloaded history. Regular expressions, one per line.',
  },
  // F1: built by apps/player/src/ai/section.ts, not by the generic panel builder.
  { id: 'style', title: 'Contributors & AI' },
];

interface BaseDef {
  name: string;
  label: string;
  section: SectionId;
  /** Default value in Gource syntax. */
  default: string;
  hint?: string;
}

export interface ColourDef extends BaseDef {
  kind: 'colour';
}
export interface RangeDef extends BaseDef {
  kind: 'range';
  min: number;
  max: number;
  step: number;
  /** Shown after the value, e.g. "s". */
  unit?: string;
  /** Logarithmic slider (wide ranges such as seconds-per-day). */
  log?: boolean;
}
export interface BoolDef extends BaseDef {
  kind: 'bool';
  /** The checkbox shows the opposite of the option (e.g. "Auto-rotate" for disable-auto-rotate). */
  invert?: boolean;
}
export interface SelectDef extends BaseDef {
  kind: 'select';
  choices: ReadonlyArray<{ value: string; label: string }>;
}
export interface TextDef extends BaseDef {
  kind: 'text';
  placeholder?: string;
}
/** Newline-separated multi-value option, edited as a textarea. */
export interface ListDef extends BaseDef {
  kind: 'list';
  placeholder?: string;
}

export type OptionDef = ColourDef | RangeDef | BoolDef | SelectDef | TextDef | ListDef;

/** Elements of the `hide` option, in Gource's order. */
export const HIDE_ELEMENTS = [
  'date',
  'users',
  'tree',
  'files',
  'usernames',
  'filenames',
  'dirnames',
  'progress',
  'bloom',
  'mouse',
  'root',
  'pull-requests',
  // F1: AI decorations (capes, shades, halo, glow, badges); last, as in the engine.
  'decorations',
] as const;
export type HideElement = (typeof HIDE_ELEMENTS)[number];

const colour = (name: string, label: string, def: string): ColourDef => ({
  name,
  label,
  section: 'look',
  kind: 'colour',
  default: def,
});

export const OPTION_DEFS: ReadonlyArray<OptionDef> = [
  // --- Colours & bloom ---
  colour('background-colour', 'Background', '1A1A1A'),
  colour('font-colour', 'Font', 'FFFFFF'),
  colour('dir-colour', 'Directories', 'FFFFFF'),
  colour('selection-colour', 'Selection', 'FFFF4D'),
  colour('highlight-colour', 'Highlight', 'FFFFFF'),
  colour('filename-colour', 'File names', 'FFFFFF'),
  { name: 'bloom-intensity', label: 'Bloom intensity', section: 'look', kind: 'range', min: 0.05, max: 3, step: 0.05, default: '0.75' },
  { name: 'bloom-multiplier', label: 'Bloom multiplier', section: 'look', kind: 'range', min: 0.1, max: 3, step: 0.05, default: '1' },
  { name: 'hash-seed', label: 'Colour seed', section: 'look', kind: 'text', default: '31', hint: 'Seed of the name/extension colour hash.' },

  // --- Playback & camera ---
  { name: 'seconds-per-day', label: 'Seconds per day', section: 'playback', kind: 'range', min: 0.05, max: 30, step: 0.05, unit: 's', log: true, default: '10' },
  { name: 'time-scale', label: 'Time scale', section: 'playback', kind: 'range', min: 0.1, max: 4, step: 0.1, unit: '×', default: '1' },
  { name: 'auto-skip-seconds', label: 'Auto-skip after', section: 'playback', kind: 'range', min: 0.5, max: 30, step: 0.5, unit: 's', default: '3' },
  { name: 'max-file-lag', label: 'Max file lag', section: 'playback', kind: 'range', min: 0.1, max: 30, step: 0.1, unit: 's', default: '5' },
  {
    name: 'camera-mode',
    label: 'Camera',
    section: 'playback',
    kind: 'select',
    default: 'overview',
    choices: [
      { value: 'overview', label: 'Overview' },
      { value: 'track', label: 'Track active users' },
    ],
  },
  { name: 'follow-user', label: 'Follow user', section: 'playback', kind: 'text', default: '', placeholder: 'User name' },
  { name: 'padding', label: 'Camera padding', section: 'playback', kind: 'range', min: 0.5, max: 1.95, step: 0.05, default: '1.1', hint: 'Gource accepts values below 2.' },
  { name: 'disable-auto-rotate', label: 'Auto-rotate', section: 'playback', kind: 'bool', invert: true, default: 'false' },
  { name: 'max-user-speed', label: 'User speed', section: 'playback', kind: 'range', min: 10, max: 2000, step: 10, default: '500' },
  { name: 'user-friction', label: 'User friction', section: 'playback', kind: 'range', min: 0.05, max: 2, step: 0.05, default: '1' },
  { name: 'elasticity', label: 'Elasticity', section: 'playback', kind: 'range', min: 0, max: 2, step: 0.05, default: '0' },
  { name: 'loop', label: 'Loop at the end', section: 'playback', kind: 'bool', default: 'false' },
  { name: 'no-time-travel', label: 'No time travel', section: 'playback', kind: 'bool', default: 'false', hint: 'Ignore commits that go back in time.' },

  // --- Visibility & labels ---
  { name: 'hide', label: 'Hide', section: 'visibility', kind: 'text', default: '' },
  { name: 'max-files', label: 'Displayed file limit', section: 'visibility', kind: 'text', default: '0', hint: 'Whole number. 0 means unlimited. Replays the downloaded history, without deleting acquired changes.' },
  { name: 'key', label: 'File extension key', section: 'visibility', kind: 'bool', default: 'false' },
  { name: 'hide-pull-requests', label: 'Show pull requests', section: 'visibility', kind: 'bool', invert: true, default: 'false', hint: 'Rebuilds this history without PR nodes or review-only contributors.' },
  { name: 'pr-flow-hud', label: 'Repository PR flow', section: 'visibility', kind: 'bool', default: 'false', hint: 'Open count and oldest replay-clock age, only with complete lifecycle coverage.' },
  { name: 'pull-request-style', label: 'Pull request style', section: 'visibility', kind: 'select', default: 'dim-highlight', choices: [{ value: 'dim-highlight', label: 'Dim open, highlight merged' }] },
  { name: 'pull-request-mode', label: 'Reviews and code links', section: 'visibility', kind: 'select', default: 'basic', choices: [{ value: 'basic', label: 'Basic (existing pull requests)' }, { value: 'full', label: 'Reviews and code links' }], hint: 'Hosted acquisition requires paid access. Imported and frozen histories are never reacquired.' },
  { name: 'title', label: 'Title', section: 'visibility', kind: 'text', default: '', placeholder: 'Shown bottom left' },
  { name: 'font-scale', label: 'Font scale', section: 'visibility', kind: 'range', min: 0.5, max: 4, step: 0.1, unit: '×', default: '1' },
  { name: 'font-size', label: 'Date & title size', section: 'visibility', kind: 'range', min: 6, max: 64, step: 1, default: '16' },
  { name: 'file-font-size', label: 'File name size', section: 'visibility', kind: 'range', min: 6, max: 64, step: 1, default: '14' },
  { name: 'dir-font-size', label: 'Directory name size', section: 'visibility', kind: 'range', min: 6, max: 64, step: 1, default: '14' },
  { name: 'user-font-size', label: 'User name size', section: 'visibility', kind: 'range', min: 6, max: 64, step: 1, default: '14' },
  { name: 'dir-name-depth', label: 'Dir name depth', section: 'visibility', kind: 'range', min: 0, max: 10, step: 1, default: '0', hint: '0 shows every level.' },
  { name: 'dir-name-position', label: 'Dir name position', section: 'visibility', kind: 'range', min: 0.1, max: 1, step: 0.05, default: '0.5' },
  { name: 'highlight-dirs', label: 'Highlight directories', section: 'visibility', kind: 'bool', default: 'false' },
  { name: 'highlight-users', label: 'Highlight all users', section: 'visibility', kind: 'bool', default: 'false' },
  { name: 'highlight-user', label: 'Highlight users', section: 'visibility', kind: 'list', default: '', placeholder: 'One user name per line' },
  { name: 'file-extensions', label: 'Extensions only', section: 'visibility', kind: 'bool', default: 'false', hint: 'Label files by extension.' },
  { name: 'user-scale', label: 'User scale', section: 'visibility', kind: 'range', min: 0.25, max: 5, step: 0.05, unit: '×', default: '1' },
  { name: 'file-idle-time', label: 'File idle time', section: 'visibility', kind: 'range', min: 0, max: 600, step: 5, unit: 's', default: '0', hint: '0 keeps files forever.' },
  { name: 'filename-time', label: 'File name time', section: 'visibility', kind: 'range', min: 2, max: 30, step: 0.5, unit: 's', default: '4' },

  // --- Filters (re-simulate) ---
  { name: 'file-filter', label: 'Hide files matching', section: 'filters', kind: 'list', default: '', placeholder: '\\.min\\.js$' },
  { name: 'file-show-filter', label: 'Show only files matching', section: 'filters', kind: 'list', default: '', placeholder: '^src/' },
  { name: 'user-filter', label: 'Hide users matching', section: 'filters', kind: 'list', default: '', placeholder: '\\[bot\\]$' },
  { name: 'user-show-filter', label: 'Show only users matching', section: 'filters', kind: 'list', default: '' },
  { name: 'start-date', label: 'Start date', section: 'filters', kind: 'text', default: '', placeholder: 'YYYY-MM-DD' },
  { name: 'stop-date', label: 'Stop date', section: 'filters', kind: 'text', default: '', placeholder: 'YYYY-MM-DD' },

  // --- Contributors & AI (F1; controls in apps/player/src/ai/section.ts) ---
  {
    name: 'ai-style',
    label: 'Built-in look',
    section: 'style',
    kind: 'select',
    default: 'default',
    choices: [
      { value: 'default', label: 'Cape + shades (default)' },
      { value: 'cape', label: 'Cape only' },
      { value: 'shades', label: 'Shades only' },
      { value: 'hat', label: 'Glowing hat' },
      { value: 'none', label: 'None (only my rules)' },
    ],
  },
  { name: 'agent-key', label: 'Agent key', section: 'style', kind: 'bool', default: 'false' },
  {
    name: 'ai-filter',
    label: 'AI commits',
    section: 'style',
    kind: 'select',
    default: 'all',
    choices: [
      { value: 'all', label: 'All' },
      { value: 'only-assisted', label: 'Only assisted' },
      { value: 'exclude-assisted', label: 'Exclude assisted' },
    ],
  },
  {
    name: 'decor-motion',
    label: 'Decoration motion',
    section: 'style',
    kind: 'select',
    default: 'full',
    choices: [
      { value: 'full', label: 'Full' },
      { value: 'off', label: 'Off' },
    ],
  },
  { name: 'user-style', label: 'My rules', section: 'style', kind: 'list', default: '', placeholder: 'ai:codex => hat=agent; glow=agent' },
];

export const OPTIONS_BY_NAME: ReadonlyMap<string, OptionDef> = new Map(OPTION_DEFS.map((d) => [d.name, d]));

/** Options whose change needs `restart()` (the simulation is rebuilt from the event log). */
export const RESTART_OPTIONS: ReadonlySet<string> = new Set([
  'file-filter',
  'file-show-filter',
  'user-filter',
  'user-show-filter',
  'start-date',
  'stop-date',
  'hash-seed',
  'ai-filter',
  'max-files',
]);

export function isMultiValue(name: string): boolean {
  return OPTIONS_BY_NAME.get(name)?.kind === 'list';
}

/** Normalises a value for comparison and the page URL. */
export function normaliseValue(name: string, value: string): string {
  const def = OPTIONS_BY_NAME.get(name);
  if (!def) return value;
  switch (def.kind) {
    case 'colour':
      return value.replace(/^#/, '').toUpperCase();
    case 'range': {
      const n = Number(value);
      return Number.isFinite(n) ? String(n) : value;
    }
    case 'list':
      return value
        .split('\n')
        .map((l) => l.trim())
        .filter((l) => l !== '')
        .join('\n');
    default:
      break;
  }
  if (name === 'hide') return normaliseHide(value.split(',')).join(',');
  return value.trim();
}

export function isDefault(name: string, value: string): boolean {
  const def = OPTIONS_BY_NAME.get(name);
  return def !== undefined && normaliseValue(name, value) === normaliseValue(name, def.default);
}

/** Known hide elements, deduplicated, in Gource's order. */
export function normaliseHide(items: Iterable<string>): HideElement[] {
  const set = new Set<string>();
  for (const item of items) set.add(item.trim().toLowerCase());
  return HIDE_ELEMENTS.filter((e) => set.has(e));
}

export function isColour(value: string): boolean {
  return /^[0-9A-Fa-f]{6}$/.test(value);
}
