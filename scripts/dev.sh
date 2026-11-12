#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Development setup: build the wasm engine, then run the player's Vite dev
# server on http://127.0.0.1:5174. Open a log file or drop one on the page, or
# open /#src=/samples/<file>.log; ?mock=1 uses the placeholder engine.
#
#   scripts/dev.sh [--dev|--release]   wasm profile (default: --release)
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"
cd "$root"

"$root/scripts/build-web.sh" "${1:---release}"
[[ -d node_modules ]] || pnpm install --frozen-lockfile
pnpm run build:packages

echo "==> try http://127.0.0.1:5174/#src=/samples/file-removal.log"
cd apps/player
# Vite directly (not through `pnpm exec`, which does not pass signals on).
exec node_modules/.bin/vite --host 127.0.0.1 --port 5174 --strictPort
