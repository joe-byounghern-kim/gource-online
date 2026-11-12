# gource-cli

The native binary `gource-rs`. It converts logs and runs the simulation without a window; it draws nothing.

| Command | What | Related C++ |
|---|---|---|
| `gource-rs convert` | Git raw log or custom log in, custom log out | `gource --output-custom-log` (`Gource::writeCustomLog`), reading what `formats/git.cpp` reads |
| `gource-rs simulate` | A headless run that prints a hash of the scene per report | Used to compare runs with each other and with the C++ |
| `gource-rs bench` | Timings of each phase of the frame pipeline | None |

`convert` reads the output of Gource's own git command, which this repository pins in
`packages/protocol/src/git-log.ts`:

    git log --reverse --raw --no-renames --pretty=format:... | gource-rs convert --from git

The parsers and the writer are in `gource-sources`, the simulation in `gource-core`. Releases ship static Linux
builds of `gource-rs` with the notices of the crates they contain.

    cargo run -p gource-cli -- --help
    cargo test -p gource-cli

GPL-3.0-or-later, derived from Gource. See `LICENSING.md`.
