// SPDX-License-Identifier: GPL-3.0-or-later

// Where this build's source is (design rule L5: the page links the exact
// public tag it was built from). Vite defines the constants from
// GOURCE_SOURCE_REF / GOURCE_REPO_URL or `git describe` (vite.config.ts);
// without Vite (unit tests) the defaults apply.

declare const __GOURCE_SOURCE_REF__: string | undefined;
declare const __GOURCE_REPO_URL__: string | undefined;
declare const __GOURCE_CLI_VERSION__: string | undefined;

/** The public repository: the player's source as released. */
export const PUBLIC_REPO_URL = 'https://github.com/joe-byounghern-kim/gource-online';

/** The hosted site, when a build names none of its own. */
export const DEFAULT_SITE_URL = 'https://gource.online';

/** A release tag, `v<version>`: the refs the public repository is sure to have. */
export const RELEASE_TAG = /^v\d+\.\d+\.\d+(?:-[0-9A-Za-z.]+)?$/;

export const SOURCE_REF: string = typeof __GOURCE_SOURCE_REF__ === 'string' ? __GOURCE_SOURCE_REF__ : '';
export const REPO_URL: string = typeof __GOURCE_REPO_URL__ === 'string' ? __GOURCE_REPO_URL__ : PUBLIC_REPO_URL;
/** The version of `npx gource-online` this player belongs to (apps/cli/package.json). */
export const CLI_VERSION: string = typeof __GOURCE_CLI_VERSION__ === 'string' ? __GOURCE_CLI_VERSION__ : '';

/**
 * The ref a build links: the one it is given (GOURCE_SOURCE_REF), else the release tag its commit has, else none.
 * A branch or a commit of a checkout is not used: it may exist only in a private repository.
 */
export function buildSourceRef(given: string | undefined, exactTag: () => string): string {
  if (given) return given;
  try {
    const tag = exactTag().trim();
    return RELEASE_TAG.test(tag) ? tag : '';
  } catch {
    return '';
  }
}

/**
 * The source of this build: the tree of its ref, or the repository's front page when it has none. In the public
 * repository only a release tag is linked, so the link never names a branch or commit that is not public. Null
 * while the build names no repository.
 */
export function sourceUrl(ref: string = SOURCE_REF, repo: string = REPO_URL): string | null {
  if (!repo) return null;
  if (!ref || (repo === PUBLIC_REPO_URL && !RELEASE_TAG.test(ref))) return repo;
  return `${repo}/tree/${ref.split('/').map(encodeURIComponent).join('/')}`;
}

/**
 * The hosted site to point people to: the gource.online site this build lets hand over logs (a staging build names
 * staging), else gource.online. A self-hosted build's own pages are not that site.
 */
export function hostedSiteUrl(senders: readonly string[]): string {
  return (
    senders.find((origin) => {
      const url = new URL(origin);
      return url.protocol === 'https:' && (url.hostname === 'gource.online' || url.hostname.endsWith('.gource.online'));
    }) ?? DEFAULT_SITE_URL
  );
}

/** The one-step command, pinned to this build's version: npm publishes pre-releases under `beta`, not `latest`. */
export function npxCommand(version: string = CLI_VERSION): string {
  return `npx -y gource-online@${version || 'beta'}`;
}
