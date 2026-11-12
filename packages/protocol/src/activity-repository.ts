// SPDX-License-Identifier: MIT
import { parseRepoUrl } from './repo-url.js';

// Repository boundaries in a multi-repository activity log (My work).
//
// Each repository owns exactly one top-level directory, `/<owner>:<repo>/`, and one
// actor named by its canonical `owner/repo`. ':' never occurs in a GitHub owner or
// repository name, so the directory maps back to exactly one repository, and two
// repositories of the same owner never share a directory node. The repository list
// itself is explicit job data (`ActivityAnalysisState.projects`); a reader uses these
// rules to place a log row, never to discover repositories from paths.
//
// Logs and snapshots made before this rule use `/<owner>/<repo>/` and one actor for
// every repository. `activityRepositoryOf` and `activityRepositoryFilter` accept both.

const canonical = (name: string): boolean => {
  if (typeof name !== 'string' || name.length > 201) return false;
  // Match canonical owner/repo exactly, preserving a literal .git repository suffix.
  const parsed = parseRepoUrl(`https://github.com/${name}.git`);
  return parsed.ok && parsed.repo.host === 'github.com' && parsed.repo.path === name;
};
const checked = (name: string): string => {
  if (!canonical(name)) throw new RangeError('invalid-repository');
  return name;
};

/** `owner/repo` → `owner:repo`, the single top-level directory of that repository. */
export function activityRepositoryDirectory(name: string): string {
  return checked(name).replace('/', ':');
}

/** The actor that carries this repository's activity: its canonical `owner/repo`. */
export function activityRepositoryActor(name: string): string {
  return checked(name);
}

/** Which of `names` a log path belongs to, in either the current or the legacy layout. */
export function activityRepositoryOf(path: string, names: readonly string[]): string | null {
  const relative = path.startsWith('/') ? path.slice(1) : path;
  for (const name of names) {
    if (!canonical(name)) continue;
    if (relative.startsWith(`${activityRepositoryDirectory(name)}/`) || relative.startsWith(`${name}/`)) return name;
  }
  return null;
}

/**
 * A `file-show-filter` regular expression for one repository, in either layout.
 * Saved snapshot settings reject '|', so `[:/]` stands in for an alternation.
 */
export function activityRepositoryFilter(name: string): string {
  // Lenient on purpose: a project button must not throw for a name the service returned.
  if (typeof name !== 'string' || name.length > 201 || !/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(name)) throw new RangeError('invalid-repository');
  const escape = (value: string) => value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const [owner, repo] = name.split('/') as [string, string];
  return `^/?${escape(owner)}[:/]${escape(repo)}/`;
}
