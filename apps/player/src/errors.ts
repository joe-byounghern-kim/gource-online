// SPDX-License-Identifier: GPL-3.0-or-later

// Errors thrown by the wasm engine are `JsError`s. Coded errors read
// `<code>: <text>` (`style_bad_property: Unknown property "capes". ... (line 2)`,
// `record_empty_run: ...`; recording.md section 5.15, F1 AI-46); older option
// errors have no code. Read them here only, never by matching the message text
// elsewhere. Shared by the F1 rules editor and the F3 recorder.

export interface WasmError {
  /** Stable lower-case code such as `style_bad_property`, or the fallback when the error has none. */
  code: string;
  /** The message after the code (the whole message when there is no code). */
  text: string;
}

// Codes are snake_case with at least one underscore, so an uncoded message
// that happens to start with a lower-case word and a colon is left whole.
const CODED = /^([a-z][a-z0-9]*(?:_[a-z0-9]+)+): ([\s\S]*)$/;

/**
 * Splits a thrown wasm error (an Error, or its message) into code and text.
 * `fallback` is the code for an uncoded error: '' by default (F1), the
 * recorder passes its own (`record_mux_failed`, `internal`, ...).
 */
export function parseWasmError(e: unknown, fallback = ''): WasmError {
  const message = e instanceof Error ? e.message : typeof e === 'string' ? e : String(e);
  const m = CODED.exec(message);
  return m ? { code: m[1]!, text: m[2]! } : { code: fallback, text: message };
}

/** An error with a code the page can explain: the clone service's and the activity service's, and the player's own. */
export class ApiError extends Error {
  readonly code: string;
  readonly status: number;
  constructor(code: string, message: string, status = 0) {
    super(message);
    this.name = 'ApiError';
    this.code = code;
    this.status = status;
  }
}
