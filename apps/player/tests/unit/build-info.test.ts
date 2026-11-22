// SPDX-License-Identifier: GPL-3.0-or-later

import { expect, test } from '@playwright/test';

import { buildSourceRef, CLI_VERSION, hostedSiteUrl, npxCommand, PUBLIC_REPO_URL, REPO_URL, SOURCE_REF, sourceUrl } from '../../src/build-info';

const PUBLIC = 'https://github.com/joe-byounghern-kim/gource-online';

test('without a build, the public repository\'s front page', () => {
  expect(PUBLIC_REPO_URL).toBe(PUBLIC);
  expect(SOURCE_REF).toBe('');
  expect(REPO_URL).toBe(PUBLIC);
  expect(sourceUrl()).toBe(PUBLIC);
});

test('a release links the tree of its tag in the public repository', () => {
  expect(sourceUrl('v0.2.0-beta.2', PUBLIC)).toBe(`${PUBLIC}/tree/v0.2.0-beta.2`);
  expect(sourceUrl('v1.0.0', PUBLIC)).toBe(`${PUBLIC}/tree/v1.0.0`);
});

test('the public repository never gets a branch or a commit, which may exist only in a private repository', () => {
  for (const ref of ['web-mvp', 'main', 'agent/ux-player-start', '8943d96', 'v0.2.0-beta.2-3-g8943d96x/y', 'release-1']) {
    expect(sourceUrl(ref, PUBLIC), ref).toBe(PUBLIC);
  }
});

test('another repository (a self-hosted fork) links the ref it names, encoded', () => {
  expect(sourceUrl('v0.2.0-beta.1', 'https://github.com/o/r')).toBe('https://github.com/o/r/tree/v0.2.0-beta.1');
  expect(sourceUrl('feat/x y', 'https://github.com/o/r')).toBe('https://github.com/o/r/tree/feat/x%20y');
  expect(sourceUrl('', 'https://github.com/o/r')).toBe('https://github.com/o/r');
});

test('an empty repository URL means no source link yet', () => {
  expect(sourceUrl('v1.0.0', '')).toBeNull();
});

test('the build\'s ref: the one given, else the commit\'s release tag, else none', () => {
  const fail = () => {
    throw new Error('no tag');
  };
  expect(buildSourceRef('v0.2.0-beta.2', fail)).toBe('v0.2.0-beta.2');
  expect(buildSourceRef(undefined, () => 'v0.2.0-beta.2\n')).toBe('v0.2.0-beta.2');
  // release.yml passes an empty ref on a dry run: the commit decides, and an untagged commit links no ref.
  expect(buildSourceRef('', fail)).toBe('');
  expect(buildSourceRef(undefined, () => 'archive/rust-server\n')).toBe('');
  expect(buildSourceRef(undefined, () => '')).toBe('');
});

test('the hosted site: the gource.online origin the build lets hand over logs, else gource.online', () => {
  expect(hostedSiteUrl(['https://gource.online'])).toBe('https://gource.online');
  expect(hostedSiteUrl(['https://staging.gource.online'])).toBe('https://staging.gource.online');
  expect(hostedSiteUrl(['https://pages.example.org', 'https://staging.gource.online'])).toBe('https://staging.gource.online');
  // The CLI's copy has no senders; a self-hosted build's own pages are not the hosted site.
  expect(hostedSiteUrl([])).toBe('https://gource.online');
  expect(hostedSiteUrl(['https://pages.example.org'])).toBe('https://gource.online');
  expect(hostedSiteUrl(['https://gource.online.example.org', 'http://127.0.0.1:5173', 'http://staging.gource.online'])).toBe('https://gource.online');
});

test('the npx command is pinned to the build\'s version, else the beta dist-tag', () => {
  expect(CLI_VERSION).toBe('');
  expect(npxCommand('0.2.0-beta.2')).toBe('npx -y gource-online@0.2.0-beta.2');
  expect(npxCommand()).toBe('npx -y gource-online@beta');
});
