#!/usr/bin/env node
// SPDX-License-Identifier: GPL-3.0-or-later
//
// Browser benchmark of the release wasm build in headless Chrome.
//
//   node scripts/perf/browser-bench.mjs --base http://127.0.0.1:18803 \
//        --log /perf/stress.log [--mode fixed|realtime] [--gpu nvidia|gl|swiftshader]
//        [--frames 3000] [--seconds 60] [--finish] [--options 'seconds-per-day=1&hide=date']
//        [--profile out.cpuprofile] [--json out.json] [--viewport 1280x720]
//
// Every mode first loads `<base>/#src=<log>` (a test build, stage-site.sh) and
// reports the load phase:
// time to the first rendered frame (engine created and at least one commit),
// time until the whole log is parsed (status.seekable), worker throughput.
//
// fixed:    `?fixed-dt=1/60`; after the log is complete, runs --frames ticks of
//           window.__gource.step(1) and reads EngineStatus.last_{sim,build,gpu}_ms
//           after each, i.e. the same frames `gource-rs bench` runs natively
//           (sim = update + extract_view). --finish adds a 1x1 readPixels
//           after each step, which waits for Chrome's GPU process to finish
//           the frame (gl.finish() does not block in Chrome), so GPU-side
//           cost shows up as `gpu wait`.
// realtime: normal playback for --seconds; records every animation frame's
//           interval, the per-frame CPU timings, draw calls, vertices, uploaded
//           bytes, and the JS heap / wasm memory over time.
//
// --profile writes a Chrome CPU profile (CDP Profiler, 100 µs sampling) of
// the measured phase and prints the top self-time functions.
//
// Playwright comes from apps/player/node_modules (pnpm install).

import { writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const { chromium } = await import(resolve(here, '../../apps/player/node_modules/@playwright/test/index.mjs'));

function parseArgs(argv) {
  const out = { mode: 'fixed', gpu: 'nvidia', frames: 3000, seconds: 60, viewport: '1280x720', options: '', finish: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const take = () => argv[++i];
    switch (a) {
      case '--base': out.base = take(); break;
      case '--log': out.log = take(); break;
      case '--mode': out.mode = take(); break;
      case '--gpu': out.gpu = take(); break;
      case '--frames': out.frames = Number(take()); break;
      case '--seconds': out.seconds = Number(take()); break;
      case '--viewport': out.viewport = take(); break;
      case '--options': out.options = take(); break;
      case '--profile': out.profile = take(); break;
      case '--json': out.json = take(); break;
      case '--finish': out.finish = true; break;
      default: throw new Error(`unknown argument ${a}`);
    }
  }
  if (!out.base || !out.log) throw new Error('--base and --log are required');
  return out;
}

const GPU_ARGS = {
  nvidia: ['--use-angle=vulkan', '--enable-features=Vulkan', '--ignore-gpu-blocklist', '--enable-gpu'],
  gl: ['--use-angle=gl', '--ignore-gpu-blocklist', '--enable-gpu'],
  swiftshader: ['--enable-unsafe-swiftshader', '--use-angle=swiftshader'],
};

function summary(values) {
  if (values.length === 0) return { n: 0, mean: 0, p50: 0, p95: 0, max: 0 };
  const s = [...values].sort((a, b) => a - b);
  const rank = (p) => s[Math.min(s.length, Math.max(1, Math.ceil((s.length * p) / 100))) - 1];
  const total = s.reduce((a, b) => a + b, 0);
  return { n: s.length, total, mean: total / s.length, p50: rank(50), p95: rank(95), max: s[s.length - 1] };
}

function fmt(name, s) {
  const f = (v) => v.toFixed(4).padStart(10);
  return `${name.padEnd(16)}${f(s.mean)}${f(s.p50)}${f(s.p95)}${f(s.max)}`;
}

function topFunctions(profile, limit = 25) {
  const self = new Map();
  const byId = new Map(profile.nodes.map((n) => [n.id, n]));
  const dt = new Map();
  for (let i = 0; i < profile.samples.length; i++) {
    const id = profile.samples[i];
    dt.set(id, (dt.get(id) ?? 0) + (profile.timeDeltas[i] ?? 0));
  }
  let total = 0;
  for (const [id, us] of dt) {
    const n = byId.get(id);
    const cf = n.callFrame;
    const name = `${cf.functionName || '(anonymous)'} ${cf.url ? cf.url.split('/').pop() : ''}`.trim();
    self.set(name, (self.get(name) ?? 0) + us);
    total += us;
  }
  return [...self.entries()]
    .sort((a, b) => b[1] - a[1])
    .slice(0, limit)
    .map(([name, us]) => ({ name, ms: us / 1000, share: us / total }));
}

const args = parseArgs(process.argv.slice(2));
const [vw, vh] = args.viewport.split('x').map(Number);
const browser = await chromium.launch({
  channel: 'chrome',
  headless: true,
  args: [...GPU_ARGS[args.gpu], '--enable-precise-memory-info'],
});
const context = await browser.newContext({ viewport: { width: vw, height: vh }, deviceScaleFactor: 1 });
const page = await context.newPage();
const consoleErrors = [];
page.on('console', (m) => {
  if (m.type() === 'error') consoleErrors.push(m.text());
});
page.on('pageerror', (e) => consoleErrors.push(String(e)));

const fragment = new URLSearchParams({ src: args.log });
for (const [name, value] of new URLSearchParams(args.options)) fragment.set(`o.${name}`, value);
const query = args.mode === 'fixed' ? `?${new URLSearchParams({ 'fixed-dt': String(1 / 60) })}` : '';
const url = `${args.base}/${query}#${fragment}`;
const result = { url, mode: args.mode, gpu: args.gpu, viewport: args.viewport };

// --- load phase -----------------------------------------------------------
const t0 = Date.now();
await page.goto(url);
await page.waitForFunction(() => window.__gource && window.__gource.status().commits_loaded > 0, null, { timeout: 900_000, polling: 5 });
const tFirst = Date.now() - t0;
await page.waitForFunction(() => window.__gource.status().seekable, null, { timeout: 600_000, polling: 20 });
const tAll = Date.now() - t0;
const info = await page.evaluate(() => {
  const gl = document.querySelector('canvas.gource-canvas').getContext('webgl2');
  const ext = gl.getExtension('WEBGL_debug_renderer_info');
  const s = window.__gource.status();
  const nav = performance.getEntriesByType('navigation')[0];
  const logEntry = performance.getEntriesByType('resource').find((r) => r.name.includes('.log'));
  return {
    renderer: (ext && gl.getParameter(ext.UNMASKED_RENDERER_WEBGL)) || gl.getParameter(gl.RENDERER),
    status: s,
    domContentLoaded: nav ? nav.domContentLoadedEventEnd : null,
    logTransferBytes: logEntry ? logEntry.transferSize : null,
    jsHeap: performance.memory ? performance.memory.usedJSHeapSize : null,
    wasmMemory: window.__gource.app.memory_bytes ? window.__gource.app.memory_bytes() : null,
  };
});
result.renderer = info.renderer;
result.load = {
  first_frame_ms: tFirst,
  log_complete_ms: tAll,
  commits: info.status.commits_loaded,
  canvas: [info.status.width, info.status.height],
  js_heap_bytes: info.jsHeap,
  wasm_memory_bytes: info.wasmMemory,
};
console.log(`renderer    ${info.renderer}`);
console.log(`canvas      ${info.status.width}x${info.status.height}`);
console.log(`load        first frame ${tFirst} ms, log complete ${tAll} ms, ${info.status.commits_loaded} commits`);

let cdp = null;
if (args.profile) {
  cdp = await context.newCDPSession(page);
  await cdp.send('Profiler.enable');
  await cdp.send('Profiler.setSamplingInterval', { interval: 100 });
  await cdp.send('Profiler.start');
}

// --- measured phase -------------------------------------------------------
if (args.mode === 'fixed') {
  const r = await page.evaluate(
    async ({ frames, finish }) => {
      const g = window.__gource;
      const gl = document.querySelector('canvas.gource-canvas').getContext('webgl2');
      const sim = new Float64Array(frames);
      const build = new Float64Array(frames);
      const gpu = new Float64Array(frames);
      const call = new Float64Array(frames);
      const up = new Float64Array(frames);
      const fin = new Float64Array(frames);
      const px = new Uint8Array(4);
      const start = g.status();
      let peak = { files: 0, users: 0, dirs: 0, vertices: 0, uploaded_bytes: 0, draw_calls: 0 };
      const heap0 = performance.memory?.usedJSHeapSize ?? 0;
      const wall0 = performance.now();
      // Builds without last_*_ms (before they were added): recover the tick's
      // value from the moving averages just before and after it (weight 0.1).
      const hasLast = 'last_sim_ms' in start;
      for (let i = 0; i < frames; i++) {
        const before = hasLast ? null : JSON.parse(g.app.status());
        const a = performance.now();
        g.app.step(1);
        const b = performance.now();
        if (finish) gl.readPixels(0, 0, 1, 1, gl.RGBA, gl.UNSIGNED_BYTE, px);
        const c = performance.now();
        const s = JSON.parse(g.app.status());
        if (hasLast) {
          sim[i] = s.last_sim_ms;
          build[i] = s.last_build_ms;
          gpu[i] = s.last_gpu_ms;
        } else {
          const back = (k) => (before.ticks === 0 && i === 0 && before[k] === 0 ? s[k] : before[k] + (s[k] - before[k]) / 0.1);
          sim[i] = back('sim_ms');
          build[i] = back('build_ms');
          gpu[i] = back('gpu_ms');
        }
        call[i] = b - a;
        up[i] = s.uploaded_bytes;
        fin[i] = c - b;
        for (const k of Object.keys(peak)) peak[k] = Math.max(peak[k], s[k]);
        // Let the browser breathe (GC, compositor) like a frame loop would.
        if (i % 120 === 119) await new Promise((r) => setTimeout(r, 0));
      }
      const wall = performance.now() - wall0;
      const end = g.status();
      return {
        start_ticks: start.ticks,
        wall,
        sim: [...sim],
        build: [...build],
        gpu: [...gpu],
        call: [...call],
        finish: [...fin],
        uploads: [...up],
        peak,
        end,
        heap_growth: (performance.memory?.usedJSHeapSize ?? 0) - heap0,
        wasm_memory: g.app.memory_bytes ? g.app.memory_bytes() : null,
      };
    },
    { frames: args.frames, finish: args.finish },
  );
  const frame = r.sim.map((v, i) => v + r.build[i] + r.gpu[i]);
  // Glyph atlas uploads are whole textures (1024² or 2048² RGBA): a frame
  // uploading at least 4 MiB more than the frame before carried one.
  const atlasFrames = r.uploads.filter((v, i) => i > 0 && v - r.uploads[i - 1] >= 4 * 1024 * 1024 - 65536).length;
  result.fixed = {
    frames: args.frames,
    start_ticks: r.start_ticks,
    wall_ms: r.wall,
    sim: summary(r.sim),
    build: summary(r.build),
    gpu_submit: summary(r.gpu),
    frame: summary(frame),
    step_call: summary(r.call),
    gpu_wait: summary(r.finish),
    atlas_upload_frames: atlasFrames,
    series: { sim: r.sim, build: r.build, gpu: r.gpu, wait: r.finish, uploaded_bytes: r.uploads },
    uploaded_bytes: summary(r.uploads),
    peak: r.peak,
    end: { time: r.end.time, commits_played: r.end.commits_played, files: r.end.files, users: r.end.users, dirs: r.end.dirs },
    js_heap_growth_bytes: r.heap_growth,
    wasm_memory_bytes: r.wasm_memory,
  };
  console.log(`run         ${args.frames} ticks (from tick ${r.start_ticks}), ${r.wall.toFixed(0)} ms wall`);
  console.log(`${'phase'.padEnd(16)}${'mean'.padStart(10)}${'p50'.padStart(10)}${'p95'.padStart(10)}${'max'.padStart(10)}   (ms)`);
  console.log(fmt('sim', result.fixed.sim));
  console.log(fmt('build', result.fixed.build));
  console.log(fmt('gpu submit', result.fixed.gpu_submit));
  console.log(fmt('frame', result.fixed.frame));
  console.log(fmt('step() call', result.fixed.step_call));
  if (args.finish) console.log(fmt('gpu wait', result.fixed.gpu_wait));
  console.log(`last        time ${r.end.time}, ${r.end.commits_played} commits; ${r.end.files} files, ${r.end.users} users, ${r.end.dirs} dirs`);
  console.log(`peak        ${JSON.stringify(r.peak)}`);
  console.log(`uploads     mean ${(result.fixed.uploaded_bytes.mean / 1048576).toFixed(2)} MiB/frame, ${atlasFrames} frames with a glyph atlas upload`);
  console.log(`memory      js heap growth ${(r.heap_growth / 1024).toFixed(0)} KiB, wasm memory ${(r.wasm_memory / 1048576).toFixed(1)} MiB`);
} else if (args.mode === 'realtime') {
  const r = await page.evaluate(async (seconds) => {
    const g = window.__gource;
    const intervals = [];
    const cpu = [];
    const samples = [];
    let last = null;
    let prev = null;
    let peak = { files: 0, users: 0, dirs: 0, vertices: 0, uploaded_bytes: 0, draw_calls: 0 };
    const t0 = performance.now();
    const heap0 = performance.memory?.usedJSHeapSize ?? 0;
    const wasm0 = g.app.memory_bytes ? g.app.memory_bytes() : 0;
    await new Promise((done) => {
      const tick = (now) => {
        if (last !== null) intervals.push(now - last);
        last = now;
        const s = JSON.parse(g.app.status());
        if ('last_sim_ms' in s) cpu.push(s.last_sim_ms + s.last_build_ms + s.last_gpu_ms);
        else if (prev && s.ticks === prev.ticks + 1) cpu.push(prev.frame_ms + (s.frame_ms - prev.frame_ms) / 0.1);
        prev = s;
        for (const k of Object.keys(peak)) peak[k] = Math.max(peak[k], s[k]);
        if (now - t0 >= seconds * 1000) return done();
        requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
      const sample = () => {
        if (performance.now() - t0 >= seconds * 1000) return;
        const s = g.status();
        samples.push({
          t: performance.now() - t0,
          fps: s.fps,
          frame_ms: s.frame_ms,
          sim_ms: s.sim_ms,
          build_ms: s.build_ms,
          gpu_ms: s.gpu_ms,
          draw_calls: s.draw_calls,
          vertices: s.vertices,
          uploaded_bytes: s.uploaded_bytes,
          files: s.files,
          dirs: s.dirs,
          users: s.users,
          js_heap: performance.memory?.usedJSHeapSize ?? 0,
          wasm: g.app.memory_bytes ? g.app.memory_bytes() : 0,
        });
        setTimeout(sample, 1000);
      };
      sample();
    });
    const end = g.status();
    return {
      intervals,
      cpu,
      samples,
      peak,
      end,
      heap_growth: (performance.memory?.usedJSHeapSize ?? 0) - heap0,
      wasm_growth: (g.app.memory_bytes ? g.app.memory_bytes() : 0) - wasm0,
    };
  }, args.seconds);
  const iv = summary(r.intervals);
  const long = r.intervals.filter((v) => v > 1000 / 60 + 4).length;
  const longCpu = r.cpu.filter((v) => v > 1000 / 60).length;
  const elapsed = r.intervals.reduce((a, b) => a + b, 0);
  result.realtime = {
    seconds: args.seconds,
    frames: r.intervals.length,
    fps: (r.intervals.length * 1000) / elapsed,
    interval: iv,
    frames_over_20_7ms: long,
    cpu_frame: summary(r.cpu),
    cpu_frames_over_16_7ms: longCpu,
    peak: r.peak,
    end: { time: r.end.time, commits_played: r.end.commits_played, files: r.end.files, users: r.end.users, dirs: r.end.dirs, draw_calls: r.end.draw_calls },
    js_heap_growth_bytes: r.heap_growth,
    wasm_memory_growth_bytes: r.wasm_growth,
    samples: r.samples,
  };
  console.log(`run         ${args.seconds} s real time, ${r.intervals.length} frames, ${result.realtime.fps.toFixed(2)} fps`);
  console.log(fmt('rAF interval', iv));
  console.log(fmt('cpu frame', result.realtime.cpu_frame));
  console.log(`long        ${long} intervals > 20.7 ms (dropped frames), ${longCpu} frames with CPU > 16.7 ms`);
  console.log(`last        ${r.end.commits_played} commits; ${r.end.files} files, ${r.end.users} users, ${r.end.dirs} dirs, ${r.end.draw_calls} draw calls`);
  console.log(`peak        ${JSON.stringify(r.peak)}`);
  console.log(`memory      js heap growth ${(r.heap_growth / 1024).toFixed(0)} KiB, wasm memory growth ${(r.wasm_growth / 1048576).toFixed(1)} MiB`);
} else {
  throw new Error(`unknown mode ${args.mode}`);
}

if (cdp) {
  const { profile } = await cdp.send('Profiler.stop');
  writeFileSync(args.profile, JSON.stringify(profile));
  result.profile_top = topFunctions(profile);
  console.log('profile     top self time:');
  for (const f of result.profile_top) console.log(`  ${(f.share * 100).toFixed(1).padStart(5)}%  ${f.ms.toFixed(1).padStart(9)} ms  ${f.name}`);
}
if (consoleErrors.length) {
  result.console_errors = consoleErrors;
  console.log(`console errors: ${consoleErrors.slice(0, 5).join(' | ')}`);
}
if (args.json) writeFileSync(args.json, JSON.stringify(result, null, 2));
await browser.close();
