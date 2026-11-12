#!/usr/bin/env node
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Renders the Rust/WebAssembly port at the frames of the C++ reference renders
// (scripts/reference/, target/reference/<scenario>/) so compare.py can put
// them side by side. See docs/web-mvp/PARITY.md.
//
//   node scripts/parity/render.mjs [options] [scenario ...]
//
//   --ref DIR        reference root (default target/reference, or $PARITY_REF)
//   --out DIR        output root (default target/parity/port-<rand-mode>)
//   --rand-mode M    glibc (default: the C++ rand() sequence) or keyed
//   --gpu G          gl (default: ANGLE on the host's OpenGL driver), vulkan or
//                    swiftshader (CPU). Headless Vulkan lost every WebGL
//                    context within a second on the NVIDIA test machine.
//   --every N        render every Nth reference frame only (default 1 = all)
//   --option k=v     extra Gource option, repeatable
//   --tick-offset N  render every frame N ticks later (diagnoses the C++ log
//                    loader race that samples a whole reference run one tick
//                    late, see scripts/reference/README.md)
//
// Every scenario runs in a fresh page: a 1280x720 canvas at device pixel
// ratio 1, the time zone UTC, the options of the scenario's command.txt, the
// whole input.log parsed and loaded before the first tick, then
// GourceApp.step at a fixed dt of 1/60 s up to the reference frame's tick
// count (manifest.json: frame k = 2k + 1 ticks). The canvas is read with
// toDataURL in the same task as the step that drew it.
//
// Needs scripts/build-web.sh (packages/viewer/src/wasm/) and Playwright in apps/player/node_modules.

import { createServer } from 'node:http';
import { mkdirSync, readFileSync, writeFileSync, existsSync, readdirSync, unlinkSync } from 'node:fs';
import { dirname, extname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..');
const { chromium } = await import(join(root, 'apps/player/node_modules/@playwright/test/index.mjs'));

// --- arguments ---
const args = process.argv.slice(2);
const opts = { ref: process.env.PARITY_REF || join(root, 'target/reference'), out: null, randMode: 'glibc', gpu: 'gl', every: 1, tickOffset: 0, extra: [], scenarios: [] };
for (let i = 0; i < args.length; i++) {
  const a = args[i];
  const next = () => args[++i];
  if (a === '--ref') opts.ref = resolve(next());
  else if (a === '--out') opts.out = resolve(next());
  else if (a === '--rand-mode') opts.randMode = next();
  else if (a === '--gpu') opts.gpu = next();
  else if (a === '--tick-offset') opts.tickOffset = Number(next()) | 0;
  else if (a === '--every') opts.every = Math.max(1, Number(next()) | 0);
  else if (a === '--option') {
    const kv = next();
    const eq = kv.indexOf('=');
    opts.extra.push([kv.slice(0, eq), kv.slice(eq + 1)]);
  } else if (a === '-h' || a === '--help') {
    const lines = readFileSync(fileURLToPath(import.meta.url), 'utf8').split('\n').slice(3);
    console.log(lines.slice(0, lines.findIndex((l) => !l.startsWith('//'))).map((l) => l.replace(/^\/\/ ?/, '')).join('\n'));
    process.exit(0);
  } else if (a.startsWith('--')) throw new Error(`unknown flag ${a}`);
  else opts.scenarios.push(a);
}
opts.out ??= join(root, `target/parity/port-${opts.randMode}`);
if (!opts.scenarios.length) {
  opts.scenarios = readdirSync(opts.ref).filter((d) => existsSync(join(opts.ref, d, 'manifest.json'))).sort();
}

const GPU_ARGS = {
  vulkan: ['--use-angle=vulkan', '--enable-features=Vulkan', '--ignore-gpu-blocklist', '--enable-gpu'],
  gl: ['--use-angle=gl', '--ignore-gpu-blocklist', '--enable-gpu'],
  swiftshader: ['--enable-unsafe-swiftshader', '--use-angle=swiftshader'],
};
if (!GPU_ARGS[opts.gpu]) throw new Error(`--gpu must be one of ${Object.keys(GPU_ARGS).join(', ')}`);

/** Gource options from command.txt: every flag except the output/window ones. */
function commandOptions(command) {
  const tokens = command.trim().split(/\s+/).slice(1); // drop "gource"
  const logPath = tokens.pop();
  void logPath;
  const skip = new Map([
    ['--viewport', 1],
    ['--output-framerate', 1],
    ['--output-ppm-stream', 1],
    ['--no-vsync', 0],
  ]);
  const out = [];
  for (let i = 0; i < tokens.length; i++) {
    const t = tokens[i];
    if (!t.startsWith('--')) throw new Error(`unexpected token ${t} in command.txt`);
    if (skip.has(t)) {
      i += skip.get(t);
      continue;
    }
    const name = t.slice(2);
    const v = tokens[i + 1];
    if (v !== undefined && !v.startsWith('--')) {
      out.push([name, v]);
      i++;
    } else {
      out.push([name, 'true']);
    }
  }
  return out;
}

// --- static server: harness page, wasm bindings, font, logs ---
const HARNESS = `<!doctype html><meta charset="utf-8"><title>parity</title>
<style>html,body{margin:0;background:#000}canvas{display:block;width:1280px;height:720px}</style>
<canvas id="c" width="1280" height="720"></canvas>
<script type="module">
import init, { GourceApp, LogParser } from '/wasm/gource_web.js';
window.parity = {
  async start(logUrl, options) {
    const face = new FontFace('GourceSans', 'url(/font/FreeSans.ttf)');
    document.fonts.add(await face.load());
    await init();
    const canvas = document.getElementById('c');
    const app = new GourceApp(canvas);
    app.resize(1280, 720, 1);
    const errors = [];
    for (const [k, v] of options) {
      try { app.set_option(k, v); } catch (e) { errors.push(k + '=' + v + ': ' + e); }
    }
    const bytes = new Uint8Array(await (await fetch(logUrl)).arrayBuffer());
    const parser = new LogParser();
    app.begin_log();
    app.append_batch(parser.push(bytes));
    app.append_batch(parser.finish());
    app.finish_log(bytes.length);
    app.restart();
    app.set_fixed_dt(1 / 60);
    this.app = app;
    this.canvas = canvas;
    this.ticks = 0;
    const gl = canvas.getContext('webgl2');
    this.gl = gl;
    const info = gl && gl.getExtension('WEBGL_debug_renderer_info');
    return { errors, commits: parser.commits(), skipped: parser.skipped_lines(), lost: gl.isContextLost(),
             renderer: String(gl.getParameter(info ? info.UNMASKED_RENDERER_WEBGL : gl.RENDERER)) };
  },
  /** Advance to exactly \`ticks\` ticks and return the frame as a PNG data URL. */
  shot(ticks) {
    const n = ticks - this.ticks;
    if (n < 1) throw new Error('ticks must increase');
    this.app.step(n);
    this.ticks = ticks;
    if (this.gl.isContextLost()) throw new Error('WebGL context lost at tick ' + ticks);
    return { png: this.canvas.toDataURL('image/png'), status: JSON.parse(this.app.status()) };
  },
};
</script>`;

const MIME = { '.js': 'text/javascript', '.wasm': 'application/wasm', '.ttf': 'font/ttf', '.log': 'text/plain' };
const server = createServer((req, res) => {
  const url = new URL(req.url, 'http://x');
  let file = null;
  if (url.pathname === '/favicon.ico') {
    res.writeHead(204);
    return res.end();
  }
  if (url.pathname === '/') {
    res.writeHead(200, { 'content-type': 'text/html' });
    return res.end(HARNESS);
  }
  if (url.pathname.startsWith('/wasm/')) file = join(root, 'packages/viewer/src/wasm', url.pathname.slice(6));
  else if (url.pathname === '/font/FreeSans.ttf') file = join(root, 'data/fonts/FreeSans.ttf');
  else if (url.pathname.startsWith('/log/')) file = join(opts.ref, decodeURIComponent(url.pathname.slice(5)), 'input.log');
  if (!file || file.includes('..') || !existsSync(file)) {
    res.writeHead(404);
    return res.end();
  }
  res.writeHead(200, { 'content-type': MIME[extname(file)] || 'application/octet-stream' });
  res.end(readFileSync(file));
});
await new Promise((r) => server.listen(0, '127.0.0.1', r));
const base = `http://127.0.0.1:${server.address().port}`;

if (!existsSync(join(root, 'packages/viewer/src/wasm/gource_web.js'))) throw new Error('run scripts/build-web.sh first');

const browser = await chromium.launch({ channel: 'chrome', headless: true, args: GPU_ARGS[opts.gpu] });
try {
  for (const scenario of opts.scenarios) {
    const refDir = join(opts.ref, scenario);
    const manifest = JSON.parse(readFileSync(join(refDir, 'manifest.json'), 'utf8'));
    if (manifest.width !== 1280 || manifest.height !== 720) throw new Error(`${scenario}: only 1280x720 is supported`);
    const options = [...commandOptions(readFileSync(join(refDir, 'command.txt'), 'utf8')), ['rand-mode', opts.randMode], ...opts.extra];
    const outDir = join(opts.out, scenario);
    mkdirSync(outDir, { recursive: true });
    for (const f of readdirSync(outDir)) if (/^(frame|sheet)-\d+\.(png|jpg)$/.test(f)) unlinkSync(join(outDir, f));

    const context = await browser.newContext({ viewport: { width: 1280, height: 720 }, deviceScaleFactor: 1, timezoneId: 'UTC', locale: 'en-US' });
    const page = await context.newPage();
    const logs = [];
    page.on('console', (m) => logs.push(`${m.type()}: ${m.text()}`));
    page.on('pageerror', (e) => logs.push(`pageerror: ${e.message}`));
    await page.goto(base + '/');
    await page.waitForFunction(() => window.parity !== undefined);
    const info = await page.evaluate(([log, o]) => window.parity.start(log, o), [`/log/${encodeURIComponent(scenario)}`, options]);
    if (info.errors.length) throw new Error(`${scenario}: options rejected: ${info.errors.join('; ')}`);

    const frames = manifest.frames.filter((f, i) => i % opts.every === 0 || i === manifest.frames.length - 1);
    const statuses = [];
    for (const f of frames) {
      const { png, status } = await page.evaluate((t) => window.parity.shot(t), f.ticks + opts.tickOffset);
      writeFileSync(join(outDir, f.file), Buffer.from(png.slice(png.indexOf(',') + 1), 'base64'));
      statuses.push({ index: f.index, ticks: f.ticks, date: status.date, files: status.files, users: status.users, dirs: status.dirs, draw_calls: status.draw_calls });
    }
    writeFileSync(
      join(outDir, 'port.json'),
      JSON.stringify({ scenario, rand_mode: opts.randMode, gpu: opts.gpu, renderer: info.renderer, options, commits: info.commits, skipped_lines: info.skipped, frames: statuses, console: logs }, null, 2),
    );
    console.log(`${scenario}: ${frames.length} frames, ${info.commits} commits, renderer ${info.renderer}${logs.length ? `, ${logs.length} console messages` : ''}`);
    await context.close();
  }
} finally {
  await browser.close();
  server.close();
}
