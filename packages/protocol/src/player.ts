// SPDX-License-Identifier: MIT

/**
 * How a page gives the player a log to play. A public log goes by URL, in the player's fragment.
 * A log that must stay out of URLs goes by `postMessage`, from the page that opened or embeds the player.
 * Pages talk to the player only through this protocol.
 */
export const PLAYER_PROTOCOL = 'gource-player';
export const PLAYER_PROTOCOL_VERSION = 1;
/** The hosted service's log cap: the player takes no more than the service would serve. */
export const PLAYER_MAX_LOG_BYTES = 256 * 1024 * 1024;
/** Every chunk but the last has the size `begin` declares, within these bounds. */
export const PLAYER_MIN_CHUNK_BYTES = 64 * 1024;
export const PLAYER_MAX_CHUNK_BYTES = 4 * 1024 * 1024;
/** From the player's `ready` to the sender's `end`; the player gives up after it. */
export const PLAYER_HANDOFF_TIMEOUT_MS = 60_000;
/** The longest fragment `playerUrl` builds and `parsePlayerFragment` reads. */
export const PLAYER_MAX_FRAGMENT = 16384;
/**
 * How the player fetches a `src` log: never with cookies or other credentials, so a link can only
 * ever play what anyone could download, and never through a redirect, which could leave the allowed
 * sources. Owner-only logs go by `postMessage` instead.
 */
export const PLAYER_LOG_FETCH = Object.freeze({ credentials: 'omit', referrerPolicy: 'no-referrer', redirect: 'error' } as const);

const MAX_OPTIONS = 64;
const MAX_OPTION_VALUE = 1024;
const MAX_TITLE = 200;
const MAX_SRC = 2048;
const OPTION_NAME = /^[a-z][a-z0-9-]{0,39}$/;
const HANDOFF_ID = /^[A-Za-z0-9_-]{8,64}$/;
/** C0 and C1 controls and bidirectional overrides: a title is shown as text, never reordered. */
const UNSAFE_TEXT = /[\u0000-\u001f\u007f-\u009f‪-‮⁦-⁩]/;

/** Gource option values by option name, applied before playback starts. */
export type PlayerOptions = Readonly<Record<string, string>>;

export interface PlayerUrlHandoff {
  /** The URL the player fetches the log from: absolute, or a path on the player's own origin. */
  src: string;
  title?: string;
  options: PlayerOptions;
}

/** `cancelled`: the player gave up waiting, because the person opened another log. */
export type PlayerErrorCode = 'version' | 'invalid' | 'size' | 'order' | 'timeout' | 'cancelled';

interface Envelope { protocol: typeof PLAYER_PROTOCOL; version: typeof PLAYER_PROTOCOL_VERSION }
export type PlayerMessage = Envelope & (
  /** Player to sender: loaded and waiting for a log. */
  | { type: 'ready' }
  /** Sender to player: a log of `bytes` bytes follows in `chunks` chunks of `chunk_bytes` (the last may be shorter). */
  | { type: 'begin'; id: string; bytes: number; chunk_bytes: number; chunks: number; title?: string; options: PlayerOptions }
  /** Sender to player: chunk `seq` (from 0), transferred. */
  | { type: 'chunk'; id: string; seq: number; data: ArrayBuffer }
  | { type: 'end'; id: string }
  /** Player to sender: the whole log arrived and playback starts. */
  | { type: 'ack'; id: string }
  | { type: 'error'; id?: string; code: PlayerErrorCode }
);
type Body<T extends PlayerMessage['type']> = Omit<Extract<PlayerMessage, { type: T }>, keyof Envelope>;

export function playerMessage<T extends PlayerMessage['type']>(body: Body<T> & { type: T }): PlayerMessage {
  return { protocol: PLAYER_PROTOCOL, version: PLAYER_PROTOCOL_VERSION, ...body } as PlayerMessage;
}

const invalid = () => new Error('Invalid player handoff.');
const isRecord = (value: unknown): value is Record<string, unknown> =>
  typeof value === 'object' && value !== null && !Array.isArray(value) && Object.getPrototypeOf(value) === Object.prototype;
const exactKeys = (value: Record<string, unknown>, required: readonly string[], optional: readonly string[] = []) =>
  required.every((key) => Object.hasOwn(value, key)) && Object.keys(value).every((key) => required.includes(key) || optional.includes(key));
const count = (value: unknown, min: number, max: number): value is number =>
  typeof value === 'number' && Number.isSafeInteger(value) && value >= min && value <= max;
const title = (value: unknown): value is string =>
  typeof value === 'string' && value.length > 0 && value.length <= MAX_TITLE && !UNSAFE_TEXT.test(value);

function validOptions(value: unknown): value is PlayerOptions {
  if (!isRecord(value)) return false;
  const entries = Object.entries(value);
  return entries.length <= MAX_OPTIONS && entries.every(([name, option]) =>
    OPTION_NAME.test(name) && typeof option === 'string' && option.length <= MAX_OPTION_VALUE);
}

const loopback = (url: URL) => url.hostname === '127.0.0.1' || url.hostname === 'localhost' || url.hostname === '[::1]';

/**
 * Exact origins allowed to hand the player a log, or to serve one by URL. HTTPS only; `loopback`
 * also allows http://localhost, 127.0.0.1 and [::1], for development only. No paths or wildcards.
 */
export function parsePlayerOrigins(value: readonly string[], { loopback: allowLoopback = false } = {}): readonly string[] {
  if (!Array.isArray(value) || value.length > 16) throw invalid();
  const origins: string[] = [];
  for (const entry of value) {
    if (typeof entry !== 'string' || !/^https?:\/\/(?:\[[0-9a-fA-F:]+\]|[a-zA-Z0-9-]+(?:\.[a-zA-Z0-9-]+)*)(?::[0-9]{1,5})?$/.test(entry)) throw invalid();
    let url: URL;
    try { url = new URL(entry); } catch { throw invalid(); }
    if (url.protocol !== 'https:' && !(allowLoopback && loopback(url))) throw invalid();
    if (!origins.includes(url.origin)) origins.push(url.origin);
  }
  return Object.freeze(origins);
}

/** Whether a message or log source comes from an allowed origin: an exact match, never a prefix. */
export function playerOriginAllowed(origin: string, allowed: readonly string[]): boolean {
  return origin !== 'null' && allowed.includes(origin);
}

function checkSrc(src: unknown, player: URL, sources: readonly string[]): string {
  if (typeof src !== 'string' || !src || src.length > MAX_SRC) throw invalid();
  let url: URL;
  // A path is the player's own origin: a log it serves itself, as the CLI does.
  try { url = new URL(src, player); } catch { throw invalid(); }
  if ((url.protocol !== 'https:' && url.protocol !== 'http:') || url.username || url.password || url.hash) throw invalid();
  if (url.origin !== player.origin && !playerOriginAllowed(url.origin, sources)) throw invalid();
  if (url.origin === player.origin && url.pathname === player.pathname && url.search === player.search) throw invalid();
  return url.href;
}

function playerPage(player: string): URL {
  if (typeof player !== 'string' || player.includes('#')) throw invalid();
  let url: URL;
  try { url = new URL(player); } catch { throw invalid(); }
  if (url.protocol !== 'https:' && !(url.protocol === 'http:' && loopback(url))) throw invalid();
  return url;
}

/**
 * The player URL for a public log: everything goes in the fragment, which is never sent to a server.
 * Refuses a handoff whose fragment would exceed `PLAYER_MAX_FRAGMENT`, so the player can read every URL it builds.
 */
export function playerUrl(player: string, handoff: PlayerUrlHandoff, sources: readonly string[]): string {
  const url = playerPage(player);
  const params = new URLSearchParams({ src: checkSrc(handoff.src, url, sources) });
  if (handoff.title !== undefined) {
    if (!title(handoff.title)) throw invalid();
    params.set('title', handoff.title);
  }
  if (!validOptions(handoff.options)) throw invalid();
  for (const [name, value] of Object.entries(handoff.options)) params.set(`o.${name}`, value);
  const fragment = params.toString();
  if (fragment.length > PLAYER_MAX_FRAGMENT) throw invalid();
  url.hash = fragment;
  return url.href;
}

/** Reads the player's own fragment. `player` is the player page's URL; `sources` the origins it may fetch logs from. */
export function parsePlayerFragment(hash: string, player: string, sources: readonly string[]): PlayerUrlHandoff {
  if (typeof hash !== 'string') throw invalid();
  const fragment = hash.startsWith('#') ? hash.slice(1) : hash;
  if (fragment.length > PLAYER_MAX_FRAGMENT) throw invalid();
  const page = new URL(player);
  page.hash = '';
  const seen = new Set<string>(), options: Record<string, string> = {};
  let src: string | undefined, name: string | undefined;
  for (const [key, value] of new URLSearchParams(fragment)) {
    if (seen.has(key)) throw invalid();
    seen.add(key);
    if (key === 'src') src = value;
    else if (key === 'title') name = value;
    // Checked before it is a key, so `o.__proto__` cannot slip past as a prototype assignment.
    else if (key.startsWith('o.') && OPTION_NAME.test(key.slice(2))) options[key.slice(2)] = value;
    else throw invalid();
  }
  if (name !== undefined && !title(name)) throw invalid();
  if (!validOptions(options)) throw invalid();
  return { src: checkSrc(src, page, sources), ...(name === undefined ? {} : { title: name }), options };
}

/**
 * Validates one `postMessage` payload. Null for anything that is not this protocol, which the receiver
 * ignores. An error code for this protocol in a wrong version or shape.
 */
export function parsePlayerMessage(data: unknown): { message: PlayerMessage } | { code: PlayerErrorCode } | null {
  if (!isRecord(data) || data.protocol !== PLAYER_PROTOCOL) return null;
  if (data.version !== PLAYER_PROTOCOL_VERSION) return { code: 'version' };
  const base = ['protocol', 'version', 'type'];
  const id = (value: unknown) => typeof value === 'string' && HANDOFF_ID.test(value);
  let ok: boolean;
  switch (data.type) {
    case 'ready': ok = exactKeys(data, base); break;
    case 'begin':
      ok = exactKeys(data, [...base, 'id', 'bytes', 'chunk_bytes', 'chunks', 'options'], ['title']) && id(data.id)
        && count(data.bytes, 1, PLAYER_MAX_LOG_BYTES) && count(data.chunk_bytes, PLAYER_MIN_CHUNK_BYTES, PLAYER_MAX_CHUNK_BYTES)
        && data.chunks === Math.ceil((data.bytes as number) / (data.chunk_bytes as number))
        && (data.title === undefined || title(data.title)) && validOptions(data.options);
      break;
    case 'chunk':
      ok = exactKeys(data, [...base, 'id', 'seq', 'data']) && id(data.id) && count(data.seq, 0, PLAYER_MAX_LOG_BYTES)
        && data.data instanceof ArrayBuffer;
      break;
    case 'end': case 'ack': ok = exactKeys(data, [...base, 'id']) && id(data.id); break;
    case 'error':
      ok = exactKeys(data, [...base, 'code'], ['id']) && (data.id === undefined || id(data.id))
        && ['version', 'invalid', 'size', 'order', 'timeout', 'cancelled'].includes(data.code as string);
      break;
    default: ok = false;
  }
  return ok ? { message: data as unknown as PlayerMessage } : { code: 'invalid' };
}

export type HandoffResult =
  /** Not this protocol, a player-to-sender message, or anything after the handoff ended: nothing changed. */
  | { kind: 'ignored' }
  | { kind: 'pending' }
  | { kind: 'complete'; id: string; log: Uint8Array; title?: string; options: PlayerOptions }
  /** The handoff failed. Reported once; everything after it is ignored. */
  | { kind: 'error'; id?: string; code: PlayerErrorCode };

/**
 * The player's side of a `postMessage` handoff: one log, in order, with the protocol's fixed limits.
 * The first error ends it, and so does completion; later messages are ignored. Origin and source
 * checks belong to the caller, before this.
 */
export class PlayerHandoffReceiver {
  private begin: Extract<PlayerMessage, { type: 'begin' }> | null = null;
  private readonly parts: Uint8Array[] = [];
  private received = 0;
  private state: 'waiting' | 'receiving' | 'closed' = 'waiting';

  receive(data: unknown): HandoffResult {
    if (this.state === 'closed') return { kind: 'ignored' };
    const parsed = parsePlayerMessage(data);
    if (parsed === null) return { kind: 'ignored' };
    if ('code' in parsed) return this.fail(parsed.code, this.begin?.id);
    const message = parsed.message;
    // A sender that gives up says so for its handoff; the player's own messages change nothing.
    if (message.type === 'error' && this.begin && message.id === this.begin.id) return this.fail(message.code, message.id);
    if (message.type === 'ready' || message.type === 'ack' || message.type === 'error') return { kind: 'ignored' };
    if (message.type === 'begin') {
      if (this.state !== 'waiting') return this.fail('order', this.begin?.id);
      this.begin = message;
      this.state = 'receiving';
      return { kind: 'pending' };
    }
    const begin = this.begin;
    if (!begin || message.id !== begin.id) return this.fail('order', begin?.id);
    if (message.type === 'chunk') {
      if (message.seq !== this.parts.length || message.seq >= begin.chunks) return this.fail('order', begin.id);
      const size = message.data.byteLength;
      const expected = message.seq === begin.chunks - 1 ? begin.bytes - begin.chunk_bytes * (begin.chunks - 1) : begin.chunk_bytes;
      if (size !== expected) return this.fail('size', begin.id);
      this.parts.push(new Uint8Array(message.data));
      this.received += size;
      return { kind: 'pending' };
    }
    if (this.parts.length !== begin.chunks || this.received !== begin.bytes) return this.fail('order', begin.id);
    const log = new Uint8Array(this.received);
    let at = 0;
    for (const part of this.parts.splice(0)) { log.set(part, at); at += part.byteLength; }
    this.state = 'closed';
    return { kind: 'complete', id: begin.id, log, ...(begin.title === undefined ? {} : { title: begin.title }), options: begin.options };
  }

  /** The handoff did not finish in time. */
  timeout(): HandoffResult {
    return this.state === 'closed' ? { kind: 'ignored' } : this.fail('timeout', this.begin?.id);
  }

  /** The player stops waiting for this handoff (another log was opened). */
  cancel(): HandoffResult {
    return this.state === 'closed' ? { kind: 'ignored' } : this.fail('cancelled', this.begin?.id);
  }

  private fail(code: PlayerErrorCode, id?: string): HandoffResult {
    this.state = 'closed';
    this.parts.length = 0;
    return { kind: 'error', ...(id === undefined ? {} : { id }), code };
  }
}

/**
 * The sender's messages for one log: `begin`, the chunks and `end`. Each chunk's buffer is a fresh copy of
 * exactly its bytes (never the log's whole underlying buffer, which a view such as Node's `Buffer` shares
 * with other data), so it can be transferred.
 */
export function playerHandoffMessages(log: Uint8Array, handoff: { id: string; title?: string; options: PlayerOptions },
  chunkBytes = PLAYER_MAX_CHUNK_BYTES): PlayerMessage[] {
  if (!HANDOFF_ID.test(handoff.id) || !count(chunkBytes, PLAYER_MIN_CHUNK_BYTES, PLAYER_MAX_CHUNK_BYTES)
    || !(log.buffer instanceof ArrayBuffer) || !count(log.byteLength, 1, PLAYER_MAX_LOG_BYTES)
    || (handoff.title !== undefined && !title(handoff.title)) || !validOptions(handoff.options)) throw invalid();
  const chunks: PlayerMessage[] = [];
  for (let at = 0; at < log.byteLength; at += chunkBytes) {
    const data = log.buffer.slice(log.byteOffset + at, log.byteOffset + Math.min(at + chunkBytes, log.byteLength));
    chunks.push(playerMessage({ type: 'chunk', id: handoff.id, seq: chunks.length, data }));
  }
  return [
    playerMessage({ type: 'begin', id: handoff.id, bytes: log.byteLength, chunk_bytes: chunkBytes, chunks: chunks.length,
      ...(handoff.title === undefined ? {} : { title: handoff.title }), options: handoff.options }),
    ...chunks,
    playerMessage({ type: 'end', id: handoff.id }),
  ];
}
