// SPDX-License-Identifier: MIT

import { readFileSync } from 'node:fs';

import { expect, test } from '@playwright/test';

import { normaliseSubdir, parseRepoUrl, validateBranch, validateDate, validateSubdir } from '../src/repo-url.js';

// The shared vectors: every clone service and client must agree on each one.
interface Vector {
  input: string;
  error?: string;
  host?: string;
  path?: string | null;
  branch?: string;
  date?: string;
}
const vectors = JSON.parse(readFileSync(new URL('../vectors/repo-urls.json', import.meta.url), 'utf8')) as Record<
  'urls' | 'branches' | 'dates' | 'paths',
  Vector[]
>;

test('shared URL vectors match the clone service', () => {
  for (const v of vectors.urls) {
    const r = parseRepoUrl(v.input);
    if (v.error) {
      expect(r.ok, JSON.stringify(v.input)).toBe(false);
      if (!r.ok) expect(r.code, JSON.stringify(v.input)).toBe(v.error);
    } else {
      const repo: Record<string, string> = { host: v.host!, path: v.path! };
      if (v.branch !== undefined) repo.branch = v.branch;
      expect(r, JSON.stringify(v.input)).toEqual({ ok: true, repo });
    }
  }
});

test('shared branch, date and path vectors match the clone service', () => {
  for (const v of vectors.branches) {
    expect(validateBranch(v.input) === null, JSON.stringify(v.input)).toBe(!v.error);
  }
  for (const v of vectors.dates) {
    expect(validateDate(v.input) === null, JSON.stringify(v.input)).toBe(!v.error);
  }
  for (const v of vectors.paths) {
    expect(validateSubdir(v.input) === null, JSON.stringify(v.input)).toBe(!v.error);
    if (!v.error) expect(normaliseSubdir(v.input), JSON.stringify(v.input)).toBe(v.path ?? '');
  }
});

test('length limits match the clone service', () => {
  expect(parseRepoUrl(`github.com/o/${'r'.repeat(3000)}`)).toMatchObject({ ok: false, code: 'invalid_url' });
  expect(validateBranch('b'.repeat(201))).not.toBeNull();
  expect(validateSubdir('a'.repeat(1025))).not.toBeNull();
  expect(validateSubdir('a'.repeat(1024))).toBeNull();
});

const accepts: Array<[string, { host: string; path: string; branch?: string }]> = [
  ['github.com/acaudwell/Gource', { host: 'github.com', path: 'acaudwell/Gource' }],
  ['https://github.com/acaudwell/Gource', { host: 'github.com', path: 'acaudwell/Gource' }],
  ['HTTPS://GitHub.com/acaudwell/Gource/', { host: 'github.com', path: 'acaudwell/Gource' }],
  ['https://www.github.com/acaudwell/Gource.git', { host: 'github.com', path: 'acaudwell/Gource' }],
  ['  github.com/acaudwell/Gource  ', { host: 'github.com', path: 'acaudwell/Gource' }],
  ['git@github.com:acaudwell/Gource.git', { host: 'github.com', path: 'acaudwell/Gource' }],
  ['git@gitlab.com:gitlab-org/gitlab-test.git', { host: 'gitlab.com', path: 'gitlab-org/gitlab-test' }],
  ['https://github.com/acaudwell/Gource/tree/master', { host: 'github.com', path: 'acaudwell/Gource', branch: 'master' }],
  ['github.com/owner/repo/tree/feature/x-1', { host: 'github.com', path: 'owner/repo', branch: 'feature/x-1' }],
  ['https://github.com/acaudwell/Gource?tab=readme#top', { host: 'github.com', path: 'acaudwell/Gource' }],
  ['https://github.com/acaudwell/Gource#readme', { host: 'github.com', path: 'acaudwell/Gource' }],
  ['http://github.com/a/b', { host: 'github.com', path: 'a/b' }],
  ['ssh://git@github.com/a/b.git', { host: 'github.com', path: 'a/b' }],
  ['github.com/owner/.github', { host: 'github.com', path: 'owner/.github' }],
  ['github.com/my-org/my_repo.js', { host: 'github.com', path: 'my-org/my_repo.js' }],
  ['https://gitlab.com/gitlab-org/gitlab-test', { host: 'gitlab.com', path: 'gitlab-org/gitlab-test' }],
  ['www.gitlab.com/group/sub/deeper/repo.git', { host: 'gitlab.com', path: 'group/sub/deeper/repo' }],
  ['https://gitlab.com/group/sub/repo/-/tree/main', { host: 'gitlab.com', path: 'group/sub/repo', branch: 'main' }],
  ['gitlab.com/group/repo/-/tree/release/1.0?ref_type=heads', { host: 'gitlab.com', path: 'group/repo', branch: 'release/1.0' }],
];

for (const [input, expected] of accepts) {
  test(`accepts ${JSON.stringify(input)}`, () => {
    const r = parseRepoUrl(input);
    expect(r).toEqual({ ok: true, repo: expected });
  });
}

const rejects: Array<[string, RegExp]> = [
  ['', /Enter a GitHub or GitLab/],
  ['   ', /Enter a GitHub or GitLab/],
  ['ftp://github.com/a/b', /Unsupported URL scheme/],
  ['ssh://root@github.com/a/b.git', /git@ user/],
  ['file:///etc/passwd', /Only github\.com and gitlab\.com/],
  ['https://bitbucket.org/a/b', /Only github\.com and gitlab\.com/],
  ['https://github.com.evil.com/a/b', /Only github\.com and gitlab\.com/],
  ['https://evilgithub.com/a/b', /Only github\.com and gitlab\.com/],
  ['https://user:pass@github.com/a/b', /user name or password/],
  ['https://token@github.com/a/b', /user name or password/],
  ['https://github.com:443/a/b', /port/],
  ['github.com:8080/a/b', /port/],
  ['git@evil.com:a/b.git', /Only github\.com/],
  ['github.com', /Add the repository/],
  ['github.com/acaudwell', /Add the repository/],
  ['github.com/a/b/issues', /repository itself/],
  ['github.com/a/b/blob/main/README.md', /repository itself/],
  ['github.com/a/../b', /"\.\."|\.\./],
  ['github.com/a/b..c', /\.\./],
  ['github.com/a//b', /empty segment/],
  ['github.com/a/b%2F..', /not a valid repository/],
  ['github.com/a b/c', /spaces/],
  ['github.com/-a/b', /not a valid owner/],
  ['github.com/a/b;rm', /not a valid repository/],
  ['github.com/a/$(id)', /not a valid repository/],
  ['github.com/a/ünïcode', /not a valid repository/],
  ['github.com/a/b/tree/', /missing a branch|repository itself|Add the/],
  ['github.com/a/b/tree/-x', /not a valid branch/],
  ['github.com/a/b/tree/HEAD', /not a valid branch/],
  ['github.com/a/b/tree/a..b', /\.\./],
  ['gitlab.com/onlygroup', /Add the repository/],
  ['gitlab.com/g/r/-/blob/main/x', /repository itself/],
  ['gitlab.com/g/r/-/tree', /missing a branch/],
  ['gitlab.com/g/-r', /not a valid project/],
  ['javascript:alert(1)', /Unsupported URL/],
  ['github.com:a/b', /Unsupported URL/],
];

for (const [input, message] of rejects) {
  test(`rejects ${JSON.stringify(input)}`, () => {
    const r = parseRepoUrl(input);
    expect(r.ok).toBe(false);
    if (!r.ok) expect(r.message).toMatch(message);
    if (!r.ok) expect(['invalid_url', 'unsupported_host']).toContain(r.code);
  });
}

test('branch validation follows the git-branch rule', () => {
  expect(validateBranch('main')).toBeNull();
  expect(validateBranch('release/1.2')).toBeNull();
  expect(validateBranch('feat_x+y')).toBeNull();
  expect(validateBranch('feat=z')).not.toBeNull();
  expect(validateBranch('-main')).not.toBeNull();
  expect(validateBranch('a..b')).not.toBeNull();
  expect(validateBranch('a b')).not.toBeNull();
  expect(validateBranch('a/')).not.toBeNull();
  expect(validateBranch('x.lock')).not.toBeNull();
  expect(validateBranch('$(id)')).not.toBeNull();
});

test('date validation', () => {
  expect(validateDate('')).toBeNull();
  expect(validateDate('2020-02-29')).toBeNull();
  expect(validateDate('2021-02-29')).toMatch(/does not exist/);
  expect(validateDate('2020-13-01')).toMatch(/does not exist/);
  expect(validateDate('2020-1-1')).toMatch(/YYYY-MM-DD/);
  expect(validateDate('yesterday')).toMatch(/YYYY-MM-DD/);
  expect(validateDate('1960-01-01')).toMatch(/1970/);
});

test('subdirectory validation', () => {
  expect(validateSubdir('')).toBeNull();
  expect(validateSubdir('src')).toBeNull();
  expect(validateSubdir('/src/core/')).toBeNull();
  expect(validateSubdir('docs/user guide')).toBeNull();
  expect(validateSubdir('../etc')).not.toBeNull();
  expect(validateSubdir('src/./x')).not.toBeNull();
  expect(validateSubdir('src//x')).not.toBeNull();
  // Passed to git after `--` with literal pathspecs, so these are safe.
  expect(validateSubdir('-rf')).toBeNull();
  expect(validateSubdir('a;b')).toBeNull();
  expect(validateSubdir('a\\b')).not.toBeNull();
  expect(normaliseSubdir(' /src/core/ ')).toBe('src/core');
});
