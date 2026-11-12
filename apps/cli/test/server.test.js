// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

import assert from 'node:assert/strict';
import { writeFileSync } from 'node:fs';
import { request as httpRequest } from 'node:http';
import { connect } from 'node:net';
import { dirname, join } from 'node:path';
import { after, before, test } from 'node:test';

import { parsePlayerFragment } from '../../../packages/protocol/dist/index.js';
import { CSP, safeName, startServer } from '../src/server.js';
import { fakeDist } from './helpers.js';

/**
 * @typedef {{ status: number, headers: import('node:http').IncomingHttpHeaders, body: string }} Reply
 */

/**
 * A request with an explicit Host header (fetch does not allow setting it).
 * @param {number} port
 * @param {string} path
 * @param {{ method?: string, host?: string }} [opts]
 * @returns {Promise<Reply>}
 */
function get(port, path, { method = 'GET', host = `127.0.0.1:${port}` } = {}) {
  return new Promise((done, fail) => {
    const req = httpRequest({ host: '127.0.0.1', port, path, method, headers: { host } }, (res) => {
      /** @type {Buffer[]} */
      const parts = [];
      res.on('data', (c) => parts.push(c));
      res.on('end', () => done({ status: res.statusCode ?? 0, headers: res.headers, body: Buffer.concat(parts).toString('utf8') }));
    });
    req.on('error', fail);
    req.end();
  });
}

const LOG = [Buffer.from('user:Jane\n1700000000\n'), Buffer.from(':000000 100644 0 1 A\tREADME\n')];
/** @type {Awaited<ReturnType<typeof startServer>>} */
let server;
let dist = '';

before(async () => {
  dist = fakeDist();
  writeFileSync(join(dirname(dist), 'secret.txt'), 'outside the app\n');
  server = await startServer({ dist, log: LOG, name: 'my repo' });
});
after(() => server.close());

test('the link is the player\'s own: the history at a path with a random token, and the folder name as its title', () => {
  const url = new URL(server.url);
  assert.equal(url.origin, server.origin);
  assert.equal(url.pathname, '/');
  assert.equal(url.search, '');
  assert.equal(server.origin, `http://127.0.0.1:${server.port}`);
  assert.match(server.logPath, /^\/[0-9a-f]{32}\/my%20repo\.gitlog$/);
  // The player accepts it as it is (packages/protocol/src/player.ts): a log on its own origin, no other source.
  const handoff = parsePlayerFragment(url.hash, server.url, []);
  assert.deepEqual(handoff, { src: `${server.origin}${server.logPath}`, title: 'my repo', options: {} });
});

test('the page has the player\'s security headers', async () => {
  const res = await get(server.port, '/');
  assert.equal(res.status, 200);
  assert.match(res.body, /<title>Gource<\/title>/);
  assert.equal(res.headers['content-security-policy'], CSP);
  assert.match(CSP, /connect-src 'self';/);
  assert.match(CSP, /frame-ancestors 'none';/);
  assert.match(CSP, /form-action 'none';/);
  assert.equal(res.headers['referrer-policy'], 'no-referrer');
  assert.equal(res.headers['x-content-type-options'], 'nosniff');
  assert.equal(res.headers['cross-origin-resource-policy'], 'same-origin');
  assert.equal(res.headers['cache-control'], 'no-store');
  // Other paths without an extension are the page too; /index.html as well.
  assert.match((await get(server.port, '/somewhere')).body, /<title>Gource<\/title>/);
  assert.match((await get(server.port, '/index.html')).body, /<title>Gource<\/title>/);
});

test('the history is served once the token matches', async () => {
  const res = await get(server.port, server.logPath);
  assert.equal(res.status, 200);
  assert.equal(res.body, Buffer.concat(LOG).toString('utf8'));
  assert.equal(res.headers['content-type'], 'text/plain; charset=utf-8');
  assert.equal(res.headers['cache-control'], 'no-store');
  assert.equal(res.headers['content-length'], String(Buffer.concat(LOG).length));

  const head = await get(server.port, server.logPath, { method: 'HEAD' });
  assert.equal(head.status, 200);
  assert.equal(head.body, '');

  const token = server.logPath.split('/')[1];
  const wrong = token.replace(/^./, (c) => (c === '0' ? '1' : '0'));
  for (const path of [`/${wrong}/my%20repo.gitlog`, `/${token}/other.gitlog`, '/my%20repo.gitlog', `/${token.slice(1)}/my%20repo.gitlog`]) {
    const res = await get(server.port, path);
    assert.equal(res.status, 404, path);
    assert.ok(!res.body.includes('user:Jane'), path);
  }
});

test('requests for another host name are refused (DNS rebinding)', async () => {
  for (const host of ['evil.example', `evil.example:${server.port}`, `127.0.0.1:${server.port + 1}`, `[::1]:${server.port}`]) {
    const res = await get(server.port, server.logPath, { host });
    assert.equal(res.status, 421, host);
    assert.ok(!res.body.includes('user:Jane'));
  }
  assert.equal((await get(server.port, server.logPath, { host: `localhost:${server.port}` })).status, 200);
  // HTTP/1.0 without a Host header.
  const raw = await new Promise((done, fail) => {
    const socket = connect(server.port, '127.0.0.1', () => socket.end(`GET ${server.logPath} HTTP/1.0\r\n\r\n`));
    let text = '';
    socket.setEncoding('utf8');
    socket.on('data', (c) => (text += c));
    socket.on('end', () => done(text));
    socket.on('error', fail);
  });
  assert.match(String(raw), /^HTTP\/1\.1 421 /);
  assert.ok(!String(raw).includes('user:Jane'));
});

test('only GET and HEAD', async () => {
  const res = await get(server.port, '/', { method: 'POST' });
  assert.equal(res.status, 405);
  assert.equal(res.headers.allow, 'GET, HEAD');
});

test('there is no service: the player calls none, and the static host\'s header file is not served', async () => {
  writeFileSync(join(dist, '_headers'), '/*\n  X-Test: 1\n');
  for (const path of ['/api/features', '/api/auth/session', '/api/analyze', '/api', '/_headers']) {
    const res = await get(server.port, path);
    assert.equal(res.status, 404, path);
  }
});

test('assets have their types; nothing outside the app is served', async () => {
  const js = await get(server.port, '/assets/index-abc.js');
  assert.equal(js.headers['content-type'], 'text/javascript; charset=utf-8');
  assert.equal(js.headers['cache-control'], 'public, max-age=31536000, immutable');
  assert.equal((await get(server.port, '/assets/engine.wasm')).headers['content-type'], 'application/wasm');
  assert.equal((await get(server.port, '/assets/missing.js')).status, 404);
  for (const path of ['/..%2fsecret.txt', '/assets/..%2f..%2fsecret.txt', '/%2e%2e/secret.txt', '/../secret.txt']) {
    const res = await get(server.port, path);
    assert.ok(!res.body.includes('outside the app'), path);
    assert.ok(res.status === 404 || res.status === 400, `${path}: ${res.status}`);
  }
  assert.equal((await get(server.port, '/%zz')).status, 400);
  assert.equal((await get(server.port, '/a%00b.js')).status, 400);
});

test('log names keep only safe characters', () => {
  assert.equal(safeName('my-project'), 'my-project');
  assert.equal(safeName('über repo'), 'über repo');
  assert.equal(safeName('a/b\\c?d#e%f\u0001'), 'abcdef');
  assert.equal(safeName('a\u202eb\u2066c\u0085d'), 'abcd');
  assert.equal(safeName('x'.repeat(300)).length, 200);
  // A cut that would split a character drops it whole: the link is still valid UTF-16.
  assert.equal(safeName(`${'a'.repeat(199)}😀`), 'a'.repeat(199));
  assert.doesNotThrow(() => encodeURIComponent(safeName(`${'a'.repeat(199)}😀😀`)));
  assert.equal(safeName('..'), 'repository');
  assert.equal(safeName(''), 'repository');
});
