// SPDX-License-Identifier: GPL-3.0-or-later

// Module worker that downloads (or reads) the log and parses it off
// the main thread. Real mode: the gource-web wasm LogParser turns byte chunks
// into encoded batches for GourceApp.append_batch. Mock mode: batches are
// whole custom-log lines (understood by the mock engine), and a `mock:` URL
// fabricates a log with streamed progress. Sources are a URL (the clone
// service's log, a `?log=` deep link) or a local file. Both parsers accept a
// custom log or the output of Gource's git log command (a private
// repository's history, made in the user's own checkout).

import { loadWasm, EngineMissingError } from '../engine/load';
import type { LogParser } from '../engine/types';
import type { FromWorker, LogFetch, LogSource, ToWorker } from './protocol';

interface WorkerScope {
  postMessage(message: FromWorker, transfer?: Transferable[]): void;
  addEventListener(type: 'message', listener: (event: MessageEvent<ToWorker>) => void): void;
}
const scope = self as unknown as WorkerScope;

let abort: AbortController | null = null;

/** Common surface of the wasm parser and the mock line splitter. */
interface ChunkParser {
  push(chunk: Uint8Array): Uint8Array;
  finish(): Uint8Array;
  commits(): number;
  bytes(): number;
  skipped_lines(): number;
  /** `custom`, `git` (the output of Gource's git log command) or `unknown`. */
  format(): string;
  free(): void;
}

/**
 * Splits bytes into whole lines and counts commits the way Gource groups them.
 * Git log output (`user:` headers and raw lines) is turned into custom-log
 * lines first, so the mock engine reads both formats.
 */
class MockLineParser implements ChunkParser {
  private pending = new Uint8Array(0);
  private total = 0;
  private commitCount = 0;
  private skipped = 0;
  private lastKey = '';
  private kind: 'unknown' | 'custom' | 'git' = 'unknown';
  private gitUser = '';
  private gitTime = '';
  private gitExpectTime = false;
  private readonly decoder = new TextDecoder();
  private readonly encoder = new TextEncoder();

  push(chunk: Uint8Array): Uint8Array {
    this.total += chunk.byteLength;
    const joined = new Uint8Array(this.pending.byteLength + chunk.byteLength);
    joined.set(this.pending, 0);
    joined.set(chunk, this.pending.byteLength);
    const end = joined.lastIndexOf(10);
    if (end < 0) {
      this.pending = joined;
      return new Uint8Array(0);
    }
    const complete = joined.slice(0, end + 1);
    this.pending = joined.slice(end + 1);
    return this.lines(complete);
  }

  finish(): Uint8Array {
    const rest = this.pending;
    this.pending = new Uint8Array(0);
    if (this.kind === 'unknown') this.kind = 'custom';
    if (rest.byteLength === 0) return rest;
    const withNewline = new Uint8Array(rest.byteLength + 1);
    withNewline.set(rest, 0);
    withNewline[rest.byteLength] = 10;
    return this.lines(withNewline);
  }

  /** Counts commits and returns the batch: the lines themselves, or custom lines made from git output. */
  private lines(bytes: Uint8Array): Uint8Array {
    const text = this.decoder.decode(bytes);
    if (this.kind === 'unknown') {
      const first = text.replace(/^\uFEFF/, '').trimStart();
      if (first !== '') this.kind = first.startsWith('user:') ? 'git' : 'custom';
    }
    if (this.kind !== 'git') {
      this.count(text.split('\n'));
      return bytes;
    }
    const out: string[] = [];
    for (const line of text.split('\n')) {
      if (line.startsWith('user:')) {
        this.gitUser = line.slice(5);
        this.gitExpectTime = true;
      } else if (this.gitExpectTime) {
        this.gitTime = line.trim();
        this.gitExpectTime = false;
      } else if (line.startsWith(':')) {
        const tab = line.indexOf('\t');
        const status = line.slice(0, tab).trim().split(/\s+/).pop() ?? '';
        if (tab < 0 || !'AMD'.includes(status)) {
          this.skipped++;
          continue;
        }
        const path = line.slice(tab + 1).replace(/^"|"$/g, '');
        out.push(`${this.gitTime}|${this.gitUser}|${status}|/${path}`);
      }
    }
    this.count(out);
    return out.length > 0 ? this.encoder.encode(`${out.join('\n')}\n`) : new Uint8Array(0);
  }

  private count(lines: string[]): void {
    for (const line of lines) {
      if (line === '') continue;
      const parts = line.split('|');
      if (parts.length < 4) {
        this.skipped++;
        continue;
      }
      const key = `${parts[0]}|${parts[1]}`;
      if (key !== this.lastKey) {
        this.commitCount++;
        this.lastKey = key;
      }
    }
  }

  commits(): number {
    return this.commitCount;
  }
  bytes(): number {
    return this.total;
  }
  skipped_lines(): number {
    return this.skipped;
  }
  format(): string {
    return this.kind;
  }
  free(): void {}
}

/** A deterministic, plausible-looking repository history for `mock:` logs. */
function fabricateLog(): string {
  const users = ['Andrew Caudwell', 'Alice', 'Bob', 'Carol', 'dependabot[bot]'];
  const dirs = ['src', 'src/core', 'src/formats', 'data', 'data/shaders', 'docs', 'tests', 'tests/logs'];
  const exts = ['cpp', 'h', 'md', 'frag', 'vert', 'png', 'log'];
  const lines: string[] = [];
  let seed = 7;
  const rand = (n: number) => {
    seed = (Math.imul(seed, 1103515245) + 12345) >>> 0;
    return (seed >>> 8) % n;
  };
  let t = 1_230_000_000;
  const files: string[] = [];
  for (let c = 0; c < 600; c++) {
    t += 3600 * (1 + rand(96));
    const user = users[rand(users.length)]!;
    const n = 1 + rand(5);
    for (let i = 0; i < n; i++) {
      if (files.length < 12 || rand(4) === 0) {
        const dir = dirs[rand(dirs.length)]!;
        const file = `/${dir}/file${files.length}.${exts[rand(exts.length)]}`;
        files.push(file);
        lines.push(`${t}|${user}|A|${file}`);
      } else if (rand(20) === 0 && files.length > 20) {
        const [file] = files.splice(rand(files.length), 1);
        lines.push(`${t}|${user}|D|${file}`);
      } else {
        lines.push(`${t}|${user}|M|${files[rand(files.length)]}`);
      }
    }
  }
  return `${lines.join('\n')}\n`;
}

function sleep(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(resolve, ms);
    signal.addEventListener('abort', () => {
      clearTimeout(timer);
      reject(new DOMException('Aborted', 'AbortError'));
    });
  });
}

async function* fabricatedChunks(signal: AbortSignal): AsyncGenerator<Uint8Array> {
  const bytes = new TextEncoder().encode(fabricateLog());
  const chunk = Math.ceil(bytes.byteLength / 40);
  for (let off = 0; off < bytes.byteLength; off += chunk) {
    await sleep(40, signal);
    yield bytes.slice(off, off + chunk);
  }
}

async function* fetchedChunks(url: string, init: LogFetch, signal: AbortSignal): AsyncGenerator<Uint8Array> {
  let res: Response;
  try {
    res = await fetch(url, { credentials: init.credentials, referrerPolicy: init.referrerPolicy, redirect: init.redirect, signal });
  } catch (err) {
    if (signal.aborted) throw err;
    // Nothing arrived: the host is unreachable or refuses cross-origin reads.
    throw new LogFetchError(
      "Could not download the log: the server cannot be reached, does not allow this page to read it (CORS), or its origin is not in this page's configured CSP connect-src allowlist.",
    );
  }
  if (!res.ok) {
    let message = `The log request failed: ${res.status} ${res.statusText}`.trim();
    try {
      // A clone service answers a failed job's log with {"error": {code, message}}.
      const body = (await res.json()) as { error?: { message?: string } };
      if (body.error?.message) message = body.error.message;
    } catch {
      // Not JSON: keep the status line.
    }
    throw new LogFetchError(message);
  }
  if (res.headers.get('content-type')?.startsWith('text/html')) {
    throw new LogFetchError('The log URL returned a web page, not a Gource log. Check the address.');
  }
  if (!res.body) {
    yield new Uint8Array(await res.arrayBuffer());
    return;
  }
  const reader = res.body.getReader();
  try {
    for (;;) {
      const { done, value } = await reader.read();
      if (done) return;
      if (value && value.byteLength > 0) yield value;
    }
  } finally {
    reader.releaseLock();
  }
}

/** Reads a local file in chunks (Blob.stream, so large logs never sit in memory twice). */
async function* fileChunks(file: Blob, signal: AbortSignal): AsyncGenerator<Uint8Array> {
  let reader: ReadableStreamDefaultReader<Uint8Array>;
  try {
    reader = file.stream().getReader();
  } catch {
    throw new LogFetchError('The file could not be read.');
  }
  try {
    for (;;) {
      if (signal.aborted) return;
      let result: ReadableStreamReadResult<Uint8Array>;
      try {
        result = await reader.read();
      } catch {
        throw new LogFetchError('The file could not be read.');
      }
      if (result.done) return;
      if (result.value.byteLength > 0) yield result.value;
    }
  } finally {
    reader.releaseLock();
  }
}

function chunksOf(source: LogSource, mock: boolean, signal: AbortSignal): AsyncGenerator<Uint8Array> {
  if (source.kind === 'file') return fileChunks(source.file, signal);
  if (mock && source.url.startsWith('mock:')) return fabricatedChunks(signal);
  return fetchedChunks(source.url, source.fetch ?? {}, signal);
}

class LogFetchError extends Error {}

/** Posts a batch, transferring a buffer that holds exactly its bytes. */
function postBatch(batch: Uint8Array, parser: ChunkParser): void {
  if (batch.byteLength === 0) {
    scope.postMessage({ type: 'progress', commits: parser.commits(), bytes: parser.bytes() });
    return;
  }
  const own =
    batch.byteOffset === 0 && batch.byteLength === batch.buffer.byteLength && batch.buffer instanceof ArrayBuffer
      ? batch
      : batch.slice();
  const buffer = own.buffer as ArrayBuffer;
  scope.postMessage({ type: 'batch', batch: buffer, commits: parser.commits(), bytes: parser.bytes() }, [buffer]);
}

async function run(source: LogSource, mock: boolean, signal: AbortSignal): Promise<void> {
  let parser: ChunkParser;
  if (mock) {
    parser = new MockLineParser();
  } else {
    const mod = await loadWasm();
    parser = new mod.LogParser() as LogParser;
  }
  try {
    const chunks = chunksOf(source, mock, signal);
    for await (const chunk of chunks) {
      if (signal.aborted) break;
      postBatch(parser.push(chunk), parser);
    }
    if (signal.aborted) {
      scope.postMessage({ type: 'cancelled' });
      return;
    }
    const last = parser.finish();
    if (last.byteLength > 0) postBatch(last, parser);
    scope.postMessage({
      type: 'done',
      commits: parser.commits(),
      bytes: parser.bytes(),
      skipped: parser.skipped_lines(),
      format: parser.format() === 'git' ? 'git' : 'custom',
    });
  } finally {
    parser.free();
  }
}

scope.addEventListener('message', (event) => {
  const msg = event.data;
  if (msg.type === 'cancel') {
    abort?.abort();
    return;
  }
  if (msg.type !== 'start') return;
  abort?.abort();
  const controller = new AbortController();
  abort = controller;
  run(msg.source, msg.mock, controller.signal).catch((err: unknown) => {
    if (controller.signal.aborted) {
      scope.postMessage({ type: 'cancelled' });
    } else if (err instanceof EngineMissingError) {
      scope.postMessage({ type: 'error', code: 'engine_missing', message: err.message });
    } else if (err instanceof LogFetchError || err instanceof TypeError) {
      scope.postMessage({
        type: 'error',
        code: 'log_failed',
        message: err instanceof LogFetchError ? err.message : 'The history download was interrupted.',
      });
    } else {
      scope.postMessage({
        type: 'error',
        code: 'parse_failed',
        message: err instanceof Error ? err.message : String(err),
      });
    }
  });
});
