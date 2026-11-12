// SPDX-License-Identifier: GPL-3.0-or-later
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import { CSP as CLI_CSP } from '../apps/cli/src/server.js';

// Runs before the packages are built, so read the MIT constant from its source.
const source = readFileSync(new URL('../packages/protocol/src/csp.ts', import.meta.url), 'utf8');
const declaration = /export const GOURCE_BASE_CSP =\s*((?:"[^"]*"\s*\+?\s*)+);/.exec(source);
const shared = declaration ? [...declaration[1].matchAll(/"([^"]*)"/g)].map((part) => part[1]).join('') : '';
// The CLI serves the player (apps/player): the shared policy as the player widens it with no senders and no log
// sources, which only closes form-action (apps/player/src/headers.ts, playerCsp).
const player = readFileSync(new URL('../apps/player/src/headers.ts', import.meta.url), 'utf8');

test('the CLI server sends the player\'s policy: the shared one, with form-action closed', () => {
  assert.ok(shared, 'GOURCE_BASE_CSP is declared as string literals');
  assert.match(player, /return replaceDirective\(policy, "form-action 'self'", "form-action 'none'"\);/);
  assert.equal(CLI_CSP, shared.replace("form-action 'self'", "form-action 'none'"));
});
