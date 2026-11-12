# gource-online

Watch your git repository's history grow, commit by commit, in your browser:
[Gource](https://gource.io) compiled to WebAssembly, running on your own computer.

```
cd your-repository
npx -y gource-online
```

It reads the history with `git log`, starts a small web server on `127.0.0.1` and opens
the Gource player in your browser: playback, options, recording to MP4 and the AI
statistics, all on your computer. Press Ctrl+C to stop it. Works with any repository,
private or public, on any git host or none.

## What it reads, and where it goes

- **What:** for each commit, the author's name and email, the time, the subject, AI
  co-author trailers (`Co-authored-by`, `Made-with`) and the paths it added, changed or
  deleted. No file contents. It is Gource's own git command:

  ```
  git -c core.quotepath=off log --reverse --raw --encoding=UTF-8 --no-renames --no-show-signature --no-color \
    --pretty=format:"user:%aN%n%ct%nm1:%an%x1f%ae%x1f%cn%x1f%ce%x1f%(trailers:key=Co-authored-by,key=Made-with,unfold,separator=%x1f)%x1e%s"
  ```

- **Where:** nowhere. The history stays in memory and is served only to your browser:
  - The server listens on `127.0.0.1`, not on your network.
  - The history sits at a random path that changes on every run.
  - Requests addressed to any other host name are refused.
  - The page may connect to this server only (`connect-src 'self'`), and no other page may
    embed it (`frame-ancestors 'none'`).
  - The player opens the history by its own link (`#src=…`), a fragment no server ever sees.
  - The command sends nothing anywhere, and it has no dependencies.
  - The terminal names the hosted site, gource.online, as plain text. Nothing contacts it.

Emails and trailers are only used to recognise AI agents in the browser; they are not shown.

## Options

```
--port <n>         Port on 127.0.0.1 (default: any free port)
--no-open          Print the link instead of opening the browser
--output <file>    Write the history to <file> and exit, to open it in Gource later
-h, --help         Show this help
-v, --version      Show the version
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
