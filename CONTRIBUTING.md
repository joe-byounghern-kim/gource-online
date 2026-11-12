# Contributing

Thank you for helping. Issues and pull requests are welcome; for larger changes, open an issue
or a Discussion first.

## Sign your commits (DCO)

Every commit needs a `Signed-off-by: Your Name <you@example.com>` line, which certifies the
[Developer Certificate of Origin](https://developercertificate.org/): you wrote the change or
have the right to submit it under this project's licences. `git commit -s` adds it. There is
no CLA.

## Build and test

    export PATH="$HOME/.cargo/bin:$PATH"     # rust-toolchain.toml pins Rust and the wasm target
    pnpm install
    cargo test --workspace
    scripts/build-web.sh --dev               # wasm engine → packages/viewer/src/wasm
    pnpm run test && pnpm run typecheck
    pnpm -C apps/player test:e2e             # Playwright, placeholder engine
    scripts/dev.sh                           # http://127.0.0.1:5174/?mock=1

The C++ reference implementation builds as before (`./autogen.sh && ./configure && make`).
Behaviour of the port must match it; see `docs/web-mvp/PARITY.md`.

## Rules of the codebase

- Port Gource's behaviour faithfully before changing its look.
- A file ported from the C++ names its source in its module comment, and its crate's README lists it.
- No third-party rendering engines: rendering is hand-written WebGL2.
- The simulation never touches the GPU; the renderer only consumes frame batches.
- New source files carry an SPDX line (`GPL-3.0-or-later`; `MIT` under `packages/protocol`).
