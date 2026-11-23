# gource-online

Watch your git repository's history grow, commit by commit, in your browser:
[Gource](https://gource.io) compiled to WebAssembly, running on your own computer.

```
cd your-repository
npx -y gource-online@<version>
```

Pin the version you mean to run, such as `npx -y gource-online@0.2.0-beta.3`, or use the
`beta` dist-tag (`npx -y gource-online@beta`); the player's start screen shows the command
with the version it belongs to.

It reads the history with `git log`, starts a small web server on `127.0.0.1` and opens
the Gource player in your browser: playback, options, recording to MP4 and the AI
statistics, all on your computer. Works with any repository, private or public, on any git
host or none.

Before it reads anything, it says what it reads and where it goes; then it prints the link
(on a line of its own) and keeps serving until you press Ctrl+C:

```
gource-online 0.2.0-beta.3

Your history stays on this computer.
  - Reads names, emails, dates, subjects and file paths, never file contents.
  - Shows them in your browser from 127.0.0.1. Nothing is uploaded.
  - Inspect the code: https://github.com/joe-byounghern-kim/gource-online

Reading the history of /home/you/your-repository
  456 commits, 204.0 KB, 2024-03-01 to 2026-11-17

Gource is running at:

  http://127.0.0.1:60766/#src=%2F5538af5c08b44aae2f1d65d366ae5277%2Fyour-repository.gitlog&title=your-repository

Opening it in your browser. Press Ctrl+C to stop.

Public repositories and sign-in features: https://gource.online
```

In a terminal a few headings are bold, the last line is dim and the counts update while git
runs; the link itself is never styled. Piped or redirected, with `NO_COLOR` set,
`FORCE_COLOR=0` or `TERM=dumb`, the output is plain text (a `FORCE_COLOR` set by a CI
service or a test runner does not add codes to piped output). All of it is ASCII.

Errors go to standard error, one line saying what happened and the next what to do:

```
gource-online: /home/you is not inside a git repository.
Run it from your project's folder: cd into the repository, then npx gource-online.
```

Exit status: 0 when stopped with Ctrl+C or after `--output`, 1 when it cannot run (no git, no
repository, no commits, the port is taken), 2 for an unknown option or argument.

## What it reads, and where it goes

- **What:** for each commit, the author's and committer's names and emails, the time, the subject, AI
  co-author trailers (`Co-authored-by`, `Made-with`) and the paths it added, changed or
  deleted. No file contents. It is Gource's own git command:

  ```
  git -c core.quotepath=off log --reverse --raw --encoding=UTF-8 --no-renames --no-show-signature --no-color \
    --pretty=format:"user:%aN%n%ct%nm1:%an%x1f%ae%x1f%cn%x1f%ce%x1f%(trailers:key=Co-authored-by,key=Made-with,unfold,separator=%x1f)%x1e%s"
  ```

- **Where:** nowhere. The history stays in memory (or in the file you name with `--output`)
  and is served only to your browser:
  - The server listens on `127.0.0.1`, not on your network.
  - The history sits at a random path that changes on every run.
  - Requests addressed to any other host name are refused.
  - The page may connect to this server only (`connect-src 'self'`), and no other page may
    embed it (`frame-ancestors 'none'`).
  - The player opens the history by its own link (`#src=…`), a fragment no server ever sees.
  - The command sends nothing anywhere, and it has no dependencies. (`npx` itself downloads
    the package from the npm registry before it runs.)
  - The terminal names the hosted site, gource.online, as plain text. Nothing contacts it.

Emails and trailers are only used to recognise AI agents in the browser; they are not shown.

## Options

```
--port <n>         Port on 127.0.0.1 (default: any free port)
--no-open          Print the link instead of opening the browser
--output <file>    Save the history to <file> and exit, to open it in the
                   Gource player later
-h, --help         Show this help
-v, --version      Show the version (only the version, for scripts)
```

On a machine reached over SSH, forward the port it prints
(`ssh -L <port>:127.0.0.1:<port> <host>`) and open the link on your computer, or make a
file with `--output history.gitlog` and open that.

Needs Node.js 18.18 or later and git.

## What comes from Gource

The command itself is new. It runs the git command that Gource runs (`formats/git.cpp`), with a few
fields added for AI co-authors, and the page it opens is Gource ported to WebAssembly: the simulation,
the layout and the look are Gource's (`crates/README.md` in the source maps the port to the C++).

## Licence and source

GPL-3.0-or-later, like Gource; the licence is in `COPYING`. This package contains the
compiled page (`dist/`). Its source, and the command's, is at
https://github.com/joe-byounghern-kim/gource-online (`apps/cli/` for the command, `apps/player/`, `packages/viewer/` and
`crates/` for the page).
