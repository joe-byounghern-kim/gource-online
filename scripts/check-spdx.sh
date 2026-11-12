#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Every tracked source file of the port carries an SPDX licence line in its
# first five lines. The upstream C++ tree (src/, and the core submodule) is
# Gource's and keeps its own headers.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
missing=0
while IFS= read -r f; do
  if ! head -n 5 "$f" | grep -q 'SPDX-License-Identifier:'; then
    echo "no SPDX line: $f"
    missing=1
  fi
done < <(git ls-files 'crates/*.rs' 'apps/*.ts' 'apps/*.js' 'apps/*.mjs' 'packages/*.ts' 'scripts/*.sh' 'scripts/*.mjs' \
          ':!packages/viewer/src/wasm/*' ':!**/*.d.ts')
exit "$missing"
