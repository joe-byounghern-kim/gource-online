// SPDX-License-Identifier: GPL-3.0-or-later

// The settings the player keeps between visits. A log a page handed over can belong to a private repository, and
// settings can hold names from it (style rules), while signing out of that page clears only that page's own origin.
// So while such a log plays, changes stay in memory and end with it. Reads and writes throw as localStorage does.

let memory: Map<string, string> | null = null;

/** Keeps changes in memory (`true`), or goes back to the browser's storage and forgets them (`false`). */
export function keepSettingsInMemory(keep: boolean): void {
  memory = keep ? new Map() : null;
}

export function readSetting(key: string): string | null {
  return memory?.get(key) ?? window.localStorage.getItem(key);
}

export function writeSetting(key: string, value: string): void {
  if (memory) memory.set(key, value);
  else window.localStorage.setItem(key, value);
}
