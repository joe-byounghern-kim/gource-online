# Gource player

The standalone player (GPL-3.0-or-later), deployed as its own site (viewer.gource.online) and
bundled by `npx gource-online`. It plays one Gource log, which it gets in one of three ways:

- **By URL:** `#src=<log URL>&title=<text>&o.<option>=<value>`. The player fetches the log itself,
  with no credentials, no referrer and no redirects.
- **By postMessage:** from the page that opened it, or the page that embeds it in an iframe.
- **From a local file:** opened or dropped on the page. The file is read on this computer.

The first two follow the MIT handoff protocol in `packages/protocol/src/player.ts`, documented in
[docs/features/player-handoff.md](../../docs/features/player-handoff.md). The player has no sign-in,
no account and no paid features. It reads no cookies and sends nothing anywhere.

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
| `GOURCE_SOURCE_REF`, `GOURCE_REPO_URL` | The exact source the page links: a tag or commit (default: `git describe`) and the repository. |

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

Both the start screen and the settings panel link the licence, the exact source of the build and the third-party notices.

A page that shows these screens may plug in analytics and a run-ending hook (`src/host.ts`). The
player plugs in neither.
