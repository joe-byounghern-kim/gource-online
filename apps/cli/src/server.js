// SPDX-License-Identifier: GPL-3.0-or-later
// @ts-check

// The local server: the bundled player (apps/player) and one repository's
// history, on 127.0.0.1 only.
//
// - The history is served at an unguessable path (`/<token>/<name>.gitlog`),
//   so other accounts on a shared machine cannot read it from the port.
// - Requests must be addressed to this port on 127.0.0.1 or localhost
//   (the Host header): a website that points its own name at 127.0.0.1
//   (DNS rebinding) gets nothing.
// - The page gets the player's security headers (apps/player/src/headers.ts),
//   with `connect-src 'self'`: it can talk to nothing but this server, and no
//   other page may embed it or hand it a log.
// - The player opens the history by its own link, `#src=<path>&title=<name>`
//   (the handoff protocol): the fragment never reaches a server.

import { randomBytes, timingSafeEqual } from 'node:crypto';
import { createReadStream, readFileSync, statSync } from 'node:fs';
import { createServer } from 'node:http';
import { extname, join, resolve, sep } from 'node:path';

export const CSP =
  "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; style-src 'self'; " +
  "img-src 'self' data: blob:; media-src 'self' blob:; font-src 'self'; " +
  "connect-src 'self'; worker-src 'self'; manifest-src 'self'; base-uri 'none'; " +
  "form-action 'none'; frame-ancestors 'none'; object-src 'none'";

const SECURITY_HEADERS = {
  'Content-Security-Policy': CSP,
  'Referrer-Policy': 'no-referrer',
  'X-Content-Type-Options': 'nosniff',
  'Cross-Origin-Opener-Policy': 'same-origin',
  'Cross-Origin-Resource-Policy': 'same-origin',
  'Permissions-Policy': 'camera=(), microphone=(), geolocation=(), payment=()',
};

/** @type {Record<string, string>} */
const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.mjs': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.wasm': 'application/wasm',
  '.json': 'application/json',
  '.svg': 'image/svg+xml',
  '.png': 'image/png',
  '.ico': 'image/x-icon',
  '.ttf': 'font/ttf',
  '.woff2': 'font/woff2',
  '.txt': 'text/plain; charset=utf-8',
  '.log': 'text/plain; charset=utf-8',
};


/**
 * @typedef {object} ServeOptions
 * @property {string} dist     the built player (apps/player/dist)
 * @property {Buffer[]} log    the history, as read from git
 * @property {string} name     the repository's folder name (the log's file name)
 * @property {number} [port]   0 or absent: any free port
 */

/**
 * @typedef {object} LocalServer
 * @property {number} port
 * @property {string} origin   `http://127.0.0.1:<port>`
 * @property {string} logPath  the history's path, with its token
 * @property {string} url      the page that plays the history
 * @property {() => Promise<void>} close
 */

/**
 * @param {ServeOptions} options
 * @returns {Promise<LocalServer>}
 */
export async function startServer({ dist, log, name, port = 0 }) {
  const root = resolve(dist);
  const index = readFileSync(join(root, 'index.html'), 'utf8');
  const token = randomBytes(16).toString('hex');
  const logName = `${safeName(name)}.gitlog`;
  const logPath = `/${token}/${encodeURIComponent(logName)}`;
  const logBytes = log.reduce((n, c) => n + c.length, 0);
  let bound = 0;

  const server = createServer((req, res) => {
    for (const [k, v] of Object.entries(SECURITY_HEADERS)) res.setHeader(k, v);
    const host = req.headers.host ?? '';
    if (host !== `127.0.0.1:${bound}` && host !== `localhost:${bound}`) {
      send(res, 421, 'text/plain; charset=utf-8', 'This server only answers requests for its own address.\n');
      return;
    }
    if (req.method !== 'GET' && req.method !== 'HEAD') {
      res.setHeader('Allow', 'GET, HEAD');
      send(res, 405, 'text/plain; charset=utf-8', 'Method not allowed.\n');
      return;
    }
    const head = req.method === 'HEAD';
    let path;
    try {
      path = decodeURIComponent(new URL(req.url ?? '/', 'http://127.0.0.1').pathname);
    } catch {
      send(res, 400, 'text/plain; charset=utf-8', 'Bad request.\n');
      return;
    }
    if (path.includes('\0')) {
      send(res, 400, 'text/plain; charset=utf-8', 'Bad request.\n');
      return;
    }

    if (isLogPath(path, token, logName)) {
      res.writeHead(200, {
        'Content-Type': 'text/plain; charset=utf-8',
        'Content-Length': String(logBytes),
        'Cache-Control': 'no-store',
      });
      if (head) {
        res.end();
        return;
      }
      writeChunks(res, log);
      return;
    }
    if (path === '/api' || path.startsWith('/api/') || path === '/_headers') {
      // The player calls no service; its static host's header file is not a page.
      send(res, 404, 'text/plain; charset=utf-8', 'Not found.\n', head);
      return;
    }

    const file = resolve(root, `.${path}`);
    const inside = file === root || file.startsWith(root + sep);
    let stat = null;
    if (inside) {
      try {
        stat = statSync(file);
      } catch {
        stat = null;
      }
    }
    if (!stat || !stat.isFile()) {
      // The page itself is `/`; any other missing path with an extension is
      // a missing asset, not a page.
      if (inside && (stat?.isDirectory() || extname(path) === '')) {
        send(res, 200, 'text/html; charset=utf-8', index, head, 'no-store');
      } else {
        send(res, 404, 'text/plain; charset=utf-8', 'Not found.\n', head);
      }
      return;
    }
    if (file === join(root, 'index.html')) {
      send(res, 200, 'text/html; charset=utf-8', index, head, 'no-store');
      return;
    }
    res.writeHead(200, {
      'Content-Type': TYPES[extname(file).toLowerCase()] ?? 'application/octet-stream',
      'Content-Length': String(stat.size),
      'Cache-Control': path.startsWith('/assets/') ? 'public, max-age=31536000, immutable' : 'no-cache',
    });
    if (head) {
      res.end();
      return;
    }
    createReadStream(file).pipe(res);
  });

  await new Promise((done, fail) => {
    server.once('error', fail);
    server.listen(port, '127.0.0.1', () => {
      server.off('error', fail);
      done(undefined);
    });
  });
  const address = server.address();
  bound = typeof address === 'object' && address ? address.port : port;
  const origin = `http://127.0.0.1:${bound}`;
  return {
    port: bound,
    origin,
    logPath,
    url: `${origin}/#${new URLSearchParams({ src: logPath, title: safeName(name) })}`,
    close: () =>
      new Promise((done) => {
        server.closeAllConnections();
        server.close(() => done(undefined));
      }),
  };
}

/**
 * Whether `path` is `/<token>/<logName>`, comparing the token in constant time.
 * @param {string} path
 * @param {string} token
 * @param {string} logName
 */
function isLogPath(path, token, logName) {
  const slash = path.indexOf('/', 1);
  if (slash < 0 || path.slice(slash + 1) !== logName) return false;
  const given = Buffer.from(path.slice(1, slash));
  const want = Buffer.from(token);
  return given.length === want.length && timingSafeEqual(given, want);
}

/**
 * The log's file name and the player's title: the folder name without
 * characters that would need escaping in a path, or that the handoff protocol
 * refuses in a title (control and bidirectional-override characters), and at
 * most 200 UTF-16 units, as the protocol counts them, never half a character.
 * @param {string} name
 */
export function safeName(name) {
  const clean = name.replace(/[\u0000-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069/\\?#%]/g, '').trim().slice(0, 200).replace(/[\uD800-\uDBFF]$/, '').trim();
  return clean === '' || clean === '.' || clean === '..' ? 'repository' : clean;
}

/**
 * @param {import('node:http').ServerResponse} res
 * @param {number} status
 * @param {string} type
 * @param {string} body
 * @param {boolean} [head]
 * @param {string} [cache]
 */
function send(res, status, type, body, head = false, cache) {
  const bytes = Buffer.from(body);
  /** @type {Record<string, string>} */
  const headers = { 'Content-Type': type, 'Content-Length': String(bytes.length) };
  if (cache) headers['Cache-Control'] = cache;
  res.writeHead(status, headers);
  res.end(head ? undefined : bytes);
}

/**
 * Writes the chunks with back-pressure.
 * @param {import('node:http').ServerResponse} res
 * @param {Buffer[]} chunks
 */
function writeChunks(res, chunks) {
  let i = 0;
  const next = () => {
    while (i < chunks.length) {
      if (!res.write(chunks[i++])) {
        res.once('drain', next);
        return;
      }
    }
    res.end();
  };
  res.on('close', () => {
    i = chunks.length;
  });
  next();
}
