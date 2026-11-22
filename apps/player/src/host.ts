// SPDX-License-Identifier: GPL-3.0-or-later

// What the player's screens report to, and the hook a sign-out runs. The
// player plugs in its events for the page that opened or embeds it
// (./events.ts); it sends no analytics itself and has no sign-in.

import type { AnalyticsEvent, PropValue } from '@gource-online/protocol';

export interface PlayerHost {
  /** Records a product event (the recording's start, end or failure). */
  track(event: AnalyticsEvent, props?: Record<string, PropValue>): void;
  /** Runs `hook` before a private run ends (a sign-out), so a recording can stop and save. Returns its removal. */
  onRunEnding(hook: () => Promise<void> | void): () => void;
}

const host: PlayerHost = {
  track: () => {},
  onRunEnding: () => () => {},
};

/** The hosting page's hooks; the defaults do nothing. */
export const playerHost: Readonly<PlayerHost> = host;

export function configurePlayerHost(hooks: Partial<PlayerHost>): void {
  Object.assign(host, hooks);
}
