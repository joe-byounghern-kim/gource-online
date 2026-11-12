// SPDX-License-Identifier: GPL-3.0-or-later

//! Native runner for the Gource port. Used for log conversion, headless
//! simulation runs, benchmarks, and (later) frame dumps and parity checks
//! against the C++ reference build.
//!
//! ```text
//! gource-rs convert  --from git|custom <in> <out|->
//! gource-rs simulate <custom.log> [--frames N] [--dt S] [--seed N] [--width W --height H] [--every K]
//! gource-rs bench    <custom.log> [--frames N] [--dt S] [--json]
//! ```
//!
//! Exit status: 0 on success, 2 for a bad command line, 3 when a `convert`
//! limit stops the conversion, 1 when a command fails while running (an
//! unreadable file, a write error).
//!
//! The runner uses only the public contracts of `gource-core`,
//! `gource-sources` and `gource-render`, so it works unchanged as the core
//! simulation fills in.

#![forbid(unsafe_code)]

mod args;
mod bench;
mod convert;
mod error;
mod json;
mod load;
mod run;
mod simulate;
mod stats;
mod streams;

#[cfg(test)]
mod ai_tests;

// The frame fingerprint moved to `gource-core` so the browser recorder can
// hash its frames too (`recording.md` REC-82); `crate::hash` keeps its path.
use gource_core::view_hash as hash;

use std::ffi::OsString;
use std::io::Write;
use std::process::ExitCode;

use error::CliError;

const HELP: &str = "\
gource-rs: native runner for the Gource Rust port

USAGE:
    gource-rs <COMMAND> [ARGS]

COMMANDS:
    convert     convert a git raw log or a custom log to a custom log
    simulate    run the simulation headless and print a state hash
    bench       time the simulation and frame-building phases

OPTIONS:
    -h, --help       print this help (every command has its own)
    -V, --version    print the version
        --version --json   the version, the Gource version ported and the attribution registry, as JSON

Run 'gource-rs <COMMAND> --help' for the arguments of a command.
";

fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    match dispatch(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => fail(&err),
    }
}

fn dispatch(mut args: Vec<OsString>) -> Result<(), CliError> {
    if args.is_empty() {
        return Err(CliError::usage_top("missing command"));
    }
    let command = args.remove(0);
    match command.to_str() {
        Some("convert") => convert::run(args),
        Some("simulate") => simulate::run(args),
        Some("bench") => bench::run(args),
        Some("-h" | "--help") => streams::print_stdout(HELP),
        Some("-V" | "--version") => {
            if args.first().is_some_and(|a| a == "--json") {
                streams::print_stdout(&format!(
                    "{{\"version\":\"{}\",\"reference_gource\":\"{}\",\"registry\":{}}}\n",
                    env!("CARGO_PKG_VERSION"),
                    gource_core::REFERENCE_GOURCE_VERSION,
                    gource_core::agents::REGISTRY_VERSION
                ))
            } else {
                streams::print_stdout(&format!(
                    "gource-rs {} (port of Gource {})\n",
                    env!("CARGO_PKG_VERSION"),
                    gource_core::REFERENCE_GOURCE_VERSION
                ))
            }
        }
        _ => Err(CliError::usage_top(format!(
            "unknown command '{}'",
            command.to_string_lossy()
        ))),
    }
}

/// Report `err` on standard error and pick the exit status.
fn fail(err: &CliError) -> ExitCode {
    if err.is_broken_pipe() {
        return ExitCode::SUCCESS;
    }
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "gource-rs: error: {err}");
    if let CliError::Usage { command, .. } = err {
        let _ = match command {
            Some(command) => writeln!(stderr, "Try 'gource-rs {command} --help' for usage."),
            None => writeln!(stderr, "Try 'gource-rs --help' for usage."),
        };
    }
    ExitCode::from(err.exit_code())
}
