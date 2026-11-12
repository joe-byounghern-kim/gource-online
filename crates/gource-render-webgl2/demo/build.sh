#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Build the WebGL2 backend demo into ./pkg (release by default, --debug for
# GL error checks inside the backend).
set -euo pipefail
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:$PATH"
profile=release
flag=--release
if [[ "${1:-}" == "--debug" ]]; then profile=debug; flag=; fi
cargo build $flag --target wasm32-unknown-unknown
wasm-bindgen --target web --no-typescript --out-dir pkg \
    "target/wasm32-unknown-unknown/$profile/gource_render_webgl2_demo.wasm"
