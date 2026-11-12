// SPDX-License-Identifier: GPL-3.0-or-later

// @gource-online/viewer: Gource's renderer for a web page. Importing it
// defines <gource-viewer>.

export { GourceViewerElement, type GourceViewerEventMap } from './element';
export { EngineMissingError, loadEngine, loadWasm, wasmAvailable } from './engine/load';
export { parseStatus, type EngineFactory, type EngineStatus, type GourceApp, type GourceWasmModule, type LogParser, type OptionEffect } from './engine/types';
export { FONT_TIMEOUT_MS, loadGourceFont, webgl2Available } from './font';
export { LogLoader, type LogDone, type LogError, type LogLoaderEvents, type LogProgress } from './log-loader';
export type { LogFetch, LogSource } from './worker/protocol';
