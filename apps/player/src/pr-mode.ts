// SPDX-License-Identifier: GPL-3.0-or-later

// What a recording's fixed layout locks of pull request mode.

import { normaliseHide } from '@gource-online/viewer/options';

/** Reject layout/source changes before mutating the engine while F3 owns its fork. */
export function prRecordingLocked(name: string, value: string, currentHide: string): boolean {
  if (name === 'pull-request-mode' || name === 'hide-pull-requests') return true;
  if (name !== 'hide') return false;
  const hidden = (text: string) => normaliseHide(text.split(',')).includes('pull-requests');
  return hidden(value) !== hidden(currentHide);
}
