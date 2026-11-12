// SPDX-License-Identifier: GPL-3.0-or-later

// While a handed-over log plays, settings stay in memory: each kind the player keeps, through its own module.

import { expect, test } from '@playwright/test';

import { loadStoredAi, storeAiOption } from '../../src/ai/stored';
import { loadPresets, loadRemembered, savePresets, saveRemembered } from '../../src/record/options';
import { keepSettingsInMemory } from '../../src/saved-settings';

const storage = new Map<string, string>();
(globalThis as unknown as { window: unknown }).window = {
  localStorage: {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => void storage.set(key, value),
  },
};

test.beforeEach(() => {
  storage.clear();
  keepSettingsInMemory(false);
});

test('in memory, nothing reaches storage, and the values read back until the log ends', () => {
  keepSettingsInMemory(true);
  storeAiOption('user-style', 'private-repo => hat=on');
  saveRemembered({ 'record-framerate': '30' });
  savePresets(new Map([['private preset', { 'record-framerate': '30' }]]));
  expect([...storage.keys()]).toEqual([]);
  expect(loadStoredAi().options['user-style']).toBe('private-repo => hat=on');
  expect(loadRemembered()['record-framerate']).toBe('30');
  expect([...loadPresets().keys()]).toEqual(['private preset']);

  keepSettingsInMemory(false);
  expect([...storage.keys()]).toEqual([]);
  expect(loadStoredAi().options['user-style']).toBeUndefined();
  expect([...loadPresets().keys()]).toEqual([]);
});

test('in memory, earlier stored settings are still read, and stay as they were', () => {
  saveRemembered({ 'record-framerate': '60' });
  const before = new Map(storage);
  keepSettingsInMemory(true);
  expect(loadRemembered()['record-framerate']).toBe('60');
  saveRemembered({ 'record-framerate': '30' });
  expect(loadRemembered()['record-framerate']).toBe('30');
  expect(storage).toEqual(before);
});

test('outside it, settings are stored', () => {
  storeAiOption('user-style', 'a => hat=on');
  saveRemembered({ 'record-framerate': '30' });
  savePresets(new Map([['mine', {}]]));
  expect([...storage.keys()].sort()).toEqual(['gource.ai-options.v1', 'gource.record.last', 'gource.record.presets']);
});
