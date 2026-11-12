# Self-hosting the player

The player needs no server code: local files and logs by link play from static hosting.

1. Download `player-dist.tar.gz` from a release, or build your own (below). A release is built for
   viewer.gource.online: only `https://gource.online` may hand it a log or be a log source.
2. Unpack it to your host's web root.
3. Serve `_headers` (Cloudflare Pages, Netlify) or set the same headers in your server.
   `.wasm` must be served as `application/wasm`.

To let your own pages hand over logs, or fetch logs from other origins, fix those origins at build time:

```sh
scripts/build-web.sh --release
GOURCE_PLAYER_SENDERS='https://pages.example.org' \
GOURCE_PLAYER_SOURCES='https://logs.example.org https://cdn.example.org:8443' \
GOURCE_REPO_URL='https://example.org/your/fork' \
  pnpm -C apps/player build
```

Every entry must be an exact HTTPS origin; anything else fails the build. The log host must also permit
CORS from the player. Redirects are refused. `apps/player/README.md` has the settings. If you change the
player, name the repository and the source you serve: the page links them (GPL-3.0-or-later).
