// SPDX-License-Identifier: GPL-3.0-or-later

// Recording entry points (recording.md section 5.12): the wasm classes of
// the engine module with WebCodecs, or the mock backend for the mock suite.
// The mock module is loaded only when asked for.

import { loadWasm, type GourceApp } from '@gource-online/viewer';
import { testConfig } from './hooks';
import { webCodecsEncoder, webCodecsSupported } from './encoder';
import type { MuxerLike, PlannerLike, RecordFactory, RecorderLike } from './types';

/** The recording part of the generated bindings (packages/viewer/src/wasm/gource_web.d.ts). */
interface RecordWasm {
  GourceRecorder: { create(app: GourceApp, canvas: HTMLCanvasElement | undefined, options_json: string): RecorderLike };
  RecordPlanner: { create(app: GourceApp, options_json: string): PlannerLike };
  Mp4Muxer: new (config_json: string, avcc: Uint8Array, recipe?: Uint8Array) => MuxerLike;
  record_plan(app: GourceApp, options_json: string): string;
  parse_record_options(pairs_json: string): string;
}

let factory: Promise<RecordFactory> | null = null;

export function loadRecordFactory(mock: boolean): Promise<RecordFactory> {
  if (mock) return import('./mock').then((m) => m.mockRecordFactory);
  factory ??= loadWasm().then((m) => {
    const mod = m as unknown as RecordWasm;
    const f: RecordFactory = {
      kind: 'wasm',
      createRecorder: (app, json) => mod.GourceRecorder.create(app, undefined, json),
      createPlanner: (app, json) => mod.RecordPlanner.create(app, json),
      createMuxer: (config, avcc, recipe) => new mod.Mp4Muxer(config, avcc, recipe),
      plan: (app, json) => mod.record_plan(app, json),
      parseOptions: (json) => mod.parse_record_options(json),
      createEncoder: webCodecsEncoder,
      isConfigSupported: async (config) => {
        if (testConfig().rejectConfig?.(config)) return false;
        return webCodecsSupported(config);
      },
    };
    return f;
  });
  factory.catch(() => (factory = null));
  return factory;
}
