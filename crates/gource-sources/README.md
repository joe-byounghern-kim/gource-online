# gource-sources

Readers that turn bytes into the event stream of `gource-core`, and the writer for the custom log. It never
starts a process: the caller runs `git` and passes the output in.

## What it ports

| File | C++ source | Notes |
|---|---|---|
| `custom.rs` | `formats/custom.cpp` | The custom log, `timestamp\|username\|A/M/D\|/path[\|RRGGBB]`, the interchange format |
| `git.rs` | `formats/git.cpp` | The output of Gource's `git log --raw` command |
| `utf8filter.rs` | `formats/commitlog.cpp`: `RCommitLog::filter_utf8` | Invalid UTF-8 becomes `?`, as utf8cpp does |
| `datetime.rs` | `core/settings.cpp`: `SDLAppSettings::parseDateTime`; `core/timezone.cpp` | Date strings in the timestamp column |
| `writer.rs` | `gource.cpp`: `Gource::writeCustomLog` | `gource --output-custom-log` |

The line splitting follows `core/seeklog.cpp`, and the patterns follow `core/regex.cpp` as the C++ uses them.

## What is new

| File | What |
|---|---|
| `meta.rs` | An optional sixth field of the custom log, `key=value` pairs. C++ Gource ignores it |
| `classify.rs`, `attrs.rs` | Which AI agents a commit is credited to, from the author identity and the `Co-authored-by` and `Made-with` trailers |
| `pulls.rs`, `rich_pulls.rs` | Pull request events in the commit stream |
| `batch.rs` | Binary batches that carry parsed commits from a Web Worker to the main thread |
| `bounded.rs` | A writer with a size limit, for histories that are not trusted |
| `source.rs` | A narrow grammar for an opt-in source producer |
| `sink.rs` | Appends parsed commits to an event log |

## Tests against the C++

`tests/cpp-reference/` builds a harness from Gource's own parser sources; `tests/cpp_reference.rs` and
`tests/cpp_fuzz.rs` compare its output with the Rust parsers when the harness is built. `fuzz/` at the repository
root holds the libFuzzer targets for these parsers.

    cargo test -p gource-sources

GPL-3.0-or-later, derived from Gource. See `LICENSING.md`.
