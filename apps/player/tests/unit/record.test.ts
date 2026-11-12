// SPDX-License-Identifier: GPL-3.0-or-later

// Pure parts of F3 recording: the option table (and its agreement with the
// Rust table when the wasm build exists, T-11), URL parameters, file names,
// the error helper and the in-memory sink's patch and splice logic.

import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

import { expect, test } from '@playwright/test';

import { parseWasmError } from '../../src/errors';
import { RECORD_OPTION_DEFS, fileStem, parseRecordParams, sanitiseStem } from '../../src/record/options';
import { MemorySink, decodePatches, sinkError } from '../../src/record/sink';

const WASM_JS = fileURLToPath(new URL('../../../../packages/viewer/src/wasm/gource_web.js', import.meta.url));
const WASM_BIN = fileURLToPath(new URL('../../../../packages/viewer/src/wasm/gource_web_bg.wasm', import.meta.url));

test('T-11: RECORD_OPTION_DEFS agrees with the Rust table (record_option_defs)', async () => {
  test.skip(!existsSync(WASM_JS), 'needs scripts/build-web.sh');
  const mod = (await import(WASM_JS)) as { initSync(o: { module: Uint8Array }): unknown; record_option_defs(): string };
  mod.initSync({ module: readFileSync(WASM_BIN) });
  const rust = JSON.parse(mod.record_option_defs()) as Array<{ name: string; default: string; choices: string[]; range: [number, number] | null; special: string[] }>;
  expect(RECORD_OPTION_DEFS.map((d) => ({ name: d.name, default: d.default, choices: [...d.choices], range: d.range ? [...d.range] : null, special: [...d.special] }))).toEqual(rust);
});

test('record URL parameters: record=1 and record-* only', () => {
  const p = parseRecordParams('?repo=github.com/a/b&record=1&record-framerate=30&record-viewport=&seconds-per-day=2&record-bogus=1');
  expect(p.open).toBe(true);
  expect(p.options).toEqual({ 'record-framerate': '30', 'record-viewport': '' });
  expect(parseRecordParams('?record=0').open).toBe(false);
});

test('REC-62 file names', () => {
  expect(sanitiseStem('owner/repo name')).toBe('owner-repo-name');
  expect(sanitiseStem('x'.repeat(100))).toHaveLength(80);
  expect(fileStem('acaudwell-Gource', 1920, 1080, 60, new Date(2026, 9, 2))).toBe('acaudwell-Gource_1920x1080_60fps_2026-10-02');
  expect(fileStem('', 1280, 720, 30, new Date(2026, 0, 5))).toBe('gource_1280x720_30fps_2026-01-05');
});

test('parseWasmError splits coded messages', () => {
  expect(parseWasmError(new Error('record_empty_run: The selected range has no commits.'))).toEqual({ code: 'record_empty_run', text: 'The selected range has no commits.' });
  expect(parseWasmError(new Error('style_bad_value: bad (line 2)'))).toEqual({ code: 'style_bad_value', text: 'bad (line 2)' });
  expect(parseWasmError('plain text', 'record_mux_failed')).toEqual({ code: 'record_mux_failed', text: 'plain text' });
  expect(parseWasmError(new Error("invalid value 'x' for 'hide': y"), 'internal')).toEqual({ code: 'internal', text: "invalid value 'x' for 'hide': y" });
});

test('REC-65: sink errors are classified by DOMException name', () => {
  expect(sinkError(new DOMException('x', 'QuotaExceededError')).code).toBe('record_disk_full');
  expect(sinkError(new DOMException('x', 'NotAllowedError'))).toMatchObject({ code: 'record_sink_failed', name: 'NotAllowedError' });
});

function wire(patches: Array<[number, number[]]>): Uint8Array {
  const parts: number[] = [];
  for (const [offset, bytes] of patches) {
    const head = new DataView(new ArrayBuffer(12));
    head.setUint32(0, offset % 2 ** 32, true);
    head.setUint32(4, Math.floor(offset / 2 ** 32), true);
    head.setUint32(8, bytes.length, true);
    parts.push(...new Uint8Array(head.buffer), ...bytes);
  }
  return new Uint8Array(parts);
}

test('patch wire format decodes, including 64-bit offsets', () => {
  const p = decodePatches(wire([[5, [1, 2]], [2 ** 33 + 7, [9]]]));
  expect(p.map((x) => [x.offset, [...x.bytes]])).toEqual([[5, [1, 2]], [2 ** 33 + 7, [9]]]);
});

async function blobBytes(sink: MemorySink, tail: number[], placement: { kind: 'append' | 'insert_at'; offset?: number; len?: number }, patches: Uint8Array = new Uint8Array(0)): Promise<number[]> {
  const r = await sink.finish(new Uint8Array(tail), placement, patches);
  return [...new Uint8Array(await r.blob!.arrayBuffer())];
}

test('memory sink: patches across parts and the faststart-deferred splice after ftyp', async () => {
  let sink = new MemorySink('a.mp4', 1 << 20);
  sink.write(new Uint8Array([0, 1, 2, 3]));
  sink.write(new Uint8Array([4, 5, 6, 7]));
  expect(await blobBytes(sink, [8], { kind: 'append' }, wire([[3, [30, 40]]]))).toEqual([0, 1, 2, 30, 40, 5, 6, 7, 8]);

  // Insert inside a part (offset 2) and at a part boundary (offset 4).
  sink = new MemorySink('b.mp4', 1 << 20);
  sink.write(new Uint8Array([0, 1, 2, 3]));
  sink.write(new Uint8Array([4, 5]));
  expect(await blobBytes(sink, [90, 91, 99], { kind: 'insert_at', offset: 2, len: 2 })).toEqual([0, 1, 90, 91, 2, 3, 4, 5, 99]);
  sink = new MemorySink('c.mp4', 1 << 20);
  sink.write(new Uint8Array([0, 1, 2, 3]));
  sink.write(new Uint8Array([4, 5]));
  expect(await blobBytes(sink, [90, 99], { kind: 'insert_at', offset: 4, len: 1 })).toEqual([0, 1, 2, 3, 90, 4, 5, 99]);
});

test('memory sink: the cap marks it full', () => {
  const sink = new MemorySink('a.mp4', 10);
  sink.write(new Uint8Array(6));
  expect(sink.full).toBe(false);
  sink.write(new Uint8Array(6));
  expect(sink.full).toBe(true);
  expect(sink.limit).toBe(10);
});
