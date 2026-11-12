// SPDX-License-Identifier: GPL-3.0-or-later

// The viewer's own AI options (F1 US-15): the last rules and AI options are
// kept in localStorage and restored on any repository; options in the page
// URL win over them. Only changes the viewer makes are stored, so opening a
// link never overwrites the viewer's own rules. Storage may be unavailable
// (private windows, blocked site data): every access is wrapped and the page
// works without it.

import { normaliseHide, normaliseValue } from '@gource-online/viewer/options';
import { readSetting, writeSetting } from '../saved-settings';

/** Options stored and restored as a group (the `hide` element `decorations` is stored as a flag). */
export const AI_OPTION_NAMES: readonly string[] = ['user-style', 'ai-style', 'ai-filter', 'agent-key', 'decor-motion'];

const STORAGE_KEY = 'gource.ai-options.v1';

export interface StoredAi {
  options: Record<string, string>;
  /** Whether the viewer hid the decorations (`hide=decorations`). */
  decorationsHidden?: boolean;
}

export function loadStoredAi(): StoredAi {
  try {
    const raw = readSetting(STORAGE_KEY);
    if (!raw) return { options: {} };
    const parsed = JSON.parse(raw) as Partial<StoredAi>;
    const options: Record<string, string> = {};
    if (parsed.options && typeof parsed.options === 'object') {
      for (const name of AI_OPTION_NAMES) {
        const v = (parsed.options as Record<string, unknown>)[name];
        if (typeof v === 'string') options[name] = v;
      }
    }
    const stored: StoredAi = { options };
    if (typeof parsed.decorationsHidden === 'boolean') stored.decorationsHidden = parsed.decorationsHidden;
    return stored;
  } catch {
    return { options: {} };
  }
}

/** Records one option the viewer changed (AI options and the `decorations` hide element). */
export function storeAiOption(name: string, value: string): void {
  if (name !== 'hide' && !AI_OPTION_NAMES.includes(name)) return;
  const stored = loadStoredAi();
  if (name === 'hide') stored.decorationsHidden = normaliseHide(value.split(',')).includes('decorations');
  else stored.options[name] = normaliseValue(name, value);
  try {
    writeSetting(STORAGE_KEY, JSON.stringify(stored));
  } catch {
    // Storage is unavailable or full: the change still applies to this view.
  }
}

export interface AiStart {
  /** URL options with the stored AI options filled in where the URL gives none. */
  options: Record<string, string>;
  /** The URL applies rules the viewer did not store themselves (AI-74 notice). */
  styleFromUrl: boolean;
  /** The URL filters AI commits (AI-74 notice). */
  filterFromUrl: boolean;
}

/**
 * Options to start a view with: the URL's, then stored AI options for the
 * names the URL leaves out, then `decor-motion=off` once from the OS
 * preference when neither gives it (AI-77).
 */
export function aiStartOptions(url: Record<string, string>, stored: StoredAi, reducedMotion: boolean): AiStart {
  const options = { ...url };
  for (const name of AI_OPTION_NAMES) {
    const v = stored.options[name];
    if (options[name] === undefined && v !== undefined && v !== '') options[name] = v;
  }
  if (options.hide === undefined && stored.decorationsHidden) options.hide = 'decorations';
  if (options['decor-motion'] === undefined && reducedMotion) options['decor-motion'] = 'off';

  const same = (name: string) => normaliseValue(name, url[name] ?? '') === normaliseValue(name, stored.options[name] ?? '');
  const style = url['user-style'];
  const filter = url['ai-filter'];
  return {
    options,
    styleFromUrl: style !== undefined && normaliseValue('user-style', style) !== '' && !same('user-style'),
    filterFromUrl: filter !== undefined && normaliseValue('ai-filter', filter) !== 'all' && !same('ai-filter'),
  };
}

export function prefersReducedMotion(): boolean {
  try {
    return window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  } catch {
    return false;
  }
}
