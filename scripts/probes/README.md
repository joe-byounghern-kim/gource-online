<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# PR activity M0 probes

These are **local research tools for issue #4 M0**, not a paid acquisition backend or production lifecycle implementation. The approved design remains in `docs/features/pull-request-activity.md`. Nothing here activates on a normal engine build. Do not apply the scratch hook patch to the product tree, commit local histories/screenshots, or use this as v3/M1a acceptance.

## Inputs and privacy

Run from the repository root. Use a new local scratch output directory, a trusted existing public `sharkdp/bat` bare history with `FETCH_HEAD` containing the completed 2024 default-branch ancestry and the pre-year tree, and the project's already-installed tools. No installation, upload, deployment, automatic clone, remote creation, token printing, or production generated-WASM replacement is required. Acquisition is explicitly opt-in and uses the existing authenticated `gh` integration for read-only public GraphQL. Never pass a credential as an argument or environment override.

The acquisition requests no PR titles, bodies, emails, patches, or blobs. Git author name/email identities are read in memory only for contributor counting and stable pseudonyms. Public GitHub logins remain in the local source JSON, while scene labels use aliases. Subprocess and browser failure boundaries emit allowlisted codes, not child error objects, stdout/stderr, causes, raw browser messages, or argument lists. Treat all output files, including screenshots and sidecars, as local evidence.

## Pure checks and source preparation

```sh
node --test scripts/probes/*.test.mjs
# Eleven named tests, no outer experimental flag and no skips.
# Three exact-harness VM failure tests, no browser/socket: metadata, launch,
# browser shutdown cleanup. If VM module APIs are unavailable, each named case
# runs one self-child with the VM flag, bounded to5seconds/64KiB. Failures propagate
# to the default runner with raw child stderr suppressed. Synthetic evidence only.
# Eight pure helper tests: failure privacy, anchors/ties/root/unknown, future-path
# collisions, stream order/replay age, independent style, merged closure
# companion, exact-present-file flash without mutation.

# Set these to existing trusted local inputs, not remote URLs.
export PR_SCRATCH="/absolute/local/new-pr-evidence-directory"
export PR_GIT="/absolute/local/existing-bat-bare-history"

# OPTIONAL network step, only with explicit public read authorization.
# Fixed repository/year, <=500 requests, cumulative actual cost checked after
# every response against 1000 points, >=100 remaining-point reserve.
node scripts/probes/pull-request-acquire.mjs "$PR_SCRATCH" "$PR_GIT"

# Offline step, may instead use an already-acquired local source.json.
node scripts/probes/pull-request-prepare.mjs "$PR_SCRATCH" "$PR_GIT"
```

Acquisition records actual request costs/pages and connection counts, not an estimated billing total. All candidate PRs are acquired unless a bound/failure aborts. Paginated current reviews/files do not imply complete historic lifecycle coverage. Search terminal timestamps can miss historical close/reopen cycles. Restricted timeline `totalCount` did not equal selected node counts in the measured source, so full timeline completeness is explicitly not claimed. Final/current PR file snapshots are not historically exact opening diffs. Exact landing association requires an acquired default-history merge SHA and path intersection with its first-parent diff. No speculative association or PR-ref fetch occurs.

## Isolated engine build

Record a trusted source commit, archive it locally, and preserve an **unchanged** baseline. The measured run used `81f195c4443c77e98e936b5e8b33bc209378f141`. Commands below require a fresh `PR_SCRATCH`, cached Cargo dependencies, Rust/WASM target, `wasm-bindgen` matching the lockfile, existing web test dependencies, and system Chrome. They do not install missing tools. Coordinate compilation with other performance workers.

```sh
export PR_ROOT="$PWD"
export PR_BASE_COMMIT="81f195c4443c77e98e936b5e8b33bc209378f141"
mkdir -p "$PR_SCRATCH/baseline" "$PR_SCRATCH/prototype"
git archive "$PR_BASE_COMMIT" | tar -x -C "$PR_SCRATCH/baseline"
cp -R "$PR_SCRATCH/baseline/." "$PR_SCRATCH/prototype/"

# These commands change only the disposable prototype archive.
git -C "$PR_SCRATCH/prototype" apply --check "$PR_ROOT/scripts/probes/pull-request-prototype.patch"
git -C "$PR_SCRATCH/prototype" apply "$PR_ROOT/scripts/probes/pull-request-prototype.patch"
cp scripts/probes/pull-request-prototype.rs "$PR_SCRATCH/prototype/crates/gource-web/src/probe.rs"

cargo test --offline --locked --manifest-path "$PR_SCRATCH/prototype/Cargo.toml" -p gource-web probe::tests
# Three scratch adapter tests. Filtered-out project suites are not acceptance.

cargo build --offline --locked --manifest-path "$PR_SCRATCH/baseline/Cargo.toml" -p gource-web --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir "$PR_SCRATCH/baseline-wasm" "$PR_SCRATCH/baseline/target/wasm32-unknown-unknown/release/gource_web.wasm"

cargo build --offline --locked --manifest-path "$PR_SCRATCH/prototype/Cargo.toml" -p gource-web --target wasm32-unknown-unknown --release
wasm-bindgen --target web --out-dir "$PR_SCRATCH/wasm" "$PR_SCRATCH/prototype/target/wasm32-unknown-unknown/release/gource_web.wasm"

node scripts/probes/pull-request-browser.mjs "$PR_SCRATCH"
```

The browser harness reserves **loopback port 5176**, fails if unavailable, and closes its server/browser afterward. Do not use PA worker port 5175. It blocks external browser requests and serves only allowlisted WASM, font, sanitized log and sidecar assets. It uses real WASM simulation and WebGL2 renderer passes at 1280×720 and 390×800, not a static overlay. It reports numeric assertions and errors in `browser.json`, with before/merge/uncapped/year-end local PNGs. Inspect assertions and images before claiming a visual pass. An exit-zero harness alone does not establish every criterion.

Plain no-sidecar pixel equivalence is against the explicitly archived baseline build, not an existing generated asset of unknown provenance. Keep source commit, source diffs, build commands and JS/WASM SHA-256 digests with the local evidence. The initial unknown-provenance comparison is weaker evidence and must not be relabeled HEAD proof.

## What the prototype means

- Real code records remain unchanged. Initial-tree files and pre-year open PRs use current-engine `A` context carriers, not actionless rich seeds. Terminal/review PR geometry uses `M` carriers, not duplicated code changes.
- A local sidecar folds real created/closed/reopened/merged/review timestamps using replay time. A transport `ClosedEvent` accompanying a merge does not grey-close the merged marker. Local generations prove only model order, not production delayed-beam protection.
- Style mapping is an independent immutable-state trait: dim open alpha .42, merged alpha 1, alternative open alpha .68. Merge highlight and landed-file flash are bounded to **two animation seconds**, while `Nd` age uses replay-clock days from original creation.
- Only exact associated paths already present in the extracted frame are flashed. No code file is added/touched/deleted by this visual effect. The scratch materialization helper unhides **only existing, explicitly typed PR paths** so current beam behavior cannot hide a live PR carrier. It does not materialize code targets or manufacture keepalive events.
- Labels are `P/M/X #number Nd` plus supported `+/!/C` review verdicts. Collision suppression consults real renderer directory/file/user label rectangles and viewport/HUD margins. A locally selected focus label may search bounded screen offsets and use the existing scene beam pass as a **view-only annotation connector**, not an acquired history event or production follow-PR feature. Ordinary directory labels remain visible. Filenames/usernames are hidden in the final observation to reduce density, with the initial all-label failure evidence retained.
- Label caps/suppression are an explicit density policy, not a complete list or review queue. The uncapped negative case remains available. The focused checkpoint validates selected lifecycle meaning, not broad end-user discoverability.

## Not implemented or accepted here

No production options/codec, v3 parser, full lifecycle acquisition backend, actionless seeds, positive-idle retention bypass, directory-delete protection, delayed-beam generation guard, agent/AI filtering, actor filtering/hide semantics, seek restoration, F3, stream completeness HUD, export/record acceptance, deployment, or paid flow is claimed. Invisible terminal PR carrier geometry can still affect layout. The measured context uses the existing `file-idle-time=0` default. Fixed-step CPU/GPU-submission timings are not real-time FPS or GPU-completion measurements.
