#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build the gource-online npm package (apps/cli/): the player (apps/player)
# with the release wasm engine goes into apps/cli/dist (without source maps or
# the static host's _headers, which the CLI's server sends itself), the licence
# into apps/cli/COPYING.
#
#   scripts/build-cli.sh [--pack]    --pack also writes apps/cli/gource-online-<version>.tgz
#
# Try the package without publishing it:
#   cd some/repository && npx -y --package=/path/to/apps/cli/gource-online-<version>.tgz gource-online
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
pack=0
case "${1:-}" in
  "") ;;
  --pack) pack=1 ;;
  -h|--help) sed -n '3,11p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
  *) echo "usage: $0 [--pack]" >&2; exit 2 ;;
esac
cd "$root"

"$root/scripts/build-web.sh" --release
[[ -d node_modules ]] || pnpm install --frozen-lockfile
# A release build: no senders, no log sources, no test hooks (apps/player/README.md).
env -u GOURCE_PLAYER_SENDERS -u GOURCE_PLAYER_SOURCES -u GOURCE_PLAYER_TEST_BUILD -u VITE_E2E_HOOKS \
  pnpm -C apps/player build

echo "==> apps/cli/dist"
rm -rf apps/cli/dist
mkdir -p apps/cli/dist
cp -R apps/player/dist/. apps/cli/dist/
find apps/cli/dist -name '*.map' -delete
rm -f apps/cli/dist/_headers
cp COPYING apps/cli/COPYING
du -sh apps/cli/dist | awk '{ print "    " $1 " unpacked" }'

if [[ "$pack" == 1 ]]; then
  echo "==> npm pack"
  (cd apps/cli && rm -f gource-online-*.tgz && npm pack --silent)
fi
