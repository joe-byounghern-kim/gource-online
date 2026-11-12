// SPDX-License-Identifier: GPL-3.0-or-later
import { test as base, expect } from '@playwright/test';

/**
 * Each test gets its own browser process, not only its own context. On CI, once other real-engine pages had run
 * in a worker's shared browser, creating the next WebGL2 context took seconds and the first viewer check missed
 * (#58). The launch options, channel, base URL and viewport are the configured ones; no check or timeout changes.
 */
export const test = base.extend({
  context: async ({ playwright, browserName, launchOptions, channel, headless, baseURL, viewport }, use) => {
    const browser = await playwright[browserName].launch({ ...launchOptions, ...(channel ? { channel } : {}), headless });
    try {
      const context = await browser.newContext({ baseURL, viewport });
      try { await use(context); } finally { await context.close(); }
    } finally { await browser.close(); }
  },
});
export { expect };
