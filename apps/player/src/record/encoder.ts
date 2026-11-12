// SPDX-License-Identifier: GPL-3.0-or-later

// Encoder support ladder (REC-13, REC-49, REC-52): the plan lists the
// candidates in order, `isConfigSupported` is asked with the exact
// configuration that will be used, and the first accepted one wins. The
// dialog shows its note when it differs from the request; nothing is
// downgraded silently after Record is pressed.

import type { EncoderCallbacks, EncoderLike, LadderCandidate } from './types';

/** The exact `VideoEncoderConfig` of a candidate (M0 spike defaults). */
export function candidateConfig(c: LadderCandidate): VideoEncoderConfig {
  return {
    codec: c.codec,
    width: c.width,
    height: c.height,
    bitrate: c.bitrate,
    framerate: c.framerate,
    hardwareAcceleration: c.hardware_acceleration,
    bitrateMode: c.bitrate_mode,
    latencyMode: c.latency_mode,
    avc: { format: 'avc' },
  };
}

export interface LadderChoice {
  candidate: LadderCandidate;
  index: number;
}

/** First candidate the browser accepts, or null (`record_no_encoder`). */
export async function chooseCandidate(
  ladder: readonly LadderCandidate[],
  isSupported: (config: VideoEncoderConfig) => Promise<boolean>,
): Promise<LadderChoice | null> {
  for (let i = 0; i < ladder.length; i++) {
    const candidate = ladder[i]!;
    let ok = false;
    try {
      ok = await isSupported(candidateConfig(candidate));
    } catch {
      ok = false;
    }
    if (ok) return { candidate, index: i };
  }
  return null;
}

/** What the UI may say about the encoder: the request, never what was used (REC-13). */
export function hardwareWords(h: HardwareAcceleration): string {
  switch (h) {
    case 'prefer-hardware':
      return 'hardware preferred';
    case 'prefer-software':
      return 'software preferred';
    default:
      return 'automatic';
  }
}

export function profileName(codec: string): string {
  if (codec.startsWith('avc1.64')) return 'H.264 High';
  if (codec.startsWith('avc1.4d')) return 'H.264 Main';
  if (codec.startsWith('avc1.42')) return 'H.264 Baseline';
  return codec;
}

/** Whether this browser can record at all (REC-4, feature detection only). */
export function recordingSupported(): boolean {
  return typeof window !== 'undefined' && window.isSecureContext && 'VideoEncoder' in window && 'VideoFrame' in window;
}

/** A real WebCodecs encoder. */
export function webCodecsEncoder(callbacks: EncoderCallbacks): EncoderLike {
  return new VideoEncoder({ output: callbacks.output, error: callbacks.error });
}

export async function webCodecsSupported(config: VideoEncoderConfig): Promise<boolean> {
  const r = await VideoEncoder.isConfigSupported(config);
  return r.supported === true;
}
