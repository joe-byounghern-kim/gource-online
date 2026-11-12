// SPDX-License-Identifier: MIT
import { Buffer } from 'node:buffer';
import { test, expect } from '@playwright/test';
import {
  PLAYER_LOG_FETCH, PLAYER_MAX_CHUNK_BYTES, PLAYER_MAX_FRAGMENT, PLAYER_MAX_LOG_BYTES, PLAYER_MIN_CHUNK_BYTES, PLAYER_PROTOCOL,
  PLAYER_PROTOCOL_VERSION, PlayerHandoffReceiver, parsePlayerFragment, parsePlayerMessage, parsePlayerOrigins, playerHandoffMessages,
  playerMessage, playerOriginAllowed, playerUrl, type HandoffResult, type PlayerMessage,
} from '../src/index.js';

const PLAYER = 'https://viewer.example.test/';
const SOURCES = ['https://app.example.test'];
const envelope = { protocol: PLAYER_PROTOCOL, version: PLAYER_PROTOCOL_VERSION };
const K = PLAYER_MIN_CHUNK_BYTES;
const bytes = (length: number) => Uint8Array.from({ length }, (_, i) => i % 251);
/** As the player receives it: through structured clone, buffers transferred. */
const delivered = (messages: PlayerMessage[]): unknown[] => messages.map((m) => structuredClone(m, { transfer: m.type === 'chunk' ? [m.data] : [] }));
const deliver = (receiver: PlayerHandoffReceiver, messages: unknown[]) => messages.map((m) => receiver.receive(m));
const outcome = (results: HandoffResult[]) => results.find((r) => r.kind === 'error' || r.kind === 'complete') ?? results.at(-1);

test('a public log URL round-trips through the fragment, with its title and options', () => {
  const url = playerUrl(PLAYER, { src: 'https://app.example.test/api/log?x=1&y=2', title: 'owner/repo & 저장소', options: { 'seconds-per-day': '0.5', hide: 'date,files' } }, SOURCES);
  expect(new URL(url).search).toBe('');
  expect(parsePlayerFragment(new URL(url).hash, PLAYER, SOURCES)).toEqual({
    src: 'https://app.example.test/api/log?x=1&y=2', title: 'owner/repo & 저장소', options: { 'seconds-per-day': '0.5', hide: 'date,files' } });
  // The player's own origin is always a source, for a log it serves itself.
  expect(parsePlayerFragment('#src=%2Flog', PLAYER, [])).toEqual({ src: 'https://viewer.example.test/log', options: {} });
  expect(PLAYER_LOG_FETCH).toEqual({ credentials: 'omit', referrerPolicy: 'no-referrer', redirect: 'error' });
});

test('every URL playerUrl builds is one the player reads, and one past the limit is refused when built', () => {
  // Percent-encoding multiplies non-ASCII values; the cap applies to the encoded fragment.
  const options: Record<string, string> = {};
  for (let i = 0; i < 64; i++) options[`o${i}`] = '가'.repeat(1024);
  expect(() => playerUrl(PLAYER, { src: '/log', options }, [])).toThrow('Invalid player handoff.');
  const fit: Record<string, string> = {};
  let built = '';
  for (let i = 0; i < 64; i++) {
    try { built = playerUrl(PLAYER, { src: '/log', options: { ...fit, [`o${i}`]: 'x'.repeat(1024) } }, []); } catch { break; }
    fit[`o${i}`] = 'x'.repeat(1024);
  }
  expect(Object.keys(fit).length).toBeGreaterThan(1);
  expect(new URL(built).hash.length - 1).toBeLessThanOrEqual(PLAYER_MAX_FRAGMENT);
  expect(Object.keys(parsePlayerFragment(new URL(built).hash, PLAYER, []).options).length).toBe(Object.keys(fit).length);
});

test('the fragment refuses other origins, credentials, odd schemes, unknown or repeated keys and bad options', () => {
  for (const hash of [
    '', '#title=x', '#src=', '#src=.', '#src=%2F', '#src=https%3A%2F%2Fevil.example.test%2Flog', '#src=https%3A%2F%2Fapp.example.test.evil.test%2Flog',
    '#src=%2F%2Fevil.example.test%2Flog', '#src=%5C%5Cevil.example.test%2Flog', '#src=data%3Atext%2Fplain%2Cx', '#src=javascript%3Aalert(1)',
    '#src=https%3A%2F%2Fuser%3Apass%40app.example.test%2Flog', '#src=https%3A%2F%2Fapp.example.test%2Flog%23x',
    '#src=%2Flog&src=%2Fother', '#src=%2Flog&extra=1', '#src=%2Flog&o.Bad=1', '#src=%2Flog&o.=1', '#src=%2Flog&o.__proto__=x',
    `#src=%2Flog&title=${'x'.repeat(201)}`, '#src=%2Flog&title=%E2%80%AEevil', '#src=%2Flog&title=a%0Ab',
    `#src=%2Flog&o.hide=${'x'.repeat(1025)}`, `#src=%2F${'x'.repeat(2048)}`, `#src=%2Flog&title=${'x'.repeat(PLAYER_MAX_FRAGMENT)}`,
  ]) expect(() => parsePlayerFragment(hash, PLAYER, SOURCES), hash).toThrow('Invalid player handoff.');
  for (const player of ['http://viewer.example.test/', `${PLAYER}#x`, `${PLAYER}#`, 'not a url'])
    expect(() => playerUrl(player, { src: '/log', options: {} }, []), player).toThrow('Invalid player handoff.');
  expect(() => playerUrl(PLAYER, { src: 'https://evil.example.test/log', options: {} }, SOURCES)).toThrow();
  expect(() => playerUrl(PLAYER, { src: '/log', title: '‮evil', options: {} }, [])).toThrow();
});

test('allowed origins are exact HTTPS origins, loopback only when asked for, and match only exactly', () => {
  expect(parsePlayerOrigins(['https://APP.example.test', 'https://app.example.test:443', 'https://[::1]:8443'])).toEqual(['https://app.example.test', 'https://[::1]:8443']);
  expect(() => parsePlayerOrigins(['http://127.0.0.1:8443'])).toThrow();
  expect(parsePlayerOrigins(['http://127.0.0.1:8443', 'http://localhost:5173'], { loopback: true })).toEqual(['http://127.0.0.1:8443', 'http://localhost:5173']);
  for (const bad of ['http://app.example.test', 'https://app.example.test/', 'https://*.example.test', 'https://app.example.test/path', 'null', '',
    'https://a.test:99999', 'https://[1]', 'https://a..b', 'https://.a.test'])
    expect(() => parsePlayerOrigins([bad], { loopback: true }), bad).toThrow('Invalid player handoff.');
  expect(() => parsePlayerOrigins(Array.from({ length: 17 }, (_, i) => `https://a${i}.test`))).toThrow();
  expect(playerOriginAllowed('https://app.example.test', SOURCES)).toBe(true);
  for (const origin of ['https://app.example.test.evil.test', 'https://evil.test', 'null'])
    expect(playerOriginAllowed(origin, [...SOURCES, 'null'])).toBe(false);
});

test('messages of other protocols are ignored, and a wrong version or shape is refused', () => {
  for (const data of [null, 'gource-player', [], { type: 'ready' }, { protocol: 'other', version: 1, type: 'ready' }]) expect(parsePlayerMessage(data)).toBeNull();
  expect(parsePlayerMessage({ ...envelope, version: 2, type: 'ready' })).toEqual({ code: 'version' });
  const begin = { ...envelope, type: 'begin', id: 'handoff-1', bytes: 3 * K, chunk_bytes: K, chunks: 3, options: {} };
  expect(parsePlayerMessage(begin)).toEqual({ message: begin });
  for (const data of [
    { ...envelope, type: 'nope' },
    { ...envelope, type: 'ready', extra: 1 },
    { ...begin, bytes: 0 },
    { ...begin, chunks: 4 },
    { ...begin, chunks: 2 },
    { ...begin, chunk_bytes: K - 1, chunks: 4 },
    { ...begin, chunk_bytes: PLAYER_MAX_CHUNK_BYTES + 1, chunks: 1 },
    { ...begin, id: 'short' },
    { ...begin, bytes: PLAYER_MAX_LOG_BYTES + 1, chunk_bytes: PLAYER_MAX_CHUNK_BYTES, chunks: 65 },
    { ...begin, options: { 'Bad Name': 'x' } },
    { ...begin, bytes: 1.5, chunks: 1 },
    { ...begin, title: '‮' },
    { ...envelope, type: 'chunk', id: 'handoff-1', seq: 0, data: 'text' },
    { ...envelope, type: 'chunk', id: 'handoff-1', seq: 0, data: new Uint8Array(4) },
    { ...envelope, type: 'chunk', id: 'handoff-1', seq: 0, data: new SharedArrayBuffer(4) },
    { ...envelope, type: 'chunk', id: 'handoff-1', seq: -1, data: new ArrayBuffer(1) },
    { ...envelope, type: 'error', code: 'other' },
  ]) expect(parsePlayerMessage(data), JSON.stringify(data)).toEqual({ code: 'invalid' });
  expect(parsePlayerMessage({ ...begin, bytes: PLAYER_MAX_LOG_BYTES, chunk_bytes: PLAYER_MAX_CHUNK_BYTES, chunks: 64 })).not.toBeNull();
  expect(parsePlayerMessage(playerMessage({ type: 'error', id: 'handoff-1', code: 'size' }))).toEqual({ message: { ...envelope, type: 'error', id: 'handoff-1', code: 'size' } });
});

test('a log sent in chunks arrives whole through structured clone, with its title and options', () => {
  const log = bytes(3 * K + 100);
  const messages = playerHandoffMessages(log, { id: 'handoff-1', title: 'Mine', options: { hide: 'date' } }, K);
  expect(messages.map((m) => m.type)).toEqual(['begin', 'chunk', 'chunk', 'chunk', 'chunk', 'end']);
  const receiver = new PlayerHandoffReceiver();
  const results = deliver(receiver, delivered(messages));
  expect(results.slice(0, -1).every((r) => r.kind === 'pending')).toBe(true);
  expect(results.at(-1)).toEqual({ kind: 'complete', id: 'handoff-1', log, title: 'Mine', options: { hide: 'date' } });
  // Done once: anything later, a second log included, changes nothing and is not an error.
  expect(deliver(receiver, delivered(playerHandoffMessages(log, { id: 'handoff-2', options: {} }, K)))).toEqual(Array(6).fill({ kind: 'ignored' }));
  expect(receiver.timeout()).toEqual({ kind: 'ignored' });
});

test('a chunk is exactly its bytes, even from a view of a larger buffer such as a Node Buffer', () => {
  // Node keeps small Buffers in one shared pool: this one's buffer holds other data too.
  const pooled = Buffer.from(bytes(100));
  const sub = new Uint8Array(new ArrayBuffer(4 * K)).subarray(K + 3, 3 * K);
  sub.set(bytes(sub.byteLength));
  for (const log of [pooled, sub]) {
    expect(log.buffer.byteLength).toBeGreaterThan(log.byteLength);
    const messages = playerHandoffMessages(log, { id: 'handoff-1', options: {} }, K);
    const data = messages.flatMap((m) => (m.type === 'chunk' ? [new Uint8Array(m.data)] : []));
    expect(data.map((d) => d.buffer.byteLength)).toEqual(log.byteLength > K ? [K, log.byteLength - K] : [log.byteLength]);
    expect(Buffer.concat(data).equals(Buffer.from(log))).toBe(true);
    expect(outcome(deliver(new PlayerHandoffReceiver(), delivered(messages)))).toMatchObject({ kind: 'complete' });
  }
  expect(() => playerHandoffMessages(new Uint8Array(new SharedArrayBuffer(8)), { id: 'handoff-1', options: {} })).toThrow();
});

test('out-of-order, mismatched, wrongly sized and short handoffs fail once, then everything is ignored', () => {
  const sent = () => delivered(playerHandoffMessages(bytes(3 * K), { id: 'handoff-1', options: {} }, K));
  const [b, c0, c1, c2, end] = [0, 1, 2, 3, 4];
  const cases: Array<[(m: unknown[]) => unknown[], HandoffResult]> = [
    [(m) => [m[b], m[c1], m[c0], m[c2], m[end]], { kind: 'error', id: 'handoff-1', code: 'order' }],
    [(m) => [m[c0]], { kind: 'error', code: 'order' }],
    [(m) => [m[b], m[c0], m[end]], { kind: 'error', id: 'handoff-1', code: 'order' }],
    [(m) => [m[b], m[b]], { kind: 'error', id: 'handoff-1', code: 'order' }],
    [(m) => [m[b], { ...(m[c0] as object), id: 'handoff-2' }], { kind: 'error', id: 'handoff-1', code: 'order' }],
    [(m) => [m[b], { ...(m[c0] as object), data: new ArrayBuffer(K - 1) }], { kind: 'error', id: 'handoff-1', code: 'size' }],
    [(m) => [m[b], m[c0], m[c1], { ...(m[c2] as object), data: new ArrayBuffer(K + 1) }], { kind: 'error', id: 'handoff-1', code: 'size' }],
    [(m) => [m[b], { ...envelope, version: 9, type: 'end', id: 'handoff-1' }], { kind: 'error', id: 'handoff-1', code: 'version' }],
    [(m) => [m[b], { ...envelope, type: 'chunk', id: 'handoff-1', seq: 0, data: 'x' }], { kind: 'error', id: 'handoff-1', code: 'invalid' }],
  ];
  for (const [edit, expected] of cases) {
    const receiver = new PlayerHandoffReceiver();
    expect(outcome(deliver(receiver, edit(sent())))).toEqual(expected);
    expect(deliver(receiver, sent())).toEqual(Array(5).fill({ kind: 'ignored' }));
  }
});

test('the player’s own messages and other protocols never disturb a handoff', () => {
  const receiver = new PlayerHandoffReceiver();
  const log = bytes(10);
  const noise = [playerMessage({ type: 'ready' }), playerMessage({ type: 'ack', id: 'handoff-1' }), playerMessage({ type: 'error', code: 'timeout' }), { type: 'other' }, 'x'];
  expect(deliver(receiver, noise)).toEqual(Array(noise.length).fill({ kind: 'ignored' }));
  const [begin, chunk, end] = delivered(playerHandoffMessages(log, { id: 'handoff-1', options: {} }));
  expect(deliver(receiver, [begin, ...noise, chunk, ...noise, end]).at(-1)).toEqual({ kind: 'complete', id: 'handoff-1', log, options: {} });
});

test('a sender that gives up ends its own handoff at once, and no other', () => {
  const receiver = new PlayerHandoffReceiver();
  const [begin, chunk] = delivered(playerHandoffMessages(bytes(10), { id: 'handoff-1', options: {} }));
  receiver.receive(begin);
  expect(receiver.receive(playerMessage({ type: 'error', id: 'handoff-2', code: 'timeout' }))).toEqual({ kind: 'ignored' });
  expect(receiver.receive(playerMessage({ type: 'error', id: 'handoff-1', code: 'size' }))).toEqual({ kind: 'error', id: 'handoff-1', code: 'size' });
  expect(receiver.receive(chunk)).toEqual({ kind: 'ignored' });
});

test('a handoff that does not finish times out, once', () => {
  const receiver = new PlayerHandoffReceiver();
  const [begin] = playerHandoffMessages(bytes(10), { id: 'handoff-1', options: {} });
  receiver.receive(begin);
  expect(receiver.timeout()).toEqual({ kind: 'error', id: 'handoff-1', code: 'timeout' });
  expect(receiver.timeout()).toEqual({ kind: 'ignored' });
  expect(new PlayerHandoffReceiver().timeout()).toEqual({ kind: 'error', code: 'timeout' });
});

test('the player can cancel a handoff, once, and a sender reads the code', () => {
  const receiver = new PlayerHandoffReceiver();
  const [begin, chunk] = playerHandoffMessages(bytes(10), { id: 'handoff-1', options: {} });
  receiver.receive(begin);
  expect(receiver.cancel()).toEqual({ kind: 'error', id: 'handoff-1', code: 'cancelled' });
  expect(receiver.cancel()).toEqual({ kind: 'ignored' });
  expect(receiver.receive(chunk)).toEqual({ kind: 'ignored' });
  expect(parsePlayerMessage(playerMessage({ type: 'error', id: 'handoff-1', code: 'cancelled' }))).toHaveProperty('message');
  expect(parsePlayerMessage({ ...playerMessage({ type: 'error', code: 'timeout' }), code: 'gone' })).toEqual({ code: 'invalid' });
});

test('the sender refuses an empty log and a bad id, title or chunk size', () => {
  expect(() => playerHandoffMessages(new Uint8Array(0), { id: 'handoff-1', options: {} })).toThrow();
  expect(() => playerHandoffMessages(bytes(1), { id: 'bad id', options: {} })).toThrow();
  expect(() => playerHandoffMessages(bytes(1), { id: 'handoff-1', title: '', options: {} })).toThrow();
  expect(() => playerHandoffMessages(bytes(1), { id: 'handoff-1', options: {} }, K - 1)).toThrow();
  expect(() => playerHandoffMessages(bytes(1), { id: 'handoff-1', options: {} }, PLAYER_MAX_CHUNK_BYTES + 1)).toThrow();
});
