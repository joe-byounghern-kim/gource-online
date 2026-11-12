// SPDX-License-Identifier: GPL-3.0-or-later

import { execFileSync } from 'node:child_process';
import { readdirSync, readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { defineConfig, type Plugin } from 'vite';
import { playerConfig, playerHeaders } from './src/headers';

// The player's handoff settings are fixed at build time (apps/player/README.md):
//   GOURCE_PLAYER_SENDERS  pages that may hand it a log by postMessage, and embed it
//   GOURCE_PLAYER_SOURCES  origins it may download a `#src=` log from
//   GOURCE_PLAYER_TEST_BUILD=1  a build for the tests: http loopback origins, the placeholder engine (?mock=1) and the sample logs
//   GOURCE_SOURCE_REF, GOURCE_REPO_URL  the exact source the page links
// The dev server always allows both; a release build never does.

/** The files the player serves besides its build: the licence, the engine's font and its notice, and the third-party notices. */
const ASSETS = new Map<string, URL>([
  ['COPYING.txt', new URL('../../COPYING', import.meta.url)],
  ['fonts/FreeSans.ttf', new URL('./assets/fonts/FreeSans.ttf', import.meta.url)],
  ['fonts/README', new URL('./assets/fonts/README', import.meta.url)],
  ['THIRD_PARTY_LICENSES.txt', new URL('./assets/THIRD_PARTY_LICENSES.txt', import.meta.url)],
]);

/** The sample logs, for test builds and the dev server only: `#src=/samples/<name>.log`. */
const SAMPLE_DIR = new URL('./tests/samples/', import.meta.url);
const samples = () => readdirSync(SAMPLE_DIR).filter((name) => name.endsWith('.log')).map((name) => [`samples/${name}`, new URL(name, SAMPLE_DIR)] as const);

/** The exact source of this build, which the page links (GPL-3.0-or-later): an exact tag when the commit has one, else the commit. */
function sourceRef(): string {
  if (process.env.GOURCE_SOURCE_REF) return process.env.GOURCE_SOURCE_REF;
  try {
    return execFileSync('git', ['describe', '--tags', '--exact-match'], { stdio: ['ignore', 'pipe', 'ignore'] }).toString().trim();
  } catch {
    try {
      return execFileSync('git', ['rev-parse', 'HEAD'], { stdio: ['ignore', 'pipe', 'ignore'] }).toString().trim();
    } catch {
      return 'main';
    }
  }
}

export default defineConfig(({ command }) => {
  const test = command === 'serve' || process.env.GOURCE_PLAYER_TEST_BUILD === '1';
  const config = playerConfig({
    senders: process.env.GOURCE_PLAYER_SENDERS,
    sources: process.env.GOURCE_PLAYER_SOURCES,
    loopback: test ? '1' : undefined,
  });
  const files = new Map([...ASSETS, ...(test ? samples() : [])]);
  const plugin: Plugin = {
    name: 'gource-player-files',
    generateBundle() {
      this.emitFile({ type: 'asset', fileName: '_headers', source: playerHeaders(config) });
      for (const [fileName, url] of files) this.emitFile({ type: 'asset', fileName, source: readFileSync(url) });
    },
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const url = files.get((req.url ?? '').split('?')[0]!.slice(1));
        if (!url) return next();
        res.end(readFileSync(url));
      });
    },
    // `vite preview` serves the build with the headers it was built with: its own _headers, as the static host does.
    configurePreviewServer(server) {
      const file = resolve(server.config.root, server.config.build.outDir, '_headers');
      const headers = readFileSync(file, 'utf8').split('\n').flatMap((line) => {
        const match = /^ {2}([^:]+): (.+)$/.exec(line);
        return match ? [[match[1]!, match[2]!] as const] : [];
      });
      server.middlewares.use((_req, res, next) => {
        for (const [name, value] of headers) res.setHeader(name, value);
        next();
      });
    },
  };
  return {
    // Only the files above.
    publicDir: false,
    plugins: [plugin],
    define: {
      __PLAYER_CONFIG__: JSON.stringify(config),
      __PLAYER_MOCK__: JSON.stringify(test),
      __GOURCE_SOURCE_REF__: JSON.stringify(sourceRef()),
      __GOURCE_REPO_URL__: JSON.stringify(process.env.GOURCE_REPO_URL ?? 'https://github.com/joe-byounghern-kim/gource-online'),
    },
    server: { port: 5174 },
    worker: { format: 'es' },
    build: { target: 'es2022', sourcemap: true },
  };
});
