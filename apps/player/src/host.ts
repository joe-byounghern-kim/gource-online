// SPDX-License-Identifier: GPL-3.0-or-later

// What a page that shows the player's screens may plug in. The standalone
// player plugs in nothing: it sends no analytics and has no sign-in. The
// page that embeds these screens connects its own.

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
