// SPDX-License-Identifier: GPL-3.0-or-later

// F1: AI option defaults, start options from storage, the wasm error reader
// and the ai_stats() reader.

import { expect, test } from '@playwright/test';

import { parseAiStats } from '../../src/ai/stats';
import { aiStartOptions } from '../../src/ai/stored';
import { parseWasmError } from '../../src/errors';
import { HIDE_ELEMENTS, RESTART_OPTIONS, isDefault } from '@gource-online/viewer/options';

test('AI option defaults stay out of a link; decorations is the last hide element; ai-filter restarts', () => {
  for (const [name, value] of [
    ['ai-style', 'default'],
    ['ai-filter', 'all'],
    ['agent-key', 'false'],
    ['decor-motion', 'full'],
    ['user-style', ''],
  ]) {
    expect(isDefault(name!, value!)).toBe(true);
  }
  expect(HIDE_ELEMENTS.at(-1)).toBe('decorations');
  expect(RESTART_OPTIONS.has('ai-filter')).toBe(true);
});

test('start options: the URL wins over storage; storage fills the gaps; notices only for foreign values', () => {
  const stored = { options: { 'user-style': 'human => hat=on', 'agent-key': 'true', 'ai-filter': 'exclude-assisted' }, decorationsHidden: true };

  const own = aiStartOptions({}, stored, false);
  expect(own.options).toEqual({ 'user-style': 'human => hat=on', 'agent-key': 'true', 'ai-filter': 'exclude-assisted', hide: 'decorations' });
  expect(own.styleFromUrl).toBe(false);
  expect(own.filterFromUrl).toBe(false);

  // The viewer's own values echoed back by the URL after a reload: no notice.
  const echoed = aiStartOptions({ 'user-style': 'human => hat=on', 'ai-filter': 'exclude-assisted' }, stored, false);
  expect(echoed.styleFromUrl).toBe(false);
  expect(echoed.filterFromUrl).toBe(false);

  const link = aiStartOptions({ 'user-style': 'ai => glow=agent', 'ai-filter': 'only-assisted', hide: 'date' }, stored, false);
  expect(link.options['user-style']).toBe('ai => glow=agent');
  expect(link.options['ai-filter']).toBe('only-assisted');
  expect(link.options.hide).toBe('date');
  expect(link.options['agent-key']).toBe('true');
  expect(link.styleFromUrl).toBe(true);
  expect(link.filterFromUrl).toBe(true);

  // Reduced motion applies only when neither the URL nor storage says.
  expect(aiStartOptions({}, { options: {} }, true).options['decor-motion']).toBe('off');
  expect(aiStartOptions({ 'decor-motion': 'full' }, { options: {} }, true).options['decor-motion']).toBe('full');
  expect(aiStartOptions({}, { options: { 'decor-motion': 'full' } }, true).options['decor-motion']).toBe('full');
});

test('parseWasmError splits coded errors and leaves others whole', () => {
  expect(parseWasmError(new Error('style_bad_property: Unknown property "capes". (line 2)'))).toEqual({
    code: 'style_bad_property',
    text: 'Unknown property "capes". (line 2)',
  });
  expect(parseWasmError('record_empty_run: nothing to record')).toEqual({ code: 'record_empty_run', text: 'nothing to record' });
  expect(parseWasmError(new Error("invalid value 'x' for 'ai-filter': specify ai-filter"))).toEqual({
    code: '',
    text: "invalid value 'x' for 'ai-filter': specify ai-filter",
  });
  expect(parseWasmError(42)).toEqual({ code: '', text: '42' });
});

test('parseAiStats reads the engine shape and survives junk', () => {
  const json =
    '{"registry":1,"loaded":{"commits":10,"assisted":2,"agent":1,"bot":0,"sets":4,"agents":[{"id":"claude","name":"Claude",' +
    '"colour":"e69f00","commits":3,"as_author":1,"models":[{"model":"Claude Opus 5.5","commits":2}],"other_models":1}],' +
    '"more_agents":0},"played":{"commits":0,"assisted":0,"agent":0,"bot":0,"agents":[],"more_agents":0}}';
  const stats = parseAiStats(json)!;
  expect(stats.loaded.sets).toBe(4);
  expect(stats.loaded.agents[0]).toEqual({
    id: 'claude',
    name: 'Claude',
    colour: 'E69F00',
    commits: 3,
    as_author: 1,
    models: [{ model: 'Claude Opus 5.5', commits: 2 }],
    other_models: 1,
  });
  expect(parseAiStats('not json')).toBeNull();
  const odd = parseAiStats('{"loaded":{"agents":[{"id":"x","colour":"url(evil)"},{"name":"no id"}]}}')!;
  expect(odd.loaded.agents).toEqual([{ id: 'x', name: 'x', colour: 'FFFFFF', commits: 0, as_author: 0, models: [], other_models: 0 }]);
  expect(odd.loaded.sets).toBe(0);
});
