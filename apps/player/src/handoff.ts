// SPDX-License-Identifier: GPL-3.0-or-later

// How the player gets its log (packages/protocol/src/player.ts, issue #123):
// - by URL: `#src=<log URL>` in the fragment, fetched without credentials;
// - by postMessage: from the page that opened or embeds the player, checked
//   against the exact window and the configured sender origins;
// - or neither: the person opens a local file.

import {
  PLAYER_HANDOFF_TIMEOUT_MS,
  PLAYER_LOG_FETCH,
  PlayerHandoffReceiver,
  parsePlayerFragment,
  playerMessage,
  playerOriginAllowed,
  type PlayerErrorCode,
  type PlayerOptions,
} from '@gource-online/protocol';
import type { LogSource } from '@gource-online/viewer';
import type { PlayerConfig } from './headers';

export interface PlayerLog {
  source: LogSource;
  title?: string;
  options: PlayerOptions;
}

/** `none`: no sender spoke before the timeout, so the person opens a local file instead. */
export type Handoff = { kind: 'log'; log: PlayerLog } | { kind: 'failed'; message: string } | { kind: 'none' };

const FAILURES: Record<PlayerErrorCode, string> = {
  version: 'The page that opened the player uses another version of the player protocol.',
  invalid: 'The page that opened the player sent a message the player does not understand.',
  size: 'The log did not arrive in the sizes the page announced.',
  order: 'The log did not arrive in order.',
  timeout: `The log did not arrive within ${PLAYER_HANDOFF_TIMEOUT_MS / 1000} seconds.`,
  cancelled: 'The page that opened the player cancelled the handoff.',
};

/** The log named in the page's fragment, a failure for a fragment the player refuses, or null when there is none. */
export function urlHandoff(location: Location, config: PlayerConfig): Handoff | null {
  if (location.hash.length <= 1) return null;
  try {
    const handoff = parsePlayerFragment(location.hash, location.href, config.sources);
    return {
      kind: 'log',
      log: {
        source: { kind: 'url', url: handoff.src, fetch: PLAYER_LOG_FETCH },
        ...(handoff.title === undefined ? {} : { title: handoff.title }),
        options: handoff.options,
      },
    };
  } catch {
    return {
      kind: 'failed',
      message: 'This player link is not valid: the log must come from an allowed origin, and the link must have only src, title and o.<option> values.',
    };
  }
}

/** The window that may hand the player a log: the one that opened it, else the one that embeds it. */
export function senderWindow(win: Window): Window | null {
  if (win.opener && win.opener !== win) return win.opener as Window;
  if (win.parent !== win) return win.parent;
  return null;
}

/**
 * Says `ready` to `sender` and takes one log from it. Only messages from that exact window, and from an
 * allowed origin, count; the first such message fixes the origin for the rest of the handoff.
 * The answer (`ack` or `error`) goes to that origin only. `done` is called once, in the same task as the
 * `ack`, so a log the sender was told is playing is never dropped. `cancel` tells the sender with `cancelled`.
 */
export function messageHandoff(win: Window, sender: Window, config: PlayerConfig, done: (handoff: Handoff) => void): { cancel(): void } {
  const receiver = new PlayerHandoffReceiver();
  let origin: string | null = null;
  let open = true;

  // Nothing more is said to the page that opened this window, and no page this window goes to may reach back to it.
  const close = () => {
    open = false;
    win.removeEventListener('message', onMessage);
    win.clearTimeout(timer);
    win.opener = null;
  };
  const finish = (handoff: Handoff) => {
    if (!open) return;
    close();
    done(handoff);
  };
  const answer = (code: PlayerErrorCode, id?: string) => {
    const message = playerMessage({ type: 'error', code, ...(id === undefined ? {} : { id }) });
    // Before any sender spoke there is no origin yet; an error carries no data, so each allowed one may hear it.
    for (const target of origin ? [origin] : config.senders) sender.postMessage(message, target);
  };
  const onMessage = (event: MessageEvent) => {
    if (event.source !== sender || !playerOriginAllowed(event.origin, config.senders)) return;
    if (origin !== null && event.origin !== origin) return;
    const outcome = receiver.receive(event.data);
    if (outcome.kind === 'ignored') return;
    origin = event.origin;
    if (outcome.kind === 'error') {
      // A sender's own error ends the handoff without an answer.
      if ((event.data as { type?: unknown }).type !== 'error') answer(outcome.code, outcome.id);
      finish({ kind: 'failed', message: FAILURES[outcome.code] });
    } else if (outcome.kind === 'complete') {
      sender.postMessage(playerMessage({ type: 'ack', id: outcome.id }), origin);
      finish({
        kind: 'log',
        log: {
          source: { kind: 'file', file: new Blob([outcome.log as Uint8Array<ArrayBuffer>]), name: outcome.title ?? 'log' },
          ...(outcome.title === undefined ? {} : { title: outcome.title }),
          options: outcome.options,
        },
      });
    }
  };

  win.addEventListener('message', onMessage);
  const timer = win.setTimeout(() => {
    const outcome = receiver.timeout();
    if (outcome.kind !== 'error') return;
    answer(outcome.code, outcome.id);
    // A page that opened the player without ever speaking (not a sender, or one that only opens it) is no failure.
    finish(origin === null ? { kind: 'none' } : { kind: 'failed', message: FAILURES[outcome.code] });
  }, PLAYER_HANDOFF_TIMEOUT_MS);
  // `ready` carries nothing, so it may go to each allowed origin in turn; only the sender's own is delivered.
  for (const target of config.senders) sender.postMessage(playerMessage({ type: 'ready' }), target);
  return {
    cancel: () => {
      if (!open) return;
      const outcome = receiver.cancel();
      if (outcome.kind === 'error') answer(outcome.code, outcome.id);
      close();
    },
  };
}
