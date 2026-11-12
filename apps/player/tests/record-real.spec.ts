// SPDX-License-Identifier: GPL-3.0-or-later

// F3 recording with the real engine (E2E_REAL=1): the wasm recorder, Chrome's
// WebCodecs H.264 encoder, gource-mux and the in-memory (Blob) sink, against
// a static test build (GOURCE_PLAYER_TEST_BUILD=1 VITE_E2E_HOOKS=1 scripts/serve.sh; the
// hooks need `?fixed-dt`). Headless Chrome runs with the GPU flags of the M0
// spike (`--use-angle=gl`); E2E_GPU=0 keeps playwright.config.ts's
// SwiftShader instead (slow: short clips only).
//
// Each run checks the file by itself, in the page: walks the MP4 boxes,
// decodes every sample with VideoDecoder, compares decoded frames with the
// recorder's own canvas at the same index, and loads it in <video>. The
// numbers (throughput, errors, sizes) are printed and attached; decoded
// frames are saved as PNG (E2E_RECORD_PNG_DIR, default test-results/record).
//
// E2E_RECORD_LOG=/samples/<name>.log adds runs over a larger log (copy it
// into apps/player/dist/samples first) at 720p, 1080p and 4K.

import { mkdirSync, writeFileSync } from 'node:fs';
import { join } from 'node:path';

import { expect, test, type Page, type TestInfo } from '@playwright/test';
import { realLog } from './mock-engine';

test.skip(process.env.E2E_REAL !== '1', 'set E2E_REAL=1 with a test build served by scripts/serve.sh');

if (process.env.E2E_GPU !== '0') {
  test.use({ launchOptions: { args: ['--use-angle=gl', '--ignore-gpu-blocklist', '--enable-gpu'] } });
}

const PNG_DIR = process.env.E2E_RECORD_PNG_DIR ?? 'test-results/record';
const BIG_LOG = process.env.E2E_RECORD_LOG;

/** M0 spike thresholds (recording.md "REC-48 thresholds"). */
const CAPTURE_EXACT = 0;
const FRAME_MAE_1080 = 7.7;

interface RunSpec {
  name: string;
  log: string;
  size: string;
  fps?: string;
  spd?: string;
  maxSeconds?: string;
  exact?: boolean;
  extra?: string;
  /** `record-layout` (the memory sink writes faststart-deferred unless fragmented is chosen). */
  layout?: 'fragmented' | 'faststart';
  /** `record-metadata` (`?log=` sources default to none). */
  metadata?: 'embed' | 'none';
}

interface Analysis {
  bytes: number;
  brands: string;
  layout: string;
  samples: number;
  keys: number;
  width: number;
  height: number;
  avcProfile: number;
  avcLevel: number;
  colr: string;
  mvhdSeconds: number;
  mdhdSeconds: number;
  sumDurationsSeconds: number;
  ptsMonotonic: boolean;
  decoded: number;
  decodeError: string;
  displayWidth: number;
  displayHeight: number;
  compare: Array<{ index: number; mae: number; p99: number; max: number; png: string }>;
  videoDuration: number;
  videoWidth: number;
  videoHeight: number;
  seekReadyState: number;
  videoError: string;
  /** The embedded recipe (`moov/udta/meta/ilst/----` data), if any. */
  recipe: string | null;
  comment: string | null;
}

async function openRun(page: Page, spec: RunSpec): Promise<void> {
  page.on('pageerror', (e) => console.log(`[${spec.name}] pageerror: ${e.message}`));
  await page.goto(realLog(spec.log, spec.extra ?? '', 'fixed-dt=0.016666667'));
  await expect(page.locator('[data-screen="viewer"]')).toBeVisible();
  await expect(page.locator('.record-btn')).toHaveAttribute('aria-disabled', 'false', { timeout: 120_000 });
}

/** Opens the dialog, sets the fields, optionally runs the exact dry run; returns the plan. */
async function configure(page: Page, spec: RunSpec): Promise<{ frames: number; upper: number; fps: number; width: number; height: number }> {
  await page.locator('.record-btn').click();
  const dialog = page.getByRole('dialog', { name: 'Record video' });
  await expect(dialog).toBeVisible();
  await dialog.locator(`input[name="rec-size"][value="${spec.size}"]`).check();
  await dialog.locator(`input[name="rec-fps"][value="${spec.fps ?? '60'}"]`).check();
  if (spec.spd) await dialog.locator('#rec-in-spd').fill(spec.spd);
  if (spec.maxSeconds) await dialog.locator('#rec-in-max').fill(spec.maxSeconds);
  if (spec.layout || spec.metadata) await dialog.locator('.rec-details').evaluate((d) => ((d as HTMLDetailsElement).open = true));
  if (spec.layout) await dialog.locator('#rec-sel-layout').selectOption(spec.layout);
  if (spec.metadata) await dialog.locator('#rec-sel-metadata').selectOption(spec.metadata);
  await expect(dialog.locator('.rec-encoder')).toContainText('H.264', { timeout: 30_000 });
  if (spec.exact) {
    await dialog.getByRole('button', { name: 'Calculate exact length' }).click();
    await expect(dialog.locator('.rec-estimate')).toContainText('Exactly', { timeout: 300_000 });
  }
  await expect(dialog.locator('.rec-estimate')).not.toContainText('Fix');
  const plan = await page.evaluate(() => window.__record!.plan() as { fps: number; width: number; height: number; estimate: { frames: number | null; upper_frames: number }; max_frames: number | null });
  return { frames: plan.estimate.frames ?? plan.max_frames ?? -1, upper: plan.estimate.upper_frames, fps: plan.fps, width: plan.width, height: plan.height };
}

async function record(page: Page, captureFrames: number[]): Promise<{ frames: number; timing: Record<string, number> }> {
  await page.evaluate((frames) => {
    window.__recordTest!.sink = 'blob';
    window.__recordTest!.captureFrames = frames;
  }, captureFrames);
  await page.locator('.rec-start').click();
  const confirm = page.locator('.rec-confirm');
  await expect(page.locator('.rec-progress').or(confirm)).toBeVisible();
  if (await confirm.isVisible()) await confirm.click();
  await expect.poll(() => page.evaluate(() => window.__record!.state()), { timeout: 14 * 60_000, intervals: [1000] }).toMatch(/done|failed|cancelled/);
  const state = await page.evaluate(() => ({ state: window.__record!.state(), error: window.__record!.error() }));
  expect(state.error, JSON.stringify(state.error)).toBeNull();
  expect(state.state).toBe('done');
  return page.evaluate(() => ({ frames: window.__record!.frames(), timing: window.__record!.timing() }));
}

/** Walks the file's boxes, decodes it and compares frames; all in the page. */
function analyse(page: Page, compareFrames: number[]): Promise<Analysis> {
  return page.evaluate(async (want) => {
    const blob = window.__record!.output()!;
    const buf = new Uint8Array(await blob.arrayBuffer());
    const dv = new DataView(buf.buffer);
    const u32 = (p: number) => dv.getUint32(p);
    const u64 = (p: number) => dv.getUint32(p) * 2 ** 32 + dv.getUint32(p + 4);
    const type = (p: number) => String.fromCharCode(buf[p]!, buf[p + 1]!, buf[p + 2]!, buf[p + 3]!);
    interface Box {
      type: string;
      start: number;
      body: number;
      end: number;
    }
    const boxes = (start: number, end: number): Box[] => {
      const out: Box[] = [];
      let p = start;
      while (p + 8 <= end) {
        let size = u32(p);
        let body = p + 8;
        if (size === 1) {
          size = u64(p + 8);
          body = p + 16;
        } else if (size === 0) size = end - p;
        if (size < 8 || p + size > end) throw new Error(`bad box ${type(p + 4)} at ${p}`);
        out.push({ type: type(p + 4), start: p, body, end: p + size });
        p += size;
      }
      return out;
    };
    const child = (b: Box, t: string, skip = 0) => boxes(b.body + skip, b.end).find((x) => x.type === t);
    const path = (b: Box, ...ts: string[]) => {
      let cur: Box | undefined = b;
      for (const t of ts) cur = cur && child(cur, t);
      return cur;
    };

    const top = boxes(0, buf.byteLength);
    const brands = top[0]?.type === 'ftyp' ? type(top[0].body) : '';
    const moov = top.find((b) => b.type === 'moov')!;
    const mvhd = child(moov, 'mvhd')!;
    const v1 = buf[mvhd.body] === 1;
    const mvTs = u32(mvhd.body + (v1 ? 20 : 12));
    const mvDur = v1 ? u64(mvhd.body + 24) : u32(mvhd.body + 16);
    const trak = child(moov, 'trak')!;
    const mdhd = path(trak, 'mdia', 'mdhd')!;
    const m1 = buf[mdhd.body] === 1;
    const mdTs = u32(mdhd.body + (m1 ? 20 : 12));
    const mdDur = m1 ? u64(mdhd.body + 24) : u32(mdhd.body + 16);
    const stbl = path(trak, 'mdia', 'minf', 'stbl')!;
    const stsd = child(stbl, 'stsd')!;
    const avc1 = boxes(stsd.body + 8, stsd.end)[0]!;
    // VisualSampleEntry: 78 bytes before the child boxes.
    const entry = boxes(avc1.body + 78, avc1.end);
    const avcC = entry.find((b) => b.type === 'avcC')!;
    const colrBox = entry.find((b) => b.type === 'colr');
    const colr = colrBox
      ? `${type(colrBox.body)} ${dv.getUint16(colrBox.body + 4)}/${dv.getUint16(colrBox.body + 6)}/${dv.getUint16(colrBox.body + 8)} ${(buf[colrBox.body + 10]! & 0x80) !== 0 ? 'full' : 'limited'}`
      : 'none';
    const avccBytes = buf.slice(avcC.body, avcC.end);
    const width = dv.getUint16(avc1.body + 24);
    const height = dv.getUint16(avc1.body + 26);

    // Embedded recipe: the `data` box after the `name` "recipe" of the `----` item.
    const text = (a: number, b: number) => new TextDecoder().decode(buf.subarray(a, b));
    let recipe: string | null = null;
    let comment: string | null = null;
    const udta = child(moov, 'udta');
    const meta = udta && child(udta, 'meta');
    const ilst = meta && child(meta, 'ilst', 4);
    if (ilst) {
      for (const item of boxes(ilst.body, ilst.end)) {
        const inner = boxes(item.body, item.end);
        const data = inner.find((b) => b.type === 'data');
        if (!data) continue;
        const value = text(data.body + 8, data.end);
        if (item.type === '----' && inner.some((b) => b.type === 'name' && text(b.body + 4, b.end) === 'recipe')) recipe = value;
        if (item.type === '\u00a9cmt') comment = value;
      }
    }

    // Samples: flat tables (faststart) or fragments.
    const samples: Array<{ offset: number; size: number; key: boolean; dur: number }> = [];
    let layout = 'flat';
    const stsz = child(stbl, 'stsz');
    const sampleCount = stsz ? u32(stsz.body + 8) : 0;
    if (sampleCount > 0 && stsz) {
      const fixed = u32(stsz.body + 4);
      const sizes = Array.from({ length: sampleCount }, (_, i) => fixed || u32(stsz.body + 12 + 4 * i));
      const stco = child(stbl, 'stco');
      const co64 = child(stbl, 'co64');
      const chunks: number[] = [];
      if (stco) for (let i = 0; i < u32(stco.body + 4); i++) chunks.push(u32(stco.body + 8 + 4 * i));
      if (co64) for (let i = 0; i < u32(co64.body + 4); i++) chunks.push(u64(co64.body + 8 + 8 * i));
      const stsc = child(stbl, 'stsc')!;
      const runs = Array.from({ length: u32(stsc.body + 4) }, (_, i) => ({ first: u32(stsc.body + 8 + 12 * i), per: u32(stsc.body + 12 + 12 * i) }));
      const stss = child(stbl, 'stss');
      const sync = new Set<number>();
      if (stss) for (let i = 0; i < u32(stss.body + 4); i++) sync.add(u32(stss.body + 8 + 4 * i) - 1);
      const stts = child(stbl, 'stts')!;
      const durs: number[] = [];
      for (let i = 0; i < u32(stts.body + 4); i++) {
        const n = u32(stts.body + 8 + 8 * i);
        const d = u32(stts.body + 12 + 8 * i);
        for (let k = 0; k < n; k++) durs.push(d);
      }
      let s = 0;
      for (let c = 0; c < chunks.length; c++) {
        let per = 0;
        for (const r of runs) if (r.first - 1 <= c) per = r.per;
        let off = chunks[c]!;
        for (let k = 0; k < per && s < sampleCount; k++, s++) {
          samples.push({ offset: off, size: sizes[s]!, key: stss ? sync.has(s) : true, dur: durs[s] ?? 0 });
          off += sizes[s]!;
        }
      }
    } else {
      layout = 'fragmented';
      const trex = path(moov, 'mvex', 'trex');
      const defDur = trex ? u32(trex.body + 12) : 0;
      const defSize = trex ? u32(trex.body + 16) : 0;
      const defFlags = trex ? u32(trex.body + 20) : 0;
      for (const moof of top.filter((b) => b.type === 'moof')) {
        for (const traf of boxes(moof.body, moof.end).filter((b) => b.type === 'traf')) {
          const tfhd = child(traf, 'tfhd')!;
          const tf = u32(tfhd.body) & 0xffffff;
          let p = tfhd.body + 8;
          let base = moof.start;
          if (tf & 1) {
            base = u64(p);
            p += 8;
          }
          if (tf & 2) p += 4;
          let dDur = defDur;
          let dSize = defSize;
          let dFlags = defFlags;
          if (tf & 8) {
            dDur = u32(p);
            p += 4;
          }
          if (tf & 0x10) {
            dSize = u32(p);
            p += 4;
          }
          if (tf & 0x20) dFlags = u32(p);
          for (const trun of boxes(traf.body, traf.end).filter((b) => b.type === 'trun')) {
            const fl = u32(trun.body) & 0xffffff;
            const n = u32(trun.body + 4);
            let q = trun.body + 8;
            let off = base;
            if (fl & 1) {
              off = base + dv.getInt32(q);
              q += 4;
            }
            let first = -1;
            if (fl & 4) {
              first = u32(q);
              q += 4;
            }
            for (let i = 0; i < n; i++) {
              let dur = dDur;
              let size = dSize;
              let flags = i === 0 && first >= 0 ? first : dFlags;
              if (fl & 0x100) {
                dur = u32(q);
                q += 4;
              }
              if (fl & 0x200) {
                size = u32(q);
                q += 4;
              }
              if (fl & 0x400) {
                flags = u32(q);
                q += 4;
              }
              if (fl & 0x800) q += 4;
              samples.push({ offset: off, size, key: (flags & 0x10000) === 0, dur });
              off += size;
            }
          }
        }
      }
    }

    // Decode every sample.
    const toCompare = new Set(want);
    const decodedFrames = new Map<number, ImageData>();
    const pngs = new Map<number, string>();
    let decoded = 0;
    let decodeError = '';
    let displayWidth = 0;
    let displayHeight = 0;
    const canvas = document.createElement('canvas');
    const ctx = canvas.getContext('2d', { willReadFrequently: true })!;
    const decoder = new VideoDecoder({
      output: (f) => {
        const i = decoded++;
        displayWidth = f.displayWidth;
        displayHeight = f.displayHeight;
        if (toCompare.has(i)) {
          canvas.width = f.displayWidth;
          canvas.height = f.displayHeight;
          ctx.drawImage(f, 0, 0);
          decodedFrames.set(i, ctx.getImageData(0, 0, canvas.width, canvas.height));
          pngs.set(i, canvas.toDataURL('image/png'));
        }
        f.close();
      },
      error: (e) => (decodeError = `${e.name}: ${e.message}`),
    });
    const profile = avccBytes[1]!;
    const level = avccBytes[3]!;
    const hex = (n: number) => n.toString(16).padStart(2, '0');
    decoder.configure({ codec: `avc1.${hex(profile)}${hex(avccBytes[2]!)}${hex(level)}`, description: avccBytes, optimizeForLatency: false });
    let pts = 0;
    let monotonic = true;
    let lastPts = -1;
    for (const s of samples) {
      decoder.decode(new EncodedVideoChunk({ type: s.key ? 'key' : 'delta', timestamp: Math.round((pts * 1e6) / mdTs), duration: Math.round((s.dur * 1e6) / mdTs), data: buf.subarray(s.offset, s.offset + s.size) }));
      if (pts <= lastPts) monotonic = false;
      lastPts = pts;
      pts += s.dur;
      if (decoder.decodeQueueSize > 16) await new Promise((r) => decoder.addEventListener('dequeue', r, { once: true }));
    }
    try {
      await decoder.flush();
    } catch (e) {
      decodeError ||= String(e);
    }
    decoder.close();

    // Compare with the recorder's canvas at the same frame index.
    const captured = window.__record!.captured();
    const compare: Analysis['compare'] = [];
    for (const i of want) {
      const a = captured.get(i);
      const b = decodedFrames.get(i);
      if (!a || !b || a.width !== b.width || a.height !== b.height) {
        compare.push({ index: i, mae: -1, p99: -1, max: -1, png: pngs.get(i) ?? '' });
        continue;
      }
      const hist = new Uint32Array(256);
      let sum = 0;
      let n = 0;
      for (let k = 0; k < a.data.length; k += 4) {
        for (let c = 0; c < 3; c++) {
          const d = Math.abs(a.data[k + c]! - b.data[k + c]!);
          sum += d;
          hist[d]!++;
          n++;
        }
      }
      let acc = 0;
      let p99 = 0;
      for (let d = 0; d < 256; d++) {
        acc += hist[d]!;
        if (acc >= n * 0.99) {
          p99 = d;
          break;
        }
      }
      let max = 0;
      for (let d = 255; d >= 0; d--) {
        if (hist[d]) {
          max = d;
          break;
        }
      }
      compare.push({ index: i, mae: sum / n, p99, max, png: pngs.get(i) ?? '' });
    }

    // <video>: metadata, duration, seek.
    const url = URL.createObjectURL(blob);
    const video = document.createElement('video');
    video.muted = true;
    let videoError = '';
    video.onerror = () => (videoError = video.error?.message ?? 'error');
    video.src = url;
    await new Promise<void>((resolve) => {
      video.onloadedmetadata = () => resolve();
      setTimeout(resolve, 15000);
    });
    const videoDuration = video.duration;
    video.currentTime = videoDuration / 2;
    await new Promise<void>((resolve) => {
      video.onseeked = () => resolve();
      setTimeout(resolve, 15000);
    });
    const seekReadyState = video.readyState;
    URL.revokeObjectURL(url);

    return {
      bytes: buf.byteLength,
      brands,
      layout,
      samples: samples.length,
      keys: samples.filter((s) => s.key).length,
      width,
      height,
      avcProfile: profile,
      avcLevel: level,
      colr,
      mvhdSeconds: mvDur / mvTs,
      mdhdSeconds: mdDur / mdTs,
      sumDurationsSeconds: samples.reduce((a, s) => a + s.dur, 0) / mdTs,
      ptsMonotonic: monotonic,
      decoded,
      decodeError,
      displayWidth,
      displayHeight,
      compare,
      videoDuration,
      videoWidth: video.videoWidth,
      videoHeight: video.videoHeight,
      seekReadyState,
      videoError,
      recipe,
      comment,
    };
  }, compareFrames);
}

async function runAndVerify(page: Page, info: TestInfo, spec: RunSpec): Promise<void> {
  await openRun(page, spec);
  const plan = await configure(page, spec);
  const frames = plan.frames > 0 ? plan.frames : plan.upper;
  const mid = Math.floor(frames / 2);
  const wall0 = Date.now();
  const run = await record(page, [0, mid]);
  const wallMs = Date.now() - wall0;
  if (process.env.E2E_RECORD_SAVE) {
    const b64 = await page.evaluate(async () => {
      const bytes = new Uint8Array(await window.__record!.output()!.arrayBuffer());
      let s = '';
      for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
      return btoa(s);
    });
    mkdirSync(process.env.E2E_RECORD_SAVE, { recursive: true });
    writeFileSync(join(process.env.E2E_RECORD_SAVE, `f3-web-${spec.name}.mp4`), Buffer.from(b64, 'base64'));
  }
  const a = await analyse(page, [0, mid]);
  const capErr = await page.evaluate(() => [...window.__record!.captureErrors().entries()]);

  const t = run.timing;
  const loopFps = (run.frames / (t['total'] ?? 1)) * 1000;
  const endFps = (run.frames / ((t['total'] ?? 0) + (t['finish'] ?? 0))) * 1000;
  const summary = {
    run: spec.name,
    size: `${plan.width}x${plan.height}`,
    fps: plan.fps,
    planFrames: plan.frames,
    recordedFrames: run.frames,
    durationS: run.frames / plan.fps,
    throughputFps: Number(endFps.toFixed(1)),
    loopFps: Number(loopFps.toFixed(1)),
    xRealTime: Number((endFps / plan.fps).toFixed(2)),
    wallS: wallMs / 1000,
    msPerFrame: Object.fromEntries(['step', 'capture', 'wait', 'mux', 'sink'].map((k) => [k, Number(((t[k] ?? 0) / run.frames).toFixed(3))])),
    finishMs: Number((t['finish'] ?? 0).toFixed(1)),
    file: { bytes: a.bytes, mbps: Number(((a.bytes * 8) / (run.frames / plan.fps) / 1e6).toFixed(2)), layout: a.layout, brand: a.brands, colr: a.colr, profile: a.avcProfile, level: a.avcLevel },
    decoded: a.decoded,
    samples: a.samples,
    keys: a.keys,
    mvhdS: a.mvhdSeconds,
    videoDuration: a.videoDuration,
    video: `${a.videoWidth}x${a.videoHeight}`,
    seekReadyState: a.seekReadyState,
    compare: a.compare.map((c) => ({ index: c.index, mae: Number(c.mae.toFixed(3)), p99: c.p99, max: c.max })),
    captureOnly: capErr,
  };
  console.log(JSON.stringify(summary));
  await info.attach(`${spec.name}.json`, { body: JSON.stringify(summary, null, 2), contentType: 'application/json' });
  mkdirSync(PNG_DIR, { recursive: true });
  for (const c of a.compare) {
    if (c.png) writeFileSync(join(PNG_DIR, `f3-web-${spec.name}-frame${c.index}.png`), Buffer.from(c.png.split(',')[1]!, 'base64'));
  }

  // The file: plan frame count, duration frames/fps, decodable, playable.
  if (plan.frames > 0) expect(run.frames).toBe(plan.frames);
  expect(a.samples).toBe(run.frames);
  expect(a.decodeError).toBe('');
  expect(a.decoded).toBe(run.frames);
  expect(a.ptsMonotonic).toBe(true);
  expect(a.width).toBe(plan.width);
  expect(a.height).toBe(plan.height);
  expect(a.displayWidth).toBe(plan.width);
  expect(a.displayHeight).toBe(plan.height);
  expect(a.avcProfile).toBe(100);
  expect(a.colr).toBe('nclx 1/13/1 full');
  const duration = run.frames / plan.fps;
  expect(Math.abs(a.mvhdSeconds - duration)).toBeLessThan(0.5 / plan.fps);
  expect(Math.abs(a.sumDurationsSeconds - duration)).toBeLessThan(0.5 / plan.fps);
  expect(a.videoError).toBe('');
  expect(Math.abs(a.videoDuration - duration)).toBeLessThanOrEqual(1 / plan.fps + 1e-3);
  expect(a.videoWidth).toBe(plan.width);
  expect(a.seekReadyState).toBeGreaterThanOrEqual(2);
  expect(a.keys).toBeGreaterThanOrEqual(Math.floor(run.frames / (2 * plan.fps)));
  // Frames: capture is exact; decoded frames are close to the recorder's.
  for (const [, err] of capErr) expect(err).toBe(CAPTURE_EXACT);
  for (const c of a.compare) {
    expect(c.mae).toBeGreaterThanOrEqual(0);
    expect(c.mae).toBeLessThanOrEqual(FRAME_MAE_1080);
  }
  // T-16b: the embedded recipe has inputs only, and no source path for a `?log=` source.
  if (spec.metadata === 'embed') {
    expect(a.recipe).not.toBeNull();
    const recipe = JSON.parse(a.recipe!) as Record<string, any>;
    console.log(`[${spec.name}] embedded recipe: ${a.recipe}`);
    console.log(`[${spec.name}] comment: ${a.comment}`);
    expect(recipe['v']).toBe(1);
    expect(recipe['plan']['fps']).toBe(plan.fps);
    expect(recipe['result']).toBeUndefined();
    expect(recipe['time_zone']).toBeUndefined();
    expect(recipe['source']['path'] ?? '').toBe('');
  } else {
    expect(a.recipe).toBeNull();
  }
}

test.describe.configure({ mode: 'serial' });

test('records the AI co-authors sample at 1080p60 to a file that decodes with the planned frames', async ({ page }, info) => {
  await runAndVerify(page, info, { name: 'ai-coauthors-1080p60', log: '/samples/ai-coauthors.log', size: '1920x1080', exact: true, metadata: 'embed' });
});

test('records the file-removal sample at 720p30 as a fragmented MP4 with the recipe embedded', async ({ page }, info) => {
  await runAndVerify(page, info, { name: 'file-removal-720p30-fragmented', log: '/samples/file-removal.log', size: '1280x720', fps: '30', exact: true, layout: 'fragmented', metadata: 'embed' });
});

test('records rich PR boundary seeds and lifecycle actions at 720p30 with the recipe embedded', async ({ page }, info) => {
  await runAndVerify(page, info, {
    name: 'rich-pr-boundary-720p30', log: '/samples/pull-request-lifecycle.log',
    size: '1280x720', fps: '30', spd: '1', maxSeconds: '2', exact: true,
    layout: 'fragmented', metadata: 'embed',
    extra: '&start-date=2024-01-02&pull-request-mode=full&pull-request-style=dim-highlight&pr-flow-hud=true',
  });
});

for (const [size, name] of [
  ['1280x720', '720p60'],
  ['1920x1080', '1080p60'],
  ['3840x2160', '4k60'],
] as const) {
  test(`larger log ${name}: throughput and decode`, async ({ page }, info) => {
    test.skip(!BIG_LOG, 'set E2E_RECORD_LOG=/samples/<log> (copied into apps/player/dist/samples)');
    await runAndVerify(page, info, { name: `big-${name}`, log: BIG_LOG!, size, spd: process.env.E2E_RECORD_SPD ?? '0.02', maxSeconds: process.env.E2E_RECORD_MAX ?? '60' });
  });
}

test('larger log, whole run at 1080p60 with the exact dry run', async ({ page }, info) => {
  test.skip(!BIG_LOG, 'set E2E_RECORD_LOG=/samples/<log> (copied into apps/player/dist/samples)');
  await runAndVerify(page, info, { name: 'big-full-1080p60', log: BIG_LOG!, size: '1920x1080', spd: process.env.E2E_RECORD_SPD ?? '0.02', exact: true });
});

/** Records with per-frame pre-encode hashes; returns them. */
async function recordHashes(page: Page, during?: () => Promise<void>): Promise<string[]> {
  await page.evaluate(() => {
    window.__recordTest!.sink = 'blob';
    window.__recordTest!.hashes = true;
    window.__recordTest!.captureFrames = [];
  });
  await page.locator('.rec-start').click();
  const confirm = page.locator('.rec-confirm');
  await expect(page.locator('.rec-progress').or(confirm)).toBeVisible();
  if (await confirm.isVisible()) await confirm.click();
  if (during) await during();
  await expect.poll(() => page.evaluate(() => window.__record!.state()), { timeout: 5 * 60_000 }).toBe('done');
  return page.evaluate(() => window.__record!.hashes());
}

test('T-5, T-27, T-30: preview and live interaction change no frame; the live view stays paused', async ({ page }) => {
  const spec: RunSpec = { name: 'determinism', log: '/samples/ai-coauthors.log', size: '1280x720', fps: '30', maxSeconds: '20' };
  await openRun(page, spec);
  await configure(page, spec);
  const first = await recordHashes(page, async () => {
    // T-30: the live engine is paused while recording, even after input reaches it.
    await expect.poll(() => page.evaluate(() => window.__gource!.status().paused)).toBe(true);
    await page.evaluate(() => {
      const app = window.__gource!.app;
      for (let i = 0; i < 20; i++) app.pointer_move(100 + i * 10, 200, 0);
      app.wheel(-300);
      app.key(' ', true, false, false);
      app.key(' ', false, false, false);
      app.key('s', true, false, false);
    });
    await expect(page.locator('.seek')).toBeDisabled();
    await expect.poll(() => page.evaluate(() => window.__gource!.status().paused)).toBe(true);
  });
  expect(first.length).toBe(600);
  expect(new Set(first).size).toBeGreaterThan(100);
  // The live view resumes as it was (fixed-dt mode: not paused before).
  await expect.poll(() => page.evaluate(() => window.__gource!.status().paused)).toBe(false);

  // A second recording after a preview of the same options gives the same frames.
  await page.locator('.rec-foot').getByRole('button', { name: 'Record again' }).click();
  await page.getByRole('button', { name: 'Refresh preview' }).click();
  await expect(page.locator('.rec-preview-state')).toHaveText('', { timeout: 30_000 });
  const second = await recordHashes(page);
  expect(second).toEqual(first);
});

test('T-15: a 4K recording and then a 1080p one on the same page both succeed', async ({ page }, info) => {
  const spec: RunSpec = { name: 'consecutive', log: '/samples/file-removal.log', size: '3840x2160', maxSeconds: '2' };
  await openRun(page, spec);
  await configure(page, spec);
  const a = await record(page, []);
  expect(a.frames).toBe(120);
  await page.locator('.rec-foot').getByRole('button', { name: 'Record again' }).click();
  await page.locator('input[name="rec-size"][value="1920x1080"]').check();
  const b = await record(page, [0]);
  expect(b.frames).toBe(120);
  const analysis = await analyse(page, [0]);
  expect(analysis.width).toBe(1920);
  expect(analysis.decoded).toBe(120);
  await info.attach('consecutive.json', { body: JSON.stringify({ a, b: b.frames, compare: analysis.compare.map((c) => ({ ...c, png: '' })) }), contentType: 'application/json' });
});

test('T-16a: the same settings on a fresh page reproduce the frame hashes', async ({ page, context }) => {
  const spec: RunSpec = { name: 'reproduce', log: '/samples/ai-coauthors.log', size: '1280x720', fps: '25', spd: '3', maxSeconds: '4' };
  await openRun(page, spec);
  await configure(page, spec);
  const original = await recordHashes(page);

  const fresh = await context.newPage();
  await openRun(fresh, spec);
  await configure(fresh, spec);
  const again = await recordHashes(fresh);
  expect(again.length).toBe(original.length);
  expect(again).toEqual(original);
  await fresh.close();
});

test('screenshots of the real dialog, progress and finished screens', async ({ page }) => {
  const spec: RunSpec = { name: 'screens', log: '/samples/ai-coauthors.log', size: '1920x1080', maxSeconds: '30' };
  await openRun(page, spec);
  await configure(page, spec);
  await expect(page.locator('.rec-preview-state')).toHaveText('', { timeout: 30_000 });
  mkdirSync(PNG_DIR, { recursive: true });
  await page.screenshot({ path: join(PNG_DIR, 'f3-web-dialog.png') });
  await page.evaluate(() => (window.__recordTest!.sink = 'blob'));
  await page.locator('.rec-start').click();
  await expect.poll(async () => Number(await page.getByRole('progressbar').getAttribute('aria-valuenow'))).toBeGreaterThan(20);
  await page.screenshot({ path: join(PNG_DIR, 'f3-web-progress.png') });
  await expect.poll(() => page.evaluate(() => window.__record!.state()), { timeout: 120_000 }).toBe('done');
  await page.screenshot({ path: join(PNG_DIR, 'f3-web-done.png') });
});

// F1 x F3 (merge wiring): the Show toggles carry F1's `decorations` hide
// element and `agent-key` into the recording, and a reproduce link with
// style rules shows F1's notice once and keeps the `record=1` prefill.
test('F1 decorations and agent key are Show toggles that change the recorded frames', async ({ page }) => {
  const spec: RunSpec = { name: 'decorations', log: '/samples/ai-coauthors.log', size: '1280x720', fps: '30', spd: '0.5', maxSeconds: '6' };
  await openRun(page, spec);
  await configure(page, spec);
  const dialog = page.locator('.rec-dialog');
  const decorations = dialog.getByLabel('AI decorations');
  const agentKey = dialog.getByLabel('Agent key', { exact: true });
  await expect(decorations).toBeChecked();
  await expect(agentKey).not.toBeChecked();
  const shown = await recordHashes(page);

  await dialog.locator('.rec-foot').getByRole('button', { name: 'Record again' }).click();
  await decorations.uncheck();
  const hidden = await recordHashes(page);
  expect(hidden.length).toBe(shown.length);
  expect(hidden.filter((h, i) => h !== shown[i]).length).toBeGreaterThan(0);

  await dialog.locator('.rec-foot').getByRole('button', { name: 'Record again' }).click();
  await decorations.check();
  await agentKey.check();
  const keyed = await recordHashes(page);
  expect(keyed.length).toBe(shown.length);
  expect(keyed.filter((h, i) => h !== shown[i]).length).toBeGreaterThan(0);
  // The live view keeps its own settings.
  expect(await page.evaluate(() => window.__gource!.app.get_option('agent-key'))).toBe('false');
});

test('a link with style rules: one F1 notice, and the same frames on a fresh page', async ({ page, context }) => {
  const spec: RunSpec = { name: 'reproduce-style', log: '/samples/ai-coauthors.log', size: '1280x720', fps: '24', maxSeconds: '3', extra: `&user-style=${encodeURIComponent('ai => glow=agent')}` };
  await openRun(page, spec);
  await expect(page.locator('.ai-notice')).toHaveCount(1);
  await configure(page, spec);
  const original = await recordHashes(page);

  const fresh = await context.newPage();
  await openRun(fresh, spec);
  await expect(fresh.locator('.ai-notice')).toHaveText('This link applies custom styling.');
  await configure(fresh, spec);
  const again = await recordHashes(fresh);
  expect(again).toEqual(original);
  await expect(fresh.locator('.ai-notice')).toHaveCount(1);
  await fresh.close();
});
