# Gource player

The standalone player (GPL-3.0-or-later), deployed as its own site (viewer.gource.online) and
bundled by `npx gource-online`. It plays one Gource log, which it gets in one of three ways:

- **By URL:** `#src=<log URL>&title=<text>&o.<option>=<value>`. The player fetches the log itself,
  with no credentials, no referrer and no redirects.
- **By postMessage:** from the page that opened it, or the page that embeds it in an iframe.
- **From a local file:** opened or dropped on the page. The file is read on this computer.

The first two follow the MIT handoff protocol in `packages/protocol/src/player.ts`, documented in
[docs/features/player-handoff.md](../../docs/features/player-handoff.md). The player has no sign-in,
no account and no paid features. It reads no cookies and requests nothing but the log it plays. It
tells the page that opened or embeds it how playback went (counts and sizes only, never a name; the
protocol's `event` message), and nothing to any other page.

## What comes from Gource

The player's code is new: Gource is a desktop program with no page around it. What it shows is Gource.
The canvas is `packages/viewer`, which runs the engine ported in `crates/` (see `crates/README.md`).
The options panel lists Gource's own options, with their names and defaults. The label font is GNU
FreeFont, the font Gource ships (`assets/fonts/README`).

| Folder | What | Gource equivalent |
|---|---|---|
| `src/screens/`, `src/ui/` | The viewer screen, the seek bar and the options panel | The HUD and `slider.cpp`; the options are `gource_settings.cpp`'s |
| `src/record/` | Recording to MP4 in the page | `--output-ppm-stream` piped to an external encoder |
| `src/ai/` | AI statistics and contributor styles | None |
| `src/handoff.ts`, `src/links.ts` | Getting a log by link or by postMessage | None; Gource reads a path or standard input |

## Build settings

| Variable | Meaning |
|---|---|
| `GOURCE_PLAYER_SENDERS` | Space-separated exact HTTPS origins whose pages may hand over a log and embed the player. |
| `GOURCE_PLAYER_SOURCES` | Space-separated exact HTTPS origins the player may fetch a `#src=` log from, besides its own. |
| `GOURCE_PLAYER_TEST_BUILD` | `1` makes a test build: http loopback origins, the placeholder engine (`?mock=1`) and the sample logs (`/samples/`). A release build has none of them; the dev server always has them. |
| `VITE_E2E_HOOKS` | `1` adds the test hooks: `window.__gource`, `?fixed-dt=<seconds>`, `?mock-record=1`. Never in a release build. |
| `GOURCE_SOURCE_REF`, `GOURCE_REPO_URL` | The source the page links: a tag or commit (default: the commit's `v*` release tag, else none) and the repository (default: the public `joe-byounghern-kim/gource-online`). With no ref the page links the repository's front page; in the public repository only a release tag is linked (`src/build-info.ts`). |

With none set, the player plays only local files and logs on its own origin, and cannot be embedded.

```sh
scripts/build-web.sh            # the engine (packages/viewer/src/wasm)
pnpm -C apps/player dev         # http://localhost:5174/?mock=1 uses the placeholder engine
pnpm -C apps/player build       # dist/, with _headers for the static host
pnpm -C apps/player test        # unit tests
pnpm -C apps/player test:e2e    # the placeholder engine on the dev server, the built player on real origins, the real engine
GOURCE_PLAYER_TEST_BUILD=1 VITE_E2E_HOOKS=1 scripts/serve.sh
E2E_REAL=1 E2E_BASE_URL=http://127.0.0.1:8787 pnpm -C apps/player exec playwright test tests/viewer-real.spec.ts tests/ai-real.spec.ts tests/pr-real.spec.ts tests/record-real.spec.ts
```

## Screens

The player plays on one viewer screen (`src/screens/viewer.ts`): playback and seeking, the options panel, recording and
MP4 export, and the AI statistics. A log by URL keeps its link equal to the view (`o.<option>` in the
fragment). Opening a local file or another link while a page is handing over a log cancels that
handoff, and the page is told (`cancelled`).

With no log to play, the start screen (`src/start.ts`) offers a local file first, then how to make one: Gource's
git command (`shellLogCommand` in `packages/protocol`) or `npx -y gource-online@<version>`, the version taken from
`apps/cli/package.json` at build time. It points to the hosted site (the `gource.online` origin among
`GOURCE_PLAYER_SENDERS`, else `https://gource.online`; left out when embedded), says what Gource is, and links the
public repository. It stays hidden while a log by link loads or a page may still hand one over.

Both the start screen and the settings panel link the licence, the source of the build and the third-party notices.

The screens report recordings and take a run-ending hook through `src/host.ts`. The player plugs in
its events for the page that opened or embeds it (`src/events.ts`) and no run-ending hook.
