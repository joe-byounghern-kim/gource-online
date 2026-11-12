#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The player's build (release wasm, apps/player/dist), served statically with
# its own headers on http://127.0.0.1:8787 (vite preview). For the real-engine
# e2e tests, make a test build, with the sample logs and the test hooks:
#   GOURCE_PLAYER_TEST_BUILD=1 VITE_E2E_HOOKS=1 scripts/serve.sh
#   E2E_REAL=1 E2E_BASE_URL=http://127.0.0.1:8787 pnpm -C apps/player exec playwright test \
#     tests/viewer-real.spec.ts tests/ai-real.spec.ts tests/pr-real.spec.ts tests/record-real.spec.ts
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$PATH"
cd "$root"

"$root/scripts/build-web.sh" --release
[[ -d node_modules ]] || pnpm install --frozen-lockfile
pnpm run build:packages
pnpm -C apps/player build
echo "==> serving apps/player/dist on http://127.0.0.1:8787"
cd apps/player
exec node_modules/.bin/vite preview --host 127.0.0.1 --port 8787 --strictPort
