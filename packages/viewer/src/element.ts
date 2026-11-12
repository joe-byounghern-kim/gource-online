// SPDX-License-Identifier: GPL-3.0-or-later

// <gource-viewer>: the canvas Gource draws on, the engine on it, the input it
// forwards and the render loop. A host page adds everything else (controls,
// panels, recording) around it and talks to the engine through this element.
//
// Two ways to use it:
// - Set `src` (and `mock`, `font-src`): the element creates the engine, loads
//   the log and starts playing by itself.
// - Call `attach(app)` with an engine the host created on `canvas`, stream a
//   log into it (`load`, or `LogLoader`) and call `start()` when it should play.

import { loadEngine } from './engine/load';
import { parseStatus, type EngineStatus, type GourceApp } from './engine/types';
import { loadGourceFont } from './font';
import { LogLoader, type LogDone, type LogError, type LogProgress } from './log-loader';
import type { LogSource } from './worker/protocol';

/** Gource's camera zoom step per wheel notch (`Gource::zoom`). */
const ZOOM_STEP = 1.1;
/** DOM wheel pixels the engine turns into one notch (`gource-web` input). */
const WHEEL_NOTCH_PX = 100;

export interface GourceViewerEventMap {
  /** Log bytes or commits arrived; `playable` once the first commit is in the engine. */
  'gource-progress': CustomEvent<LogProgress & { playable: boolean }>;
  /** The whole log is in the engine and the run is seekable. */
  'gource-done': CustomEvent<LogDone>;
  /** Loading failed (`empty_log` when no commit could be read), or the engine could not start. */
  'gource-error': CustomEvent<LogError>;
  /** `frame` threw: the render loop stopped. */
  'gource-render-error': CustomEvent<{ message: string }>;
  /** The engine handled a key press, which may have changed its options. */
  'gource-key': CustomEvent<{ key: string }>;
}

export class GourceViewerElement extends HTMLElement {
  static readonly observedAttributes = ['src'];

  /** The canvas Gource draws on. Hosts that create the engine themselves pass it to `createApp`. */
  readonly canvas: HTMLCanvasElement;
  /** Where else key presses count as aimed at the visualisation, besides the canvas and the page body. */
  keyScope: HTMLElement | null = null;
  private app: GourceApp | null = null;
  /** `play` was called: the engine is on its way, or here. */
  private started = false;
  private loader: LogLoader | null = null;
  private raf = 0;
  private focusOnStart = true;
  private running = false;
  private disposed = false;
  private readonly cleanups: Array<() => void> = [];

  constructor() {
    super();
    this.canvas = document.createElement('canvas');
    this.canvas.className = 'gource-canvas';
    this.canvas.tabIndex = 0;
  }

  connectedCallback(): void {
    // A disposed element stays disposed; a page that wants it back creates a new one.
    if (this.disposed) return;
    if (!this.canvas.isConnected) this.append(this.canvas);
    if (!this.cleanups.length) this.bindInput();
    this.playSrc();
  }

  disconnectedCallback(): void {
    // Moving the element within the page reconnects it at once; only a real removal disposes.
    queueMicrotask(() => { if (!this.isConnected) this.dispose(); });
  }

  attributeChangedCallback(name: string): void {
    if (name === 'src' && this.isConnected) this.playSrc();
  }

  /** `src` names the one log the element plays: the first one set, once the element is on the page. */
  private playSrc(): void {
    const src = this.getAttribute('src');
    if (!src || this.disposed) return;
    if (this.started || this.app) {
      console.warn('<gource-viewer> plays one log; create a new element for another. Ignoring src change.');
      return;
    }
    void this.play({ kind: 'url', url: src }, {}, false);
  }

  /** The engine, once created or attached. */
  get engine(): GourceApp | null {
    return this.app;
  }

  /** Whether the render loop runs. */
  get playing(): boolean {
    return this.running;
  }

  /** Takes an engine the host created on `canvas`, and sizes it. */
  attach(app: GourceApp): void {
    if (this.app) throw new Error('<gource-viewer> already has an engine.');
    this.app = app;
    this.resize();
  }

  /**
   * Creates the engine on `canvas` (after the font, when `font-src` is set),
   * applies `options` and loads `source`; plays from the first commit. `focus`
   * moves keyboard focus to the canvas when it starts.
   */
  async play(source: LogSource, options: Readonly<Record<string, string>> = {}, focus = true): Promise<void> {
    if (this.started || this.app) throw new Error('<gource-viewer> already has an engine.');
    this.started = true;
    this.focusOnStart = focus;
    const mock = this.hasAttribute('mock');
    try {
      const font = this.getAttribute('font-src');
      if (font) await loadGourceFont(font);
      const factory = await loadEngine(mock);
      if (this.disposed) return;
      this.attach(factory.createApp(this.canvas));
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      this.emit('gource-error', { code: err instanceof Error && err.name === 'EngineMissingError' ? 'engine_missing' : 'engine_failed', message });
      return;
    }
    for (const [name, value] of Object.entries(options)) {
      try {
        this.app!.set_option(name, value);
      } catch (err) {
        console.warn(`Ignoring option ${name}=${value}:`, err);
      }
    }
    this.load(source, mock);
  }

  /** Streams a log into the attached engine; events report its progress. Starts playing at the first commit. */
  load(source: LogSource, mock = this.hasAttribute('mock')): void {
    const app = this.app;
    if (!app || this.loader) throw new Error('<gource-viewer> needs an engine and loads one log.');
    this.loader = new LogLoader(app, mock, {
      progress: (progress, playable) => {
        this.emit('gource-progress', { ...progress, playable });
        if (playable) this.start(this.focusOnStart);
      },
      done: (done) => {
        this.emit('gource-done', done);
        this.start(this.focusOnStart);
      },
      empty: () => this.emit('gource-error', { code: 'empty_log', message: 'No commit could be read.' }),
      error: (error) => this.emit('gource-error', error),
    });
    this.loader.start(source);
  }

  /** Starts the render loop, and focuses the canvas unless `focus` is false. */
  start(focus = true): void {
    if (this.running || this.disposed) return;
    this.running = true;
    this.resize();
    const loop = (now: number) => {
      if (this.disposed) return;
      this.raf = requestAnimationFrame(loop);
      try {
        this.app?.frame(now);
      } catch (err) {
        cancelAnimationFrame(this.raf);
        this.emit('gource-render-error', { message: err instanceof Error ? err.message : String(err) });
      }
    };
    this.raf = requestAnimationFrame(loop);
    if (focus) this.canvas.focus({ preventScroll: true });
  }

  /** The engine's status, or null before it runs. */
  status(): EngineStatus | null {
    try {
      return this.app ? parseStatus(this.app.status()) : null;
    } catch {
      return null;
    }
  }

  /** Stops the loop, the input and the download, and frees the engine. */
  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    cancelAnimationFrame(this.raf);
    this.loader?.cancel();
    this.loader = null;
    for (const fn of this.cleanups.splice(0)) fn();
    try {
      this.app?.free();
    } catch {
      // Already freed.
    }
    this.app = null;
  }

  resize(): void {
    const w = this.canvas.clientWidth;
    const h = this.canvas.clientHeight;
    if (!this.app || w === 0 || h === 0) return;
    this.app.resize(w, h, window.devicePixelRatio || 1);
  }

  private emit<K extends keyof GourceViewerEventMap>(type: K, detail: GourceViewerEventMap[K]['detail']): void {
    this.dispatchEvent(new CustomEvent(type, { detail }));
  }

  private bindInput(): void {
    const canvas = this.canvas;
    const on = <K extends keyof HTMLElementEventMap>(
      target: HTMLElement | Window,
      type: K,
      fn: (e: HTMLElementEventMap[K]) => void,
      opts?: AddEventListenerOptions,
    ) => {
      target.addEventListener(type, fn as EventListener, opts);
      this.cleanups.push(() => target.removeEventListener(type, fn as EventListener, opts));
    };
    const local = (e: MouseEvent) => {
      const r = canvas.getBoundingClientRect();
      return [e.clientX - r.left, e.clientY - r.top] as const;
    };

    // Pointers: the engine sees one mouse. The first pointer down drives it;
    // a second touch turns the gesture into a pinch (wheel notches) and
    // further pointers are ignored until every finger is up.
    const down = new Map<number, readonly [number, number]>();
    let driver: number | null = null;
    let pinching = false;
    let pinchAnchor = 0;
    const spread = () => {
      const [a, b] = [...down.values()];
      return a && b ? Math.hypot(a[0] - b[0], a[1] - b[1]) : 0;
    };
    const releaseDriver = (x: number, y: number, button: number) => {
      if (driver === null) return;
      driver = null;
      this.app?.pointer_up(x, y, button);
    };
    const endPointer = (e: PointerEvent, cancelled: boolean) => {
      const last = down.get(e.pointerId);
      if (!last) return;
      down.delete(e.pointerId);
      if (canvas.hasPointerCapture(e.pointerId)) canvas.releasePointerCapture(e.pointerId);
      if (e.pointerId === driver) {
        const [x, y] = cancelled ? last : local(e);
        releaseDriver(x, y, cancelled ? 0 : e.button);
        if (cancelled) this.app?.pointer_leave();
      }
      if (down.size < 2) pinchAnchor = 0;
      if (down.size === 0) pinching = false;
    };

    on(canvas, 'pointerdown', (e) => {
      canvas.focus({ preventScroll: true });
      const xy = local(e);
      down.set(e.pointerId, xy);
      try {
        canvas.setPointerCapture(e.pointerId);
      } catch {
        // The pointer is already gone.
      }
      if (pinching) return;
      if (down.size === 1) {
        driver = e.pointerId;
        this.app?.pointer_down(xy[0], xy[1], e.button);
        return;
      }
      if (down.size === 2 && e.pointerType !== 'mouse') {
        // Second finger: end the drag where the first finger is, then pinch.
        const first = driver !== null ? down.get(driver) : undefined;
        if (first) releaseDriver(first[0], first[1], 0);
        pinching = true;
        pinchAnchor = spread();
      }
    });
    on(canvas, 'pointermove', (e) => {
      const xy = local(e);
      if (down.has(e.pointerId)) down.set(e.pointerId, xy);
      if (pinching) {
        const d = spread();
        if (pinchAnchor <= 0 || d <= 0) {
          pinchAnchor = d;
          return;
        }
        // One Gource zoom step per factor of 1.1 in finger distance;
        // spreading the fingers zooms in (a negative wheel delta).
        let steps = Math.log(d / pinchAnchor) / Math.log(ZOOM_STEP);
        while (steps >= 1) {
          this.app?.wheel(-WHEEL_NOTCH_PX);
          pinchAnchor *= ZOOM_STEP;
          steps -= 1;
        }
        while (steps <= -1) {
          this.app?.wheel(WHEEL_NOTCH_PX);
          pinchAnchor /= ZOOM_STEP;
          steps += 1;
        }
        return;
      }
      if (driver !== null ? e.pointerId !== driver : !e.isPrimary || down.size > 0) return;
      this.app?.pointer_move(xy[0], xy[1], e.buttons);
    });
    on(canvas, 'pointerup', (e) => endPointer(e, false));
    on(canvas, 'pointercancel', (e) => endPointer(e, true));
    on(canvas, 'lostpointercapture', (e) => endPointer(e, true));
    on(canvas, 'pointerleave', (e) => {
      if (down.size === 0 && e.isPrimary) this.app?.pointer_leave();
    });
    on(canvas, 'contextmenu', (e) => e.preventDefault());
    on(
      canvas,
      'wheel',
      (e) => {
        e.preventDefault();
        // Normalise to pixels: DOM_DELTA_LINE ≈ 16 px, DOM_DELTA_PAGE = canvas height.
        const scale = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? canvas.clientHeight : 1;
        this.app?.wheel(e.deltaY * scale);
      },
      { passive: false },
    );

    // Keys. Presses reach the engine while the canvas (or nothing in
    // particular) has focus; every forwarded press is released again, on
    // its keyup wherever focus is by then, or when the page loses focus,
    // because the engine polls the arrow keys as held state. Escape is the
    // host's: the engine never sees it.
    const held = new Map<string, string>();
    const keyId = (e: KeyboardEvent) => e.code || e.key;
    const releaseAll = () => {
      const keys = [...held.values()];
      held.clear();
      for (const key of keys) this.app?.key(key, false, false, false);
    };
    const keyDown = (e: KeyboardEvent) => {
      if (!this.running || !this.app || e.metaKey || e.altKey || e.key === 'Escape') return;
      const target = e.target as HTMLElement | null;
      const onCanvas = target === canvas;
      if (!onCanvas && target !== document.body && target !== this && (!this.keyScope || target !== this.keyScope)) return;
      // Tab is Gource's "next user" only on the focused canvas, and Shift+Tab
      // always moves focus, so keyboard users can leave the visualisation.
      if (e.key === 'Tab' && (!onCanvas || e.shiftKey)) return;
      const handled = this.app.key(e.key, true, e.shiftKey, e.ctrlKey);
      held.set(keyId(e), e.key);
      if (handled) {
        e.preventDefault();
        this.emit('gource-key', { key: e.key });
      }
    };
    const keyUp = (e: KeyboardEvent) => {
      const key = held.get(keyId(e));
      if (key === undefined) return;
      held.delete(keyId(e));
      if (this.app?.key(key, false, e.shiftKey, e.ctrlKey)) e.preventDefault();
    };
    on(window, 'keydown', keyDown);
    on(window, 'keyup', keyUp);
    on(window, 'blur', releaseAll);
    on(canvas, 'blur', releaseAll);
    const onVisibility = () => {
      if (document.visibilityState === 'hidden') releaseAll();
    };
    document.addEventListener('visibilitychange', onVisibility);
    this.cleanups.push(() => document.removeEventListener('visibilitychange', onVisibility));

    const observer = new ResizeObserver(() => this.resize());
    observer.observe(canvas);
    this.cleanups.push(() => observer.disconnect());

    // ResizeObserver does not fire when only the device pixel ratio changes
    // (browser zoom, moving to another monitor).
    let mq: MediaQueryList | null = null;
    const watchDpr = () => {
      mq?.removeEventListener('change', onDpr);
      mq = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      mq.addEventListener('change', onDpr);
    };
    const onDpr = () => {
      this.resize();
      watchDpr();
    };
    watchDpr();
    this.cleanups.push(() => mq?.removeEventListener('change', onDpr));
  }
}

export interface GourceViewerElement {
  addEventListener<K extends keyof GourceViewerEventMap>(type: K, listener: (this: GourceViewerElement, ev: GourceViewerEventMap[K]) => unknown, options?: boolean | AddEventListenerOptions): void;
  addEventListener(type: string, listener: EventListenerOrEventListenerObject, options?: boolean | AddEventListenerOptions): void;
  removeEventListener<K extends keyof GourceViewerEventMap>(type: K, listener: (this: GourceViewerElement, ev: GourceViewerEventMap[K]) => unknown, options?: boolean | EventListenerOptions): void;
  removeEventListener(type: string, listener: EventListenerOrEventListenerObject, options?: boolean | EventListenerOptions): void;
}

declare global {
  interface HTMLElementTagNameMap {
    'gource-viewer': GourceViewerElement;
  }
}

if (!customElements.get('gource-viewer')) customElements.define('gource-viewer', GourceViewerElement);
