// SPDX-License-Identifier: MIT

// Gource's git log command (formats/git.cpp in Gource), as a Gource Online
// clone service runs it and as a person runs it in their own checkout. The
// attribution line (`m1:`) carries raw identities and trailers, split by
// 0x1f and ended by 0x1e, then the subject; `gource-rs convert --from git`
// and the browser's parser read it and keep only agent and model names.

/** Gource's own format. */
export const PRETTY = '--pretty=format:user:%aN%n%ct';

/** Gource's format with the attribution line. */
export const PRETTY_AI =
  '--pretty=format:user:%aN%n%ct%nm1:%an%x1f%ae%x1f%cn%x1f%ce%x1f%(trailers:key=Co-authored-by,key=Made-with,unfold,separator=%x1f)%x1e%s';

/** `git log` arguments before the revision range: an argument list, never a shell string. */
export const LOG_ARGS: readonly string[] = [
  '-c',
  'core.quotepath=off',
  'log',
  '--reverse',
  '--raw',
  '--encoding=UTF-8',
  '--no-renames',
  '--no-show-signature',
  PRETTY_AI,
];

/** The same command for a person to paste into a shell; `--output` avoids shell redirection (PowerShell re-encodes it). */
export function shellLogCommand(output: string): string {
  const format = PRETTY_AI.slice('--pretty=format:'.length);
  return (
    'git -c core.quotepath=off log --reverse --raw --encoding=UTF-8 --no-renames --no-show-signature ' +
    `--pretty=format:"${format}" --output=${output}`
  );
}
