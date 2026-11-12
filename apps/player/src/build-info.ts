// SPDX-License-Identifier: GPL-3.0-or-later

// Where this build's source is (design rule L5: the hosted page links to the
// exact public tag it was built from). Vite defines the two constants from
// GOURCE_SOURCE_REF / GOURCE_REPO_URL or `git describe`; without Vite (unit
// tests) the defaults apply.

declare const __GOURCE_SOURCE_REF__: string | undefined;
declare const __GOURCE_REPO_URL__: string | undefined;

export const SOURCE_REF: string = typeof __GOURCE_SOURCE_REF__ === 'string' ? __GOURCE_SOURCE_REF__ : 'main';
export const REPO_URL: string =
  typeof __GOURCE_REPO_URL__ === 'string' ? __GOURCE_REPO_URL__ : 'https://github.com/joe-byounghern-kim/gource-online';

/** The exact source of this build, or null while the build names no repository. */
export function sourceUrl(ref: string = SOURCE_REF, repo: string = REPO_URL): string | null {
  if (!repo) return null;
  return `${repo}/tree/${ref.split('/').map(encodeURIComponent).join('/')}`;
}
