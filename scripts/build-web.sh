#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Build the gource-web WebAssembly module and its JavaScript bindings into
# packages/viewer/src/wasm/ (imported by packages/viewer/src/engine/load.ts).
#
#   scripts/build-web.sh [--release|--dev]     (default: --release)
#
# Needs the wasm32-unknown-unknown target (rust-toolchain.toml) and the
# wasm-bindgen CLI matching the wasm-bindgen crate in Cargo.lock.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
profile=release
case "${1:---release}" in
  --release) profile=release ;;
  --dev) profile=dev ;;
  -h|--help) sed -n '3,10p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
  *) echo "usage: $0 [--release|--dev]" >&2; exit 2 ;;
esac

export PATH="$HOME/.cargo/bin:$PATH"
cd "$root"

want="$(awk '/^name = "wasm-bindgen"$/ { getline; gsub(/version = |"/, ""); print; exit }' Cargo.lock)"
if ! command -v wasm-bindgen >/dev/null; then
  echo "wasm-bindgen CLI not found; install it with: cargo install wasm-bindgen-cli --version $want" >&2
  exit 1
fi
have="$(wasm-bindgen --version | awk '{ print $2 }')"
if [[ -n "$want" && "$have" != "$want" ]]; then
  echo "wasm-bindgen CLI $have does not match the crate version $want in Cargo.lock" >&2
  echo "install it with: cargo install wasm-bindgen-cli --version $want" >&2
  exit 1
fi

cargo_args=(build -p gource-web --target wasm32-unknown-unknown)
dir=debug
bindgen_args=(--debug --keep-debug)
if [[ "$profile" == release ]]; then
  cargo_args+=(--release)
  dir=release
  bindgen_args=()
fi

target_dir="$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
target_dir="${target_dir:-$root/target}"

echo "==> cargo ${cargo_args[*]}"
cargo "${cargo_args[@]}"

out="$root/packages/viewer/src/wasm"
rm -rf "$out"
mkdir -p "$out"
echo "==> wasm-bindgen --target web --out-dir packages/viewer/src/wasm"
wasm-bindgen --target web --out-dir "$out" ${bindgen_args[@]+"${bindgen_args[@]}"} \
  "$target_dir/wasm32-unknown-unknown/$dir/gource_web.wasm"

wasm="$out/gource_web_bg.wasm"
bytes="$(wc -c < "$wasm" | tr -d ' ')"
gz="$(gzip -9c "$wasm" | wc -c)"
awk -v b="$bytes" -v g="$gz" 'BEGIN { printf "==> packages/viewer/src/wasm/gource_web_bg.wasm: %d bytes (%.1f KiB, %.1f KiB gzipped)\n", b, b / 1024, g / 1024 }'

# The crates compiled into the wasm engine travel with it (served as a page
# file and copied into the npx package by build-cli.sh).
notices="$root/apps/player/assets/THIRD_PARTY_LICENSES.txt"
node "$root/scripts/third-party-licenses.mjs" --package gource-web --target wasm32-unknown-unknown > "$notices"
echo "==> apps/player/assets/THIRD_PARTY_LICENSES.txt"
