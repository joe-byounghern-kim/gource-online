# Licensing

| Part | Licence |
|---|---|
| Upstream Gource (C++: `src/`, `data/`, build files), except `data/decorations.png` | GPL-3.0-or-later, © Andrew Caudwell and contributors |
| `data/decorations.png` | GPL-3.0-or-later, made for this project |
| The Rust port (`crates/`), the player (`apps/player`), the component (`packages/viewer`), the CLI (`apps/cli`) | GPL-3.0-or-later. A derivative work of Gource |
| `packages/protocol` (and `packages/protocol/vectors/`) | MIT |

This is a modified version of Gource: it adds the Rust port, the player, the component and the CLI above, changed from
upstream in 2026. Upstream's C++ stays here, unchanged from upstream commit `70db547`, as the reference for its
behaviour. `crates/README.md` maps each ported file to the C++ it follows.

The full GPL text is in `COPYING`; the MIT text in `packages/protocol/LICENSE`. Every source
file names its licence in an SPDX line.

**Third-party code in the builds.** The wasm engine and the `gource-rs` binaries contain Rust
crates under permissive licences (MIT, Apache-2.0, Unicode-3.0 and similar, all compatible
with the GPL). Their notices ship with each build: `THIRD_PARTY_LICENSES.txt` in the player (and
in the npm package), and `gource-rs-<target>.THIRD_PARTY_LICENSES.txt` beside each
binary in a release. `scripts/third-party-licenses.mjs` generates them from `Cargo.lock`. The
player's font is GNU FreeFont (`apps/player/assets/fonts/README`).

**The hosted service.** Its clone service is a separate program, not in this
repository. It runs `git` and the `gource-rs` binary from these releases as separate
processes, and links no code from here except the MIT `@gource-online/protocol`. Its site runs
the player as a separate site, on its own origin, and talks to it only through the MIT handoff
protocol (`packages/protocol/src/player.ts`). The player it serves is a release of `apps/player`,
built from the public tag it links.

**Contributions** are accepted under the licence of the part they change (inbound = outbound),
with a Developer Certificate of Origin sign-off; see `CONTRIBUTING.md`.
