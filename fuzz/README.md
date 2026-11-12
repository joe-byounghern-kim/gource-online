# Parser fuzzing

These local libFuzzer targets exercise the production parsers. The streaming
targets also require byte-identical successful custom-log output and statistics
across input chunk boundaries. The custom-log target additionally compares error
kind and message for deliberate rich-export rejections, rather than panicking on
an expected rejection. Its shared property and minimized unsupported-coverage
seed are exercised natively by `cargo test -p gource-sources --test custom_log_fuzz`.
The pull request target checks event order and log-safe paths.

```sh
cargo install --locked cargo-fuzz
rustup toolchain install nightly --profile minimal
mkdir -p fuzz/corpus && cp -R fuzz/seeds/. fuzz/corpus/
cargo +nightly fuzz run git_raw -- -max_total_time=3600 -max_len=65536
cargo +nightly fuzz run custom_log -- -max_total_time=3600 -max_len=65536
cargo +nightly fuzz run pulls -- -max_total_time=3600 -max_len=65536
```

Run all three for one hour each before a release, and keep the final stats
and toolchain versions with the release notes. Any crash blocks release:
reproduce the saved input, add a normal regression test, fix the parser, and rerun.
Seed `fuzz/corpus/custom_log` with `tests/fixtures/custom-log/` and the other targets
with valid git raw and tab-separated pull request records. Corpora, crash artifacts,
and instrumented builds are ignored, not shipped. `fuzz/Cargo.lock` is committed so
the fuzz runtime is reproducible.
