// SPDX-License-Identifier: GPL-3.0-or-later

// Placeholder engine for development and UI tests (`?mock=1`). It implements
// the GourceApp API with a Canvas2D sketch of the visualisation so the page
// can be exercised without the wasm build. In mock mode the "batches" are
// whole lines of the custom log (see worker/parser.worker.ts).
//
// Every API call is recorded in `window.__gourceMock.calls` for tests.

import { HIDE_ELEMENTS, OPTIONS_BY_NAME, RESTART_OPTIONS, isColour, normaliseHide } from '../options';
import type { EngineStatus, GourceApp } from './types';

export interface MockCall {
  method: string;
  args: unknown[];
}

interface MockRegistry {
  calls: MockCall[];
  instances: number;
}

declare global {
  interface Window {
    __gourceMock?: MockRegistry;
  }
}

function registry(): MockRegistry {
  window.__gourceMock ??= { calls: [], instances: 0 };
  return window.__gourceMock;
}

interface MockChange {
  action: string;
  path: string;
}
interface MockCommit {
  time: number;
  user: string;
  changes: MockChange[];
  /** F1: credits of the custom log's meta field (`ai=co:claude:Model,au:cursor`). */
  ai: MockCredit[];
}
interface MockCredit {
  kind: 'co' | 'au';
  agent: string;
  model: string;
}
interface MockFile {
  x: number;
  y: number;
  colour: string;
  touched: number;
  dir: string;
}
interface MockUser {
  x: number;
  y: number;
  tx: number;
  ty: number;
  colour: string;
  active: number;
  beams: Array<{ file: MockFile; colour: string; t: number }>;
  /** F1: seconds of decoration left, and its agent colour. */
  assist: number;
  assistColour: string;
}

const DAY_NAMES = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
const MONTH_NAMES = [
  'January', 'February', 'March', 'April', 'May', 'June',
  'July', 'August', 'September', 'October', 'November', 'December',
];

/** Gource's default date format, `%A, %d %B, %Y %X`, in UTC. */
export function formatGourceDate(unixSeconds: number): string {
  const d = new Date(unixSeconds * 1000);
  const pad = (n: number) => String(n).padStart(2, '0');
  return (
    `${DAY_NAMES[d.getUTCDay()]}, ${pad(d.getUTCDate())} ${MONTH_NAMES[d.getUTCMonth()]}, ${d.getUTCFullYear()} ` +
    `${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}:${pad(d.getUTCSeconds())}`
  );
}

function hash(text: string): number {
  let h = 2166136261;
  for (let i = 0; i < text.length; i++) {
    h ^= text.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

function hashColour(text: string, seed: number, mix = 0): string {
  const h = hash(`${seed}:${text}`);
  const r = (h & 0xff) / 255;
  const g = ((h >>> 8) & 0xff) / 255;
  const b = ((h >>> 16) & 0xff) / 255;
  const len = Math.hypot(r, g, b) || 1;
  const c = [r, g, b].map((v) => Math.round(((v / len) * (1 - mix) + mix) * 255));
  return `rgb(${c[0]},${c[1]},${c[2]})`;
}

const ACTION_COLOURS: Record<string, string> = { A: '#40ff40', M: '#ffa500', D: '#ff3030' };

export class MockEngine implements GourceApp {
  private readonly canvas: HTMLCanvasElement;
  private readonly ctx: CanvasRenderingContext2D;
  private readonly decoder = new TextDecoder();
  private readonly options = new Map<string, string>();
  private commits: MockCommit[] = [];
  private complete = false;
  private paused = false;
  private played = 0;
  private simTime = 0;
  private lastNow = -1;
  private files = new Map<string, MockFile>();
  private users = new Map<string, MockUser>();
  private dirs = new Set<string>();
  private zoom = 1;
  private cssWidth = 0;
  private cssHeight = 0;
  private dpr = 1;
  private fps = 0;
  private frameMs = 0;
  private fixedDt = 0;
  private ticks = 0;
  private freed = false;

  constructor(canvas: HTMLCanvasElement) {
    const ctx = canvas.getContext('2d');
    if (!ctx) throw new Error('Canvas 2D is unavailable.');
    this.canvas = canvas;
    this.ctx = ctx;
    registry().instances += 1;
    this.record('constructor', []);
  }

  private record(method: string, args: unknown[]): void {
    registry().calls.push({ method, args });
  }

  private opt(name: string): string {
    return this.options.get(name) ?? OPTIONS_BY_NAME.get(name)?.default ?? '';
  }

  private num(name: string): number {
    return Number(this.opt(name));
  }

  private hidden(element: string): boolean {
    return this.opt('hide').split(',').includes(element);
  }

  free(): void {
    this.record('free', []);
    this.freed = true;
  }

  begin_log(): void {
    this.record('begin_log', []);
    this.commits = [];
    this.complete = false;
    this.resetSimulation();
  }

  append_batch(batch: Uint8Array): void {
    this.record('append_batch', [batch.byteLength]);
    const text = this.decoder.decode(batch);
    for (const line of text.split('\n')) {
      const parts = line.split('|');
      if (parts.length < 4) continue;
      const time = Number(parts[0]);
      const user = parts[1]!;
      const action = parts[2]!;
      const path = parts[3]!;
      if (!Number.isFinite(time) || !/^[AMD]$/.test(action)) continue;
      const last = this.commits[this.commits.length - 1];
      const ai = parseMockCredits(parts[5] ?? '');
      if (last && last.time === time && last.user === user) {
        last.changes.push({ action, path });
        if (last.ai.length === 0) last.ai = ai;
      } else this.commits.push({ time, user, changes: [{ action, path }], ai });
    }
    if (this.simTime === 0 && this.commits.length > 0) this.simTime = this.commits[0]!.time;
  }

  finish_log(total_bytes: number): void {
    this.record('finish_log', [total_bytes]);
    this.complete = true;
  }

  frame(now_ms: number): void {
    if (this.freed) return;
    const start = performance.now();
    const dt = this.lastNow < 0 ? 1 / 60 : Math.min((now_ms - this.lastNow) / 1000, 0.1);
    this.lastNow = now_ms;
    if (dt > 0) this.fps = this.fps * 0.9 + (1 / dt) * 0.1;
    if (this.fixedDt === 0 && !this.paused) {
      this.advance(dt);
      this.ticks++;
    }
    this.draw(dt);
    this.frameMs = performance.now() - start;
  }

  set_fixed_dt(seconds: number): void {
    this.record('set_fixed_dt', [seconds]);
    this.fixedDt = Number.isFinite(seconds) && seconds > 0 ? seconds : 0;
  }

  step(ticks: number): void {
    this.record('step', [ticks]);
    const dt = this.fixedDt || 1 / 60;
    for (let i = 0; i < ticks; i++) {
      if (!this.paused) this.advance(dt);
      this.ticks++;
    }
    this.draw(dt);
  }

  private advance(dt: number): void {
    const secondsPerDay = Math.max(this.num('seconds-per-day'), 0.01);
    const scale = Math.max(this.num('time-scale'), 0.01);
    this.simTime += (dt * scale * 86400) / secondsPerDay;
    const next = this.commits[this.played];
    const skip = this.num('auto-skip-seconds');
    if (next && skip > 0 && next.time - this.simTime > (skip * 86400) / secondsPerDay) {
      this.simTime = next.time;
    }
    while (this.played < this.commits.length && this.commits[this.played]!.time <= this.simTime) {
      const commit = this.commits[this.played]!;
      if (this.passesAiFilter(commit)) this.apply(commit, true);
      this.played++;
    }
    if (this.played >= this.commits.length && this.complete && this.opt('loop') === 'true') {
      this.resetSimulation();
    }
  }

  private apply(commit: MockCommit, animate: boolean): void {
    const seed = this.num('hash-seed');
    let user = this.users.get(commit.user);
    if (!user) {
      user = { x: 0, y: 0, tx: 0, ty: 0, colour: hashColour(commit.user, seed, 0.4), active: 0, beams: [], assist: 0, assistColour: '' };
      this.users.set(commit.user, user);
    }
    user.active = 3;
    const credit = commit.ai[0];
    if (credit) {
      user.assist = 3;
      user.assistColour = `#${MOCK_AGENTS[credit.agent]?.colour ?? 'FFFFFF'}`;
    }
    for (const change of commit.changes) {
      const slash = change.path.lastIndexOf('/');
      const dir = slash > 0 ? change.path.slice(0, slash) : '/';
      if (change.action === 'D') {
        this.files.delete(change.path);
      } else if (!this.files.has(change.path)) {
        const pos = this.dirPosition(dir);
        const h = hash(change.path);
        const angle = ((h & 0xffff) / 0xffff) * Math.PI * 2;
        const r = 6 + ((h >>> 16) % 24);
        const ext = change.path.includes('.') ? change.path.slice(change.path.lastIndexOf('.') + 1) : '';
        this.files.set(change.path, {
          x: pos.x + Math.cos(angle) * r,
          y: pos.y + Math.sin(angle) * r,
          colour: ext ? hashColour(ext, seed) : '#ffffff',
          touched: 0,
          dir,
        });
      }
      const file = this.files.get(change.path);
      if (file) {
        file.touched = 1;
        user.tx = file.x + 40;
        user.ty = file.y - 30;
        if (animate) user.beams.push({ file, colour: ACTION_COLOURS[change.action] ?? '#fff', t: 1 });
      }
      this.dirs.add(dir);
    }
    if (user.beams.length > 12) user.beams.splice(0, user.beams.length - 12);
  }

  private dirPosition(dir: string): { x: number; y: number } {
    if (dir === '/' || dir === '') return { x: 0, y: 0 };
    const depth = dir.split('/').length - 1;
    const parentEnd = dir.lastIndexOf('/');
    const parent = this.dirPosition(parentEnd > 0 ? dir.slice(0, parentEnd) : '/');
    const angle = ((hash(dir) & 0xffff) / 0xffff) * Math.PI * 2;
    const dist = 90 / Math.sqrt(depth);
    return { x: parent.x + Math.cos(angle) * dist, y: parent.y + Math.sin(angle) * dist };
  }

  private resetSimulation(): void {
    this.played = 0;
    this.files.clear();
    this.users.clear();
    this.dirs.clear();
    this.simTime = this.commits[0]?.time ?? 0;
  }

  private draw(dt: number): void {
    const { ctx, canvas } = this;
    const w = canvas.width;
    const h = canvas.height;
    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.globalCompositeOperation = 'source-over';
    ctx.fillStyle = `#${this.opt('background-colour')}`;
    ctx.fillRect(0, 0, w, h);
    if (w === 0 || h === 0) return;

    const scale = this.dpr * this.zoom * Math.min(w, h) / (this.dpr * 520);
    ctx.setTransform(scale, 0, 0, scale, w / 2, h / 2);

    if (!this.hidden('tree')) {
      ctx.strokeStyle = 'rgba(255,255,255,0.18)';
      ctx.lineWidth = 1.5 / scale * this.dpr;
      ctx.beginPath();
      for (const dir of this.dirs) {
        const p = this.dirPosition(dir);
        const parentEnd = dir.lastIndexOf('/');
        const q = this.dirPosition(parentEnd > 0 ? dir.slice(0, parentEnd) : '/');
        ctx.moveTo(q.x, q.y);
        ctx.lineTo(p.x, p.y);
      }
      ctx.stroke();
    }

    if (!this.hidden('bloom')) {
      ctx.globalCompositeOperation = 'lighter';
      const intensity = this.num('bloom-intensity') * 0.25;
      const radius = 40 * this.num('bloom-multiplier');
      for (const dir of this.dirs) {
        const p = this.dirPosition(dir);
        const g = ctx.createRadialGradient(p.x, p.y, 0, p.x, p.y, radius);
        g.addColorStop(0, `rgba(120,160,255,${intensity})`);
        g.addColorStop(1, 'rgba(120,160,255,0)');
        ctx.fillStyle = g;
        ctx.fillRect(p.x - radius, p.y - radius, radius * 2, radius * 2);
      }
      ctx.globalCompositeOperation = 'source-over';
    }

    if (!this.hidden('files')) {
      for (const file of this.files.values()) {
        ctx.fillStyle = file.colour;
        ctx.beginPath();
        ctx.arc(file.x, file.y, 3 + file.touched * 2, 0, Math.PI * 2);
        ctx.fill();
        file.touched = Math.max(0, file.touched - dt);
      }
    }

    if (!this.hidden('users')) {
      for (const [name, user] of this.users) {
        const k = Math.min(1, dt * 3);
        user.x += (user.tx - user.x) * k;
        user.y += (user.ty - user.y) * k;
        user.active = Math.max(0, user.active - dt);
        for (const beam of user.beams) {
          ctx.strokeStyle = beam.colour;
          ctx.globalAlpha = beam.t;
          ctx.lineWidth = 2 / scale * this.dpr;
          ctx.beginPath();
          ctx.moveTo(user.x, user.y);
          ctx.lineTo(beam.file.x, beam.file.y);
          ctx.stroke();
          beam.t -= dt * 1.5;
        }
        user.beams = user.beams.filter((b) => b.t > 0);
        ctx.globalAlpha = 0.35 + 0.65 * Math.min(1, user.active);
        user.assist = Math.max(0, user.assist - dt);
        if (user.assist > 0 && !this.hidden('decorations') && this.opt('ai-style') !== 'none') {
          ctx.strokeStyle = user.assistColour;
          ctx.lineWidth = 3 / scale * this.dpr;
          ctx.beginPath();
          ctx.arc(user.x, user.y, 13 * this.num('user-scale'), 0, Math.PI * 2);
          ctx.stroke();
        }
        ctx.fillStyle = user.colour;
        ctx.beginPath();
        ctx.arc(user.x, user.y, 9 * this.num('user-scale'), 0, Math.PI * 2);
        ctx.fill();
        if (!this.hidden('usernames')) {
          ctx.fillStyle = `#${this.opt('font-colour')}`;
          ctx.font = `${12}px GourceSans, sans-serif`;
          ctx.fillText(name, user.x + 12, user.y + 4);
        }
        ctx.globalAlpha = 1;
      }
    }

    ctx.setTransform(this.dpr, 0, 0, this.dpr, 0, 0);
    ctx.fillStyle = `#${this.opt('font-colour')}`;
    ctx.textAlign = 'center';
    const fontSize = this.num('font-size') * this.num('font-scale');
    ctx.font = `${fontSize}px GourceSans, sans-serif`;
    if (!this.hidden('date') && this.commits.length > 0) {
      ctx.fillText(formatGourceDate(this.simTime), this.cssWidth / 2, 20 + fontSize);
    }
    const filterNote = MOCK_FILTER_NOTES[this.opt('ai-filter')];
    if (filterNote) ctx.fillText(filterNote, this.cssWidth / 2, 20 + fontSize * 2.4);
    const title = this.opt('title');
    if (title) {
      ctx.textAlign = 'left';
      ctx.fillText(title, 10, this.cssHeight - 12);
    }
    ctx.textAlign = 'right';
    ctx.font = '11px GourceSans, sans-serif';
    ctx.fillStyle = 'rgba(255,255,255,0.35)';
    ctx.fillText('mock engine', this.cssWidth - 10, this.cssHeight - 8);
  }

  resize(css_width: number, css_height: number, device_pixel_ratio: number): void {
    this.record('resize', [css_width, css_height, device_pixel_ratio]);
    this.cssWidth = css_width;
    this.cssHeight = css_height;
    this.dpr = device_pixel_ratio;
    this.canvas.width = Math.max(1, Math.round(css_width * device_pixel_ratio));
    this.canvas.height = Math.max(1, Math.round(css_height * device_pixel_ratio));
  }

  pointer_move(x: number, y: number, buttons: number): void {
    this.record('pointer_move', [x, y, buttons]);
  }
  pointer_down(x: number, y: number, button: number): void {
    this.record('pointer_down', [x, y, button]);
  }
  pointer_up(x: number, y: number, button: number): void {
    this.record('pointer_up', [x, y, button]);
  }
  pointer_leave(): void {
    this.record('pointer_leave', []);
  }
  wheel(delta_y: number): void {
    this.record('wheel', [delta_y]);
    this.zoom = Math.min(8, Math.max(0.2, this.zoom * Math.exp(-delta_y / 500)));
  }

  key(key: string, pressed: boolean, shift: boolean, ctrl: boolean): boolean {
    this.record('key', [key, pressed, shift, ctrl]);
    if (key === ' ' && pressed) this.paused = !this.paused;
    return mockReactsTo(key, ctrl);
  }

  set_option(name: string, value: string): string {
    this.record('set_option', [name, value]);
    const def = OPTIONS_BY_NAME.get(name);
    if (!def) throw new Error(`unknown option '${name}'`);
    const invalid = (reason: string) => new Error(`invalid value '${value}' for '${name}': ${reason}`);
    switch (def.kind) {
      case 'colour':
        if (!isColour(value)) throw invalid('expected RRGGBB');
        break;
      case 'range':
        if (!Number.isFinite(Number(value)) || value.trim() === '') throw invalid('expected a number');
        break;
      case 'bool':
        if (value !== 'true' && value !== 'false') throw invalid('expected true or false');
        break;
      case 'select':
        if (!def.choices.some((c) => c.value === value)) throw invalid('unknown choice');
        break;
      default:
        break;
    }
    if (name === 'hide') {
      const items = value === '' ? [] : value.split(',');
      const known = normaliseHide(items);
      if (known.length !== items.length) throw invalid(`expected a list of ${HIDE_ELEMENTS.join(', ')}`);
    }
    if (name === 'hash-seed' && !/^\d+$/.test(value)) throw invalid('expected an integer');
    if (name === 'user-style') {
      const bad = mockStyleError(value);
      if (bad) throw new Error(`${bad.code}: ${bad.message} (line ${bad.line})`);
    }
    if (def.kind === 'list') {
      for (const line of value.split('\n')) {
        if (line === '') continue;
        try {
          new RegExp(line);
        } catch {
          if (def.section === 'filters') throw invalid('invalid regular expression');
        }
      }
    }
    const wasPrHidden = normaliseHide((this.options.get('hide') ?? '').split(',')).includes('pull-requests');
    this.options.set(name, value);
    if (name === 'hide') {
      this.options.set('hide-pull-requests', String(normaliseHide(value.split(',')).includes('pull-requests')));
    } else if (name === 'hide-pull-requests') {
      const hide = new Set(normaliseHide((this.options.get('hide') ?? '').split(',')));
      if (value === 'true') hide.add('pull-requests');
      else hide.delete('pull-requests');
      this.options.set('hide', normaliseHide(hide).join(','));
    }
    const isPrHidden = normaliseHide((this.options.get('hide') ?? '').split(',')).includes('pull-requests');
    return RESTART_OPTIONS.has(name) || wasPrHidden !== isPrHidden ? 'restart' : 'live';
  }

  check_style(text: string): string {
    this.record('check_style', [text]);
    const bad = mockStyleError(text);
    return JSON.stringify(bad ? { ok: false, ...bad } : { ok: true });
  }

  /** Counted from the log's `ai=` fields, in the engine's JSON shape (F1 AI-63). */
  ai_stats(): string {
    const playedCommits = this.commits.slice(0, this.played).filter((c) => this.passesAiFilter(c));
    const sets = new Set(this.commits.filter((c) => c.ai.length > 0).map((c) => JSON.stringify(c.ai)));
    return JSON.stringify({
      registry: 1,
      loaded: { ...mockTotals(this.commits), sets: sets.size },
      played: mockTotals(playedCommits),
    });
  }

  private passesAiFilter(commit: MockCommit): boolean {
    const filter = this.opt('ai-filter');
    if (filter === 'only-assisted') return commit.ai.length > 0;
    if (filter === 'exclude-assisted') return commit.ai.length === 0;
    return true;
  }

  get_option(name: string): string | undefined {
    if (!OPTIONS_BY_NAME.has(name)) return undefined;
    return this.opt(name);
  }

  private focus = '';
  set_camera_focus(dir: string): void {
    this.record('set_camera_focus', [dir]);
    this.focus = dir;
  }

  camera_focus(): string {
    return this.focus;
  }

  restart(): void {
    this.record('restart', []);
    this.resetSimulation();
  }

  set_paused(paused: boolean): void {
    this.record('set_paused', [paused]);
    this.paused = paused;
  }

  seek(fraction: number): void {
    this.record('seek', [fraction]);
    if (!this.complete || this.commits.length === 0) return;
    const target = Math.min(this.commits.length - 1, Math.floor(fraction * this.commits.length));
    this.resetSimulation();
    for (let i = 0; i < target; i++) if (this.passesAiFilter(this.commits[i]!)) this.apply(this.commits[i]!, false);
    this.played = target;
    this.simTime = this.commits[target]!.time;
  }

  date_at(fraction: number): string {
    if (this.commits.length === 0) return '';
    const i = Math.min(this.commits.length - 1, Math.max(0, Math.floor(fraction * this.commits.length)));
    return formatGourceDate(this.commits[i]!.time);
  }

  status(): string {
    const total = this.commits.length;
    const status: EngineStatus = {
      date: total > 0 ? formatGourceDate(this.simTime) : '',
      time: Math.floor(this.simTime),
      fraction: total > 0 ? this.played / total : 0,
      seekable: this.complete,
      paused: this.paused,
      finished: this.complete && this.played >= total,
      commits_loaded: total,
      commits_played: this.played,
      files: this.files.size,
      users: this.users.size,
      dirs: this.dirs.size,
      fps: Math.round(this.fps * 10) / 10,
      frame_ms: this.frameMs,
      sim_ms: this.frameMs * 0.3,
      build_ms: this.frameMs * 0.3,
      gpu_ms: this.frameMs * 0.4,
      draw_calls: 12,
      vertices: (this.files.size + this.users.size) * 4,
      fixed_dt: this.fixedDt,
      ticks: this.ticks,
    };
    return JSON.stringify(status);
  }
}

/** The keys gource-web reports as handled (`input.rs` `gource_reacts_to`). */
function mockReactsTo(key: string, ctrl: boolean): boolean {
  if ([' ', 'Tab', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(key)) return true;
  return !ctrl && key.length === 1 && 'mntgudfrkcvzs=+-[].,/'.includes(key.toLowerCase());
}

// --- F1: AI attribution in the mock ---

const MOCK_AGENTS: Record<string, { name: string; colour: string }> = {
  claude: { name: 'Claude', colour: 'E69F00' },
  codex: { name: 'Codex', colour: '56B4E9' },
  copilot: { name: 'GitHub Copilot', colour: '009E73' },
  cursor: { name: 'Cursor', colour: 'CC79A7' },
};

const MOCK_FILTER_NOTES: Record<string, string> = {
  'only-assisted': 'AI commits: only assisted',
  'exclude-assisted': 'AI commits: excluding assisted',
};

/** `ai=co:claude:Claude Opus 5.5,au:cursor` (other keys ignored). */
function parseMockCredits(meta: string): MockCredit[] {
  const pair = meta.split(';').find((p) => p.startsWith('ai='));
  if (!pair) return [];
  return pair
    .slice(3)
    .split(',')
    .flatMap((credit) => {
      const [kind, agent, model] = credit.split(':');
      if ((kind !== 'co' && kind !== 'au') || !agent) return [];
      return [{ kind, agent, model: model ?? '' }];
    });
}

function mockTotals(commits: MockCommit[]) {
  const agents = new Map<string, { commits: number; as_author: number; models: Map<string, number> }>();
  let assisted = 0;
  let agent = 0;
  for (const c of commits) {
    if (c.ai.length === 0) continue;
    if (c.ai.some((a) => a.kind === 'au')) agent++;
    else assisted++;
    for (const id of new Set(c.ai.map((a) => a.agent))) {
      const entry = agents.get(id) ?? { commits: 0, as_author: 0, models: new Map<string, number>() };
      entry.commits++;
      const credits = c.ai.filter((a) => a.agent === id);
      if (credits.some((a) => a.kind === 'au')) entry.as_author++;
      for (const model of new Set(credits.map((a) => a.model).filter((m) => m !== ''))) {
        entry.models.set(model, (entry.models.get(model) ?? 0) + 1);
      }
      agents.set(id, entry);
    }
  }
  const byCount = <T extends { commits: number }>(key: (t: T) => string) => (a: T, b: T) =>
    b.commits - a.commits || key(a).localeCompare(key(b));
  return {
    commits: commits.length,
    assisted,
    agent,
    bot: 0,
    agents: [...agents]
      .map(([id, e]) => ({
        id,
        name: MOCK_AGENTS[id]?.name ?? id,
        colour: MOCK_AGENTS[id]?.colour ?? 'FFFFFF',
        commits: e.commits,
        as_author: e.as_author,
        models: [...e.models].map(([model, n]) => ({ model, commits: n })).sort(byCount((m) => m.model)),
        other_models: 0,
      }))
      .sort(byCount((a) => a.id)),
    more_agents: 0,
  };
}

/** The mock's only rule check: every rule line needs `=>` (the full grammar is the engine's). */
function mockStyleError(text: string): { code: string; line: number; message: string } | null {
  const lines = text.split('\n');
  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]!.trim();
    if (line === '' || line.startsWith('#')) continue;
    if (!line.includes('=>')) return { code: 'style_missing_arrow', line: i + 1, message: 'Rule needs a selector, => and properties.' };
  }
  return null;
}
