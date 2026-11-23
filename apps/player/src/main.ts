// SPDX-License-Identifier: GPL-3.0-or-later

// The standalone Gource player (issue #122). It plays one log at a time,
// handed over by URL or postMessage (./handoff.ts) or opened from a local
// file, on the viewer screen: playback,
// options, recording and the AI statistics. It has no sign-in, no account
// and no paid features, and reads no cookies. It requests nothing but the
// log it plays; only the page that opened or embeds it hears how playback
// went (./events.ts).

import { playerUrl } from '@gource-online/protocol';
import { EngineMissingError, LogLoader, loadEngine, loadGourceFont, type GourceApp } from '@gource-online/viewer';
import { isDefault } from '@gource-online/viewer/options';
import { ViewerScreen } from './screens/viewer';
import './styles.css';
import { messageHandoff, senderWindow, urlHandoff, type Handoff, type PlayerLog } from './handoff';
import { PlaybackEvents, senderEvents } from './events';
import { configurePlayerHost } from './host';
import type { PlayerConfig } from './headers';
import type { PlaybackState } from './playback';
import { logSource } from './record/source';
import { keepSettingsInMemory } from './saved-settings';
import { hostedSiteUrl, sourceUrl } from './build-info';
import { icon } from './dom';
import { mark, startScreen } from './start';
import { clearTestHook, exposeEngine } from './test-hooks';

declare const __PLAYER_CONFIG__: PlayerConfig;
/** The placeholder engine (`?mock=1`) exists in development and test builds only. */
declare const __PLAYER_MOCK__: boolean;

const CONFIG = __PLAYER_CONFIG__;
const BASE = import.meta.env.BASE_URL;
const MOCK = __PLAYER_MOCK__ && new URLSearchParams(window.location.search).has('mock');
/** Embedded in another page: keyboard focus stays where that page put it. */
const EMBEDDED = window.parent !== window;
const fontReady = loadGourceFont(`${BASE}fonts/FreeSans.ttf`);
/** The page that opened or embeds the player: the only one that may hand it a log, and hear how playback went. */
const sender = senderWindow(window);
const toSender = senderEvents(sender, CONFIG.senders);
configurePlayerHost({ track: toSender.track });

type State = 'idle' | 'waiting' | 'loading' | 'playing' | 'failed';

function el<K extends keyof HTMLElementTagNameMap>(tag: K, props: Record<string, string | boolean> = {}, ...children: Array<Node | string>): HTMLElementTagNameMap[K] {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

/** The player is free software: its licence, the exact source of this build and the third-party notices. */
function licenceNote(): HTMLElement {
  const source = sourceUrl();
  return el(
    'p',
    { className: 'player-licence' },
    'Free software under the ',
    el('a', { href: `${BASE}COPYING.txt`, target: '_blank', rel: 'noopener', textContent: 'GPL-3.0-or-later' }),
    '. ',
    ...(source ? [el('a', { href: source, target: '_blank', rel: 'noopener', textContent: 'Source code' }), ' · '] : []),
    el('a', { href: `${BASE}THIRD_PARTY_LICENSES.txt`, target: '_blank', rel: 'noopener', textContent: 'Third-party licences' }),
  );
}

const title = el('h1', { className: 'player-title', textContent: 'Gource player' });
const status = el('p', { className: 'player-status', role: 'status' });
const fileInput = el('input', { type: 'file', className: 'player-file-input', accept: '.log,.gitlog,.txt,text/plain' });
/** While a log loads or a page hands one over, the start screen is hidden: this opens a local file instead. */
const openLabel = el('label', { className: 'player-open btn btn-small' }, icon('upload'), 'Open a log file', fileInput);
const banner = el('p', { className: 'player-banner', role: 'alert', hidden: true });
const start = startScreen({ site: hostedSiteUrl(CONFIG.senders), embedded: EMBEDDED, licence: licenceNote(), open: () => fileInput.click() });
const empty = start.el;
// Shown only once the player knows no log is coming (setState), so a log by link or by a page never flashes it.
empty.hidden = true;
/** The start screen: open a file, wait for a page's log, or say what went wrong. The viewer replaces it while a log plays. */
const main = el(
  'main',
  { className: 'player' },
  el('header', { className: 'player-bar' }, el('div', { className: 'player-brand' }, mark(), title), status, openLabel),
  banner,
  empty,
);
document.getElementById('app')!.replaceWith(main);

interface Run {
  viewer: ViewerScreen;
  loader: LogLoader | null;
  cancelled: boolean;
  /** The viewer is on screen. */
  shown: boolean;
  events: PlaybackEvents;
}
let run: Run | null = null;
let pending: { cancel(): void } | null = null;
/** The log by URL that plays, so option changes can keep its link up to date. */
let linked: PlayerLog | null = null;
/** The last link to `linked` the player accepts: the page goes back to it when a refused link replaces it. */
let lastLink = '';

function setState(state: State): void {
  main.dataset.state = state;
  main.hidden = state === 'playing';
  // While a log loads, or a page may still hand one over, the bar's button opens a file instead (styles.css).
  empty.hidden = state !== 'idle' && state !== 'failed';
}

function notice(message: string): void {
  banner.textContent = message;
  banner.hidden = false;
}

function stop(): void {
  if (!run) return;
  run.events.end();
  run.cancelled = true;
  run.loader?.cancel();
  run.viewer.dispose();
  run.viewer.el.remove();
  clearTestHook();
  run = null;
  linked = null;
  keepSettingsInMemory(false);
}

function reset(): void {
  banner.hidden = true;
  status.textContent = '';
  title.textContent = 'Gource player';
  document.title = 'Gource player';
}

/** The log cannot play: back to the start screen, with the way to open a local file. */
function fail(message: string): void {
  stop();
  reset();
  notice(message);
  setState('failed');
}

/**
 * A log the player's own server on this computer serves (`npx gource-online`: 127.0.0.1, a per-run token): its link
 * works only here, while the command runs.
 */
function onThisComputer(log: PlayerLog): boolean {
  if (log.source.kind !== 'url') return false;
  const url = new URL(log.source.url);
  return url.origin === window.location.origin && ['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname);
}

/** This page without its fragment. */
const page = () => `${window.location.origin}${window.location.pathname}${window.location.search}`;

/** The options a link carries: those that differ from Gource's defaults. */
function linkOptions(values: ReadonlyMap<string, string>): Record<string, string> {
  return Object.fromEntries([...values].filter(([name, value]) => !isDefault(name, value)));
}

/**
 * The share link of a log by URL with these option values, built by the handoff protocol, so it is one the
 * player itself accepts; null when the view does not fit the protocol's limits (a value over 1024 characters,
 * more than 64 options, a fragment over 16384 characters).
 */
function linkFor(log: PlayerLog, values: ReadonlyMap<string, string>): string | null {
  if (log.source.kind !== 'url') return null;
  try {
    return playerUrl(page(), { src: log.source.url, ...(log.title === undefined ? {} : { title: log.title }), options: linkOptions(values) }, CONFIG.sources);
  } catch {
    return null;
  }
}

/** The link's length for the style hint, whether or not it fits. */
function linkLength(log: PlayerLog, values: ReadonlyMap<string, string>): number {
  const params = new URLSearchParams({ src: log.source.kind === 'url' ? log.source.url : '' });
  if (log.title !== undefined) params.set('title', log.title);
  for (const [name, value] of Object.entries(linkOptions(values))) params.set(`o.${name}`, value);
  return page().length + 1 + params.toString().length;
}

/** `from`: a log by URL, one a page handed over, or a local file. */
function play(log: PlayerLog, from: 'url' | 'message' | 'file'): void {
  pending?.cancel();
  pending = null;
  stop();
  banner.hidden = true;
  const name = log.title ?? (log.source.kind === 'file' ? log.source.name : 'Gource player');
  title.textContent = name;
  // A log a page handed over can be private: its name stays out of the browser's history, and its settings out of storage.
  document.title = from === 'message' || name === 'Gource player' ? 'Gource player' : `${name} · Gource player`;
  keepSettingsInMemory(from === 'message');
  status.textContent = 'Loading…';
  setState('loading');

  const byUrl = from === 'url';
  let tooLong = false;
  const viewer = new ViewerScreen({
    display: name,
    host: from === 'file' ? 'local' : 'log',
    options: { ...log.options },
    // Only a log by URL has a link; a handed-over or local log is on no server. A log on this computer
    // (`npx gource-online`, served on 127.0.0.1 with a per-run token) has a link that works nowhere else.
    shareable: byUrl && !onThisComputer(log),
    backLabel: 'Open another log',
    // A log a page handed over may be private: its recording has no default title, is named `gource` and warns.
    recordSource: log.source.kind === 'url' ? logSource('url', log.source.url) : { ...logSource('file', log.source.name), private: from === 'message' },
    mock: MOCK,
    about: licenceNote(),
    ...(byUrl ? { linkLength: (values: ReadonlyMap<string, string>) => linkLength(log, values) } : {}),
    onOptionsChanged: (values) => {
      if (linked !== log) return;
      // Keeps the link equal to the view; replaceState fires no hashchange. A view that no longer fits a link keeps
      // the last one that did, so the page never holds a link the player would refuse.
      const link = linkFor(log, values);
      if (link === null) {
        if (!tooLong) viewer.showBanner('This view does not fit in a link: the link keeps the last view that did.');
        tooLong = true;
        return;
      }
      tooLong = false;
      lastLink = link;
      if (link !== window.location.href) history.replaceState(null, '', link);
    },
    onBack: () => {
      // The page's link would play this log again on reload.
      if (linked === log) history.replaceState(null, '', page());
      stop();
      reset();
      setState('idle');
      start.openButton.focus();
    },
    onRetry: () => play(log, from),
  });
  const events = new PlaybackEvents(toSender.track, from);
  const current: Run = { viewer, loader: null, cancelled: false, shown: false, events };
  run = current;
  if (byUrl) {
    linked = log;
    lastLink = window.location.href;
  }
  viewer.el.hidden = true;
  document.body.append(viewer.el);

  const state: PlaybackState = {
    display: name,
    host: 'local',
    source: 'local',
    private: false,
    progress: {},
    loading: {
      started: true, bytes: 0, commits: 0, finished: false,
      ...(log.source.kind === 'file' ? { totalBytes: log.source.file.size } : {}),
    },
    ready: false,
  };
  const ready = () => {
    if (state.ready) return;
    state.ready = true;
    current.shown = true;
    viewer.el.hidden = false;
    setState('playing');
    viewer.show(!EMBEDDED);
    events.shown(() => viewer.currentStatus());
  };

  void (async () => {
    let engine: GourceApp;
    try {
      await fontReady;
      const factory = await loadEngine(MOCK);
      if (current.cancelled) return;
      engine = factory.createApp(viewer.canvas);
      exposeEngine(engine);
    } catch (err) {
      if (current.cancelled) return;
      fail(err instanceof EngineMissingError ? err.message : `The visualisation could not start: ${err instanceof Error ? err.message : String(err)}`);
      return;
    }
    viewer.attach(engine);
    current.loader = new LogLoader(engine, MOCK, {
      progress: (progress, playable) => {
        state.loading.bytes = progress.bytes;
        state.loading.commits = progress.commits;
        if (playable) ready();
        viewer.updateLoading(state);
      },
      done: (done) => {
        Object.assign(state.loading, { bytes: done.bytes, commits: done.commits, finished: true, format: done.format });
        events.loaded(done.bytes, done.commits);
        ready();
        viewer.updateLoading(state);
      },
      empty: () => fail('No commit could be read from this log.'),
      error: (error) => {
        // Before anything plays there is nothing to keep; after, the viewer keeps what arrived and offers a retry.
        if (!state.ready) fail(error.message);
        else {
          state.error = error;
          viewer.updateLoading(state);
        }
      },
    });
    current.loader.start(log.source);
  })();
}

function take(handoff: Handoff, from: 'url' | 'message'): void {
  if (handoff.kind === 'log') {
    // A page handed this log over: only that page's origin hears how it plays.
    if (handoff.origin) toSender.only(handoff.origin);
    play(handoff.log, from);
  }
  else if (handoff.kind === 'failed') fail(handoff.message);
  else setState('idle');
}

function openFile(file: File | undefined): void {
  if (file) play({ source: { kind: 'file', file, name: file.name }, options: {} }, 'file');
}
fileInput.addEventListener('change', () => {
  openFile(fileInput.files?.[0]);
  fileInput.value = '';
});
// A file dragged over the start screen lights up where it can go.
let dragDepth = 0;
const dragEnd = () => {
  dragDepth = 0;
  delete main.dataset.dragging;
};
document.addEventListener('dragenter', (event) => {
  if (!event.dataTransfer?.types.includes('Files')) return;
  dragDepth += 1;
  main.dataset.dragging = '';
});
document.addEventListener('dragleave', () => {
  dragDepth -= 1;
  if (dragDepth <= 0) dragEnd();
});
document.addEventListener('dragover', (event) => event.preventDefault());
document.addEventListener('drop', (event) => {
  event.preventDefault();
  dragEnd();
  openFile(event.dataTransfer?.files[0]);
});

// A link to another log on the same page only changes the fragment: play that one.
window.addEventListener('hashchange', () => {
  const next = urlHandoff(window.location, CONFIG);
  // A bad link does not stop a log that is already playing, nor a handoff under way.
  if (next?.kind === 'failed' && (run || pending)) {
    // The page keeps no link the player refuses: back to the log that plays, or to no link while a page hands one over.
    history.replaceState(null, '', linked ? lastLink : page());
    if (run?.shown) run.viewer.showBanner(next.message);
    else notice(next.message);
  } else if (next) take(next, 'url');
});

// The page closes or goes elsewhere: the log stops playing.
window.addEventListener('pagehide', () => run?.events.end());

const byUrl = urlHandoff(window.location, CONFIG);
if (byUrl) {
  // No log comes from the page that opened this window, and no page this window goes to may reach back to it.
  // The player still tells that page how playback went, through `sender`.
  window.opener = null;
  take(byUrl, 'url');
} else if (sender && CONFIG.senders.length) {
  setState('waiting');
  status.textContent = EMBEDDED ? 'Waiting for the log from this page…' : 'Waiting for the log from the page that opened this player…';
  pending = messageHandoff(window, sender, CONFIG, (result) => {
    pending = null;
    status.textContent = '';
    take(result, 'message');
  });
} else {
  setState('idle');
}
