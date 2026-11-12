# Player handoff protocol

`packages/protocol/src/player.ts` (MIT) defines how a page gives the player a log to play. The hosted
site, third-party pages and the CLI talk to the player only through this protocol. Issue #123.

## Public log by URL

The page opens the player at `playerUrl(player, { src, title?, options }, sources)`:

```
https://viewer.example/#src=<log URL>&title=<text>&o.<option>=<value>
```

- **Fragment:** everything is in the fragment, which browsers never send to a server. `playerUrl`
  refuses a handoff whose fragment would be longer than `PLAYER_MAX_FRAGMENT` (16384 characters,
  after percent-encoding), so the player can read every URL it builds.
- **`src`:**
  - It is an absolute URL, or a path on the player's own origin.
  - Its origin must be the player's own or one of `sources`.
  - It must not carry credentials or a fragment, and must not be the player page itself.
- **Fetching `src`:** the player fetches `src` itself, with `PLAYER_LOG_FETCH`: `credentials: 'omit'`,
  no referrer and `redirect: 'error'`, so an open redirect on an allowed source cannot lead elsewhere.
  - The log server answers with CORS for the player's origin and **never** sends
    `Access-Control-Allow-Credentials`.
  - The player and the hosted site can be on the same site, so SameSite cookies do not protect a
    log. The URL path must only ever reach what anyone could download.
  - Owner-only logs never go by URL.
  - The player's CSP `connect-src` must include the `src` origins.
- **Options and title:**
  - `o.<option>` values are Gource options: at most 64, names `[a-z][a-z0-9-]*`, values up to
    1024 characters. The player still validates each one with the engine.
  - The title is at most 200 characters, with no control or bidirectional-override characters.
    The player shows it as text only.
  - Unknown or repeated keys are refused.

## Log bytes by postMessage

Such a log can be private, and the protocol does not say which. So while one plays, the player keeps changed settings
in memory (style rules, AI options, recording defaults and presets), and they end with it; earlier stored settings are
still read. Its name stays out of the tab title. The player clears `window.opener` when the handoff ends, however it ends.

This path is for logs that must not appear in a URL, browser history or server logs, such as
owner-only results. The sender opens the player in a new window, or embeds it in an iframe, and
talks to that window only.

1. Player → sender: `ready`, posted to `window.opener`, or else to `window.parent` when that is not
   the player itself. It carries nothing, so its `targetOrigin` may be each allowed sender origin
   in turn.
2. Sender → player:
   - `begin` with `{ id, bytes, chunk_bytes, chunks, title?, options }`.
   - `chunks` × `chunk` with `{ id, seq, data }`. `data` is an `ArrayBuffer`, transferred.
   - `end` with `{ id }`.
3. Player → sender: `ack` with `{ id }`, or `error` with `{ id?, code }`. A sender that gives up
   partway sends `error` with its handoff's `id`, which ends that handoff at once.
   The player sends `error` with `cancelled` when it stops waiting because the person opened another
   log (a local file, or another `#src=` link), so the sender need not wait out its timeout.

### Messages and limits

- Every message carries `protocol: 'gource-player'` and `version: 1`. The player ignores messages
  of other protocols. It refuses another version with `version` and a malformed message with
  `invalid`.
- The limits are fixed by the protocol, so every sender meets every player's:
  - A log is at most `PLAYER_MAX_LOG_BYTES` (256 MiB, the hosted log cap).
  - `chunk_bytes` is between `PLAYER_MIN_CHUNK_BYTES` (64 KiB) and `PLAYER_MAX_CHUNK_BYTES` (4 MiB).
  - Every chunk but the last is exactly `chunk_bytes`, and `chunks` is `ceil(bytes / chunk_bytes)`.
  - The whole handoff must arrive within `PLAYER_HANDOFF_TIMEOUT_MS` (60 s) of `ready`.

### The receiver

`PlayerHandoffReceiver` takes one log, in order:
- A chunk out of sequence, a different `id`, or an early `end` gives `order`.
- A chunk of the wrong size gives `size`.
- The first error ends the handoff, and is reported once. Completion ends it too. Everything after
  that is `ignored`, so a stray message never looks like a failure of a log that is already playing.
- `ready`, `ack`, an `error` for another handoff, and other protocols are `ignored` and change
  nothing.

### The sender

`playerHandoffMessages` builds the sender's side. Each chunk's buffer is a fresh copy of exactly its
bytes, never the input's whole underlying buffer: a view such as Node's pooled `Buffer` shares that
buffer with other data. A `SharedArrayBuffer` input is refused.

## Origins

Both sides check the other's exact origin before they look at a message:
- The player:
  - accepts messages only when `event.source` is the window it posted `ready` to, and `event.origin`
    is in its configured sender list;
  - keeps senders and log sources as two separate lists (`parsePlayerOrigins`, `playerOriginAllowed`).
- The sender:
  - accepts the player's messages only when `event.source` is the player window it opened or
    embedded, and `event.origin` is the player's origin;
  - posts with that exact origin as `targetOrigin`, never `'*'`.
- `parsePlayerOrigins` takes exact HTTPS origins. It allows http loopback origins only with
  `{ loopback: true }`, for development; production configuration must not pass it.
- A sandboxed page has origin `"null"`, which is never allowed.

## Headers this needs

The base headers send `Cross-Origin-Opener-Policy: same-origin`, which cuts the opener link both
ways. Under it, the new-window mode cannot work: the player's `window.opener` is null and `ready`
never arrives.
- The sender page uses `Cross-Origin-Opener-Policy: same-origin-allow-popups`. It opens the player
  without `noopener`, and keeps the returned window.
- The player document uses `Cross-Origin-Opener-Policy: unsafe-none`, or sends none.
- For the iframe mode, the player's CSP lists the senders in `frame-ancestors` instead of the base
  `'none'`. The sender's CSP allows the player in `frame-src`.

## The player

`apps/player` implements both paths (`src/handoff.ts`). Its settings are fixed when it is built:
- `GOURCE_PLAYER_SENDERS`: the pages that may hand it a log, and embed it (`frame-ancestors`);
- `GOURCE_PLAYER_SOURCES`: the origins it may fetch a `#src=` log from (`connect-src`);
- `GOURCE_PLAYER_TEST_BUILD=1`: also allows http loopback origins and the placeholder engine, for tests only.
  A release build allows neither.

It sends the headers above (`src/headers.ts`, emitted as `_headers`). `apps/player/tests/player.spec.ts`
runs the built player on real origins, with no request interception:
- a sender page in an iframe, and in a new window under `same-origin-allow-popups`;
- a frame of the sender page, which is ignored, and the opener's window after it moved to another
  allowed origin, which is ignored too (the first sender fixes the origin);
- a log server, including a redirect to an allowed log, which `redirect: 'error'` refuses;
- a hostile origin that may neither embed the player nor hand it a log.

## Not covered here

- The hosted site's CORS for public logs, and its sender.
