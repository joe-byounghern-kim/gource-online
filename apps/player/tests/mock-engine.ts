// SPDX-License-Identifier: GPL-3.0-or-later

// What the specs on the placeholder engine share: a small log, the engine's call log and the player's address.

import type { Page } from '@playwright/test';

import type { MockCall } from '@gource-online/viewer/engine/mock';

export const SMALL_LOG = [
  '1230000000|Andrew Caudwell|A|/src/gource.cpp',
  '1230000000|Andrew Caudwell|A|/src/gource.h',
  '1230000000|Andrew Caudwell|A|/README',
  '1230086400|Alice|A|/data/shaders/bloom.frag',
  '1230086400|Alice|A|/data/shaders/bloom.vert',
  '1230172800|Bob|M|/src/gource.cpp',
  '1230172800|Bob|A|/src/core/display.cpp',
  '1230259200|Andrew Caudwell|A|/docs/manual.md',
  '1230345600|Alice|M|/data/shaders/bloom.frag',
  '1230432000|Carol|A|/tests/logs/simple.log',
  '1230432000|Carol|A|/tests/test.conf',
  '1230518400|Bob|D|/README',
  '',
].join('\n');

export const SMALL_LOG_COMMITS = 7;

/** A path of this player that plays `log`: the page's query (the test switches) and the fragment the player reads. */
export function playing(log: string, options: Record<string, string> = {}, query = 'mock=1'): string {
  const fragment = new URLSearchParams({ src: log });
  for (const [name, value] of Object.entries(options)) fragment.set(`o.${name}`, value);
  return `/?${query}#${fragment}`;
}

/** `playing` with options written as `&name=value`, a repeated name joined by lines; for the real engine's test build. */
export function realLog(log: string, extra: string, query: string): string {
  const params = new URLSearchParams(extra);
  const options = Object.fromEntries([...new Set(params.keys())].map((name) => [name, params.getAll(name).join('\n')]));
  return playing(log, options, query);
}

export async function engineCalls(page: Page, method?: string): Promise<MockCall[]> {
  const calls = await page.evaluate(() => window.__gourceMock?.calls ?? []);
  return method ? calls.filter((c) => c.method === method) : calls;
}

export async function setOptionCalls(page: Page): Promise<Array<[string, string]>> {
  return (await engineCalls(page, 'set_option')).map((c) => c.args as [string, string]);
}
