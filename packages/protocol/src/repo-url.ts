// SPDX-License-Identifier: MIT

// Repository URL and analyze-option validation: exactly what a Gource Online
// clone service accepts, with its error codes. Clients reject the rest in
// place. The shared vectors in vectors/repo-urls.json pin the behaviour.
//
//   - forms: `https://host/path`, `http://host/path`, `host/path`,
//     `git@host:path`, `ssh://git@host/path`; optional `www.`, `.git`
//     suffix, one trailing slash, query string and fragment
//   - hosts: exactly github.com and gitlab.com
//   - GitHub: `owner/repo` plus `/tree/<branch>`; GitLab:
//     `group[/subgroup...]/repo` plus `/-/tree/<branch>`
//   - rejected: credentials, ports, other schemes, empty, `.` and `..`
//     segments, characters outside `[A-Za-z0-9_.-]`

export type RepoHost = 'github.com' | 'gitlab.com';

export interface RepoRef {
  host: RepoHost;
  /** `owner/repo` or `group/subgroup/repo`, without `.git`. */
  path: string;
  /** Branch taken from a `/tree/<branch>` link. */
  branch?: string;
}

/** The clone service's 400 error codes for a repository URL. */
export type RepoErrorCode = 'invalid_url' | 'unsupported_host';

export type RepoParse = { ok: true; repo: RepoRef } | { ok: false; code: RepoErrorCode; message: string };

const MAX_URL_BYTES = 2048;
const MAX_GITLAB_DEPTH = 20;

class RepoError extends Error {
  constructor(
    readonly code: RepoErrorCode,
    message: string,
  ) {
    super(message);
  }
}

const invalid = (message: string) => new RepoError('invalid_url', message);
const utf8 = new TextEncoder();
const byteLength = (s: string) => utf8.encode(s).length;
/** `to_ascii_lowercase`: keeps the length, unlike `toLowerCase` on some non-ASCII text. */
const asciiLower = (s: string) => s.replace(/[A-Z]/g, (c) => c.toLowerCase());
/** Rust's `char::is_whitespace || char::is_control`. */
const WHITESPACE_OR_CONTROL = /[\s\p{Cc}\u0085]/u;

/** Parse and validate a repository URL typed by the user. */
export function parseRepoUrl(input: string): RepoParse {
  try {
    return { ok: true, repo: parse(input) };
  } catch (err) {
    if (err instanceof RepoError) return { ok: false, code: err.code, message: err.message };
    throw err;
  }
}

function parse(input: string): RepoRef {
  const text = input.trim();
  if (text === '') throw invalid('Enter a GitHub or GitLab repository URL.');
  if (byteLength(text) > MAX_URL_BYTES) throw invalid('The URL is too long.');
  if (WHITESPACE_OR_CONTROL.test(text)) throw invalid('The URL must not contain spaces or control characters.');

  // Query and fragment carry nothing we use.
  const s = text.split(/[#?]/, 1)[0]!;
  const lower = asciiLower(s);
  let rest: string;
  const sep = lower.indexOf('://');
  if (sep >= 0) {
    const scheme = lower.slice(0, sep);
    rest = s.slice(sep + 3);
    switch (scheme) {
      case 'https':
      case 'http':
        break;
      case 'ssh':
      case 'git+ssh':
        if (!rest.startsWith('git@')) throw invalid('SSH addresses must use the git@ user.');
        rest = rest.slice(4);
        break;
      case 'file':
        throw new RepoError('unsupported_host', 'Only github.com and gitlab.com are supported, not local files.');
      default:
        throw invalid(`Unsupported URL scheme "${scheme}:". Use an https:// link.`);
    }
  } else if (s.startsWith('git@')) {
    // git@host:path
    const scp = s.slice(4);
    const colon = scp.indexOf(':');
    const host = colon < 0 ? '' : scp.slice(0, colon);
    if (colon < 0 || host.includes('/')) throw invalid('Expected git@host:owner/repo.');
    return finish(host, scp.slice(colon + 1));
  } else {
    rest = s;
  }
  const slash = rest.indexOf('/');
  return slash < 0 ? finish(rest, '') : finish(rest.slice(0, slash), rest.slice(slash + 1));
}

function finish(authority: string, path: string): RepoRef {
  if (authority.includes('@')) {
    throw invalid('Remove the user name or password from the URL; only public repositories are supported.');
  }
  if (authority.includes(':')) {
    throw invalid(/:\d*$/.test(authority) ? 'Remove the port from the URL.' : 'Unsupported URL. Paste an https:// link or host/owner/repo.');
  }
  let host = asciiLower(authority);
  if (host.startsWith('www.')) host = host.slice(4);
  if (host === 'github.com') return parseGithub(path);
  if (host === 'gitlab.com') return parseGitlab(path);
  if (host.includes('.')) {
    throw new RepoError('unsupported_host', `Only github.com and gitlab.com are supported, not "${host}".`);
  }
  throw invalid('Start with github.com/ or gitlab.com/, e.g. github.com/owner/repo.');
}

/** Path segments, allowing one trailing slash. */
function segments(path: string): string[] {
  const p = path.endsWith('/') ? path.slice(0, -1) : path;
  if (p === '') return [];
  const segs = p.split('/');
  if (segs.some((s) => s === '')) throw invalid('The path contains an empty segment.');
  return segs;
}

function stripGit(seg: string): string {
  return seg.length >= 4 && asciiLower(seg.slice(-4)) === '.git' ? seg.slice(0, -4) : seg;
}

function checkSegment(seg: string, what: string): void {
  const bad =
    seg === '' ||
    seg.length > 100 ||
    !/^[A-Za-z0-9_.-]+$/.test(seg) ||
    seg.startsWith('-') ||
    (seg.startsWith('.') && seg.length <= 2) ||
    seg.includes('..');
  if (bad) throw invalid(`"${seg}" is not a valid ${what} name.`);
}

function branchFromTree(rest: string[], kind: string): string {
  if (rest.length === 0) throw invalid(`The ${kind} link is missing a branch name.`);
  const branch = rest.join('/');
  const message = validateBranch(branch);
  if (message) throw invalid(message);
  return branch.trim();
}

function parseGithub(path: string): RepoRef {
  const segs = segments(path);
  if (segs.length < 2) throw invalid('Add the repository: github.com/owner/repo.');
  const owner = segs[0]!;
  const name = stripGit(segs[1]!);
  checkSegment(owner, 'owner');
  if (owner.includes('.')) throw invalid(`"${owner}" is not a valid owner name.`);
  checkSegment(name, 'repository');
  const repo: RepoRef = { host: 'github.com', path: `${owner}/${name}` };
  const extra = segs.slice(2);
  if (extra.length > 0) {
    if (extra[0] !== 'tree') {
      throw invalid('Link to the repository itself (github.com/owner/repo) or to a /tree/<branch> page.');
    }
    repo.branch = branchFromTree(extra.slice(1), '/tree/');
  }
  return repo;
}

function parseGitlab(path: string): RepoRef {
  const segs = segments(path);
  let repoSegs = segs;
  let branch: string | undefined;
  const dash = segs.indexOf('-');
  if (dash >= 0) {
    if (segs[dash + 1] !== 'tree') throw invalid('Link to the repository itself or to a /-/tree/<branch> page.');
    branch = branchFromTree(segs.slice(dash + 2), '/-/tree/');
    repoSegs = segs.slice(0, dash);
  }
  if (repoSegs.length < 2) throw invalid('Add the repository: gitlab.com/group/repo.');
  if (repoSegs.length > MAX_GITLAB_DEPTH) throw invalid('The GitLab group path is too deep.');
  const parts = [...repoSegs];
  const last = parts.length - 1;
  parts[last] = stripGit(parts[last]!);
  for (const [i, seg] of parts.entries()) {
    const what = i === last ? 'project' : 'group';
    checkSegment(seg, what);
    if (seg.startsWith('.') || seg.endsWith('.')) throw invalid(`"${seg}" is not a valid ${what} name.`);
  }
  const repo: RepoRef = { host: 'gitlab.com', path: parts.join('/') };
  if (branch !== undefined) repo.branch = branch;
  return repo;
}

/** `host/path`, the canonical form used in the page URL (`?repo=`). */
export function repoKey(repo: Pick<RepoRef, 'host' | 'path'>): string {
  return `${repo.host}/${repo.path}`;
}

/**
 * `git check-ref-format --branch` rules restricted to a conservative
 * character set (the server's `validate_branch`). Returns a message when the
 * branch is rejected, otherwise null. Surrounding whitespace is ignored.
 */
export function validateBranch(input: string): string | null {
  const b = input.trim();
  const bad = (why: string) => `"${b}" is not a valid branch name: ${why}.`;
  if (b === '' || b.length > 200) return bad('use 1 to 200 characters');
  if (!/^[A-Za-z0-9._\-/+]+$/.test(b)) return bad('use letters, digits and . _ - / +');
  if (/^[-/.]/.test(b) || /[/.]$/.test(b) || b.endsWith('.lock')) {
    return bad('it cannot start with - / . or end with / . .lock');
  }
  if (b.includes('..') || b.includes('//') || b.includes('/.') || b.includes('.lock/')) {
    return bad('it cannot contain .. // or components starting with .');
  }
  if (b === 'HEAD') return bad('HEAD is not a branch');
  return null;
}

/** Validates an Advanced date field (`YYYY-MM-DD`, 1970 to 9999); empty means unset. */
export function validateDate(input: string): string | null {
  const d = input.trim();
  if (d === '') return null;
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(d);
  if (!m) return 'Use the format YYYY-MM-DD.';
  const [y, mo, day] = [Number(m[1]), Number(m[2]), Number(m[3])];
  if (y < 1970) return 'Dates before 1970 are not supported.';
  const leap = (y % 4 === 0 && y % 100 !== 0) || y % 400 === 0;
  const days = [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][mo - 1];
  if (days === undefined || day === 0 || day > days) return 'That date does not exist.';
  return null;
}

/** Subdirectory as the server stores it: no `./` or `/` prefix, no trailing `/`; '' = the whole tree. */
export function normaliseSubdir(input: string): string {
  let p = input.trim();
  while (p.startsWith('./')) p = p.slice(2);
  p = p.replace(/^\/+/, '').replace(/\/+$/, '');
  return p === '.' ? '' : p;
}

/** Validates the Advanced subdirectory field (the server's `validate_path`); empty is allowed. */
export function validateSubdir(input: string): string | null {
  const p = normaliseSubdir(input);
  if (p === '') return null;
  if (byteLength(p) > 1024) return 'The subdirectory path is too long.';
  if (/[\p{Cc}\\]/u.test(p)) return 'Control characters and backslashes are not allowed.';
  for (const seg of p.split('/')) {
    if (seg === '') return 'The subdirectory path contains an empty segment.';
    if (seg === '.' || seg === '..') return 'The subdirectory must not contain "." or "..".';
  }
  return null;
}
