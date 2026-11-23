// SPDX-License-Identifier: GPL-3.0-or-later

// How playback went, for the page that opened or embeds the player (the protocol's `event` message):
// a log opened, playback started and ended, a recording started, finished or failed. Counts and sizes
// only, never a name, a path or the log. Only a page at one of the build's sender origins hears it,
// and that page decides whether to count it. With no such page (a link opened on its own,
// `npx gource-online`) nothing goes anywhere. The player makes no request for it.

import { parsePlayerMessage, playerMessage, sizeBucket, type AnalyticsEvent, type PropValue } from '@gource-online/protocol';
import type { EngineStatus } from '@gource-online/viewer';

export type Track = (event: AnalyticsEvent, props?: Record<string, PropValue>) => void;

/** How long `playback_started` samples the frame rate, and how often. */
export const FPS_SAMPLE_MS = 5000;
const FPS_EVERY_MS = 500;

export interface SenderEvents {
  track: Track;
  /** From now on only `origin`, one of the allowed ones, hears the events: the origin a handoff came from. */
  only(origin: string): void;
}

/**
 * Posts each event to `sender`, with each allowed origin in turn as `targetOrigin`: the browser delivers it only
 * at the sender's own, so a page at any other origin never hears it. No sender or no allowed origin: nothing.
 * An event the protocol would refuse is never sent.
 */
export function senderEvents(sender: Window | null, senders: readonly string[]): SenderEvents {
  let targets = sender ? senders : [];
  return {
    track: (event, props = {}) => {
      const message = playerMessage({ type: 'event', event, props });
      const parsed = parsePlayerMessage(message);
      if (!sender || !parsed || !('message' in parsed)) return;
      for (const origin of targets) {
        try {
          sender.postMessage(message, origin);
        } catch {
          // A page that is gone hears nothing; playback goes on.
        }
      }
    },
    only: (origin) => {
      if (targets.includes(origin)) targets = [origin];
    },
  };
}

/** A count to two significant figures (1234 → 1200), so an exact commit count never identifies a log. */
export function roughCount(count: number): number {
  if (!Number.isFinite(count) || count <= 0) return 0;
  if (count < 100) return Math.round(count);
  const unit = 10 ** (Math.floor(Math.log10(count)) - 1);
  return Math.round(count / unit) * unit;
}

/** A failure's code as an event may carry it: a short snake_case code, else `internal`. */
export function failureCode(code: unknown): string {
  return typeof code === 'string' && /^[a-z][a-z0-9_]{0,63}$/.test(code) ? code : 'internal';
}

/** The events of one log, each at most once. `from`: by link, handed over by a page, or a local file. */
export class PlaybackEvents {
  private readonly startedAt = performance.now();
  private opened = false;
  private shownAt: number | null = null;
  private sampler: number | undefined;
  private readonly fps: number[] = [];

  constructor(
    private readonly track: Track,
    private readonly from: 'url' | 'message' | 'file',
  ) {}

  /** The whole log is read. */
  loaded(bytes: number, commits: number): void {
    if (this.opened) return;
    this.opened = true;
    this.track('log_opened', { source: this.from, log_size: sizeBucket(bytes), commits: roughCount(commits) });
  }

  /** The viewer is on screen: the time it took, then the median frame rate over its first seconds. */
  shown(status: () => EngineStatus | null): void {
    if (this.shownAt !== null) return;
    const shownAt = performance.now();
    this.shownAt = shownAt;
    this.sampler = window.setInterval(() => {
      const fps = status()?.fps;
      if (typeof fps === 'number' && Number.isFinite(fps)) this.fps.push(fps);
      if (performance.now() - shownAt >= FPS_SAMPLE_MS) this.started(shownAt);
    }, FPS_EVERY_MS);
  }

  /** The log stops playing: another log, back to the start screen, or the page goes away. */
  end(): void {
    const shownAt = this.shownAt;
    if (shownAt === null) return;
    this.shownAt = null;
    if (this.sampler !== undefined) this.started(shownAt);
    this.track('playback_ended', { watched_s: Math.round((performance.now() - shownAt) / 1000) });
  }

  private started(shownAt: number): void {
    window.clearInterval(this.sampler);
    this.sampler = undefined;
    const props: Record<string, number> = { first_frame_ms: Math.round(shownAt - this.startedAt) };
    const sorted = [...this.fps].sort((a, b) => a - b);
    if (sorted.length) props.fps_p50_5s = sorted[Math.floor(sorted.length / 2)]!;
    this.track('playback_started', props);
  }
}
