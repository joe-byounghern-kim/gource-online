// SPDX-License-Identifier: GPL-3.0-or-later

//! Help text, version, and how the binary reports a bad command line.

mod common;

use common::run;

const COMMANDS: [&str; 3] = ["convert", "simulate", "bench"];

#[test]
fn top_level_help_lists_every_command() {
    for flag in ["--help", "-h"] {
        let out = run([flag]).ok();
        assert_eq!(out.stderr, "");
        assert!(out.stdout.contains("USAGE:"), "{}", out.stdout);
        for command in COMMANDS {
            assert!(
                out.stdout
                    .lines()
                    .any(|l| l.trim_start().starts_with(command)),
                "{command} is not listed in\n{}",
                out.stdout
            );
        }
    }
}

#[test]
fn every_command_has_help() {
    for command in COMMANDS {
        for flag in ["--help", "-h"] {
            let out = run([command, flag]).ok();
            assert_eq!(out.stderr, "", "{command} {flag}");
            assert!(
                out.stdout.starts_with(&format!("gource-rs {command}: ")),
                "{command}: {}",
                out.stdout
            );
            assert!(out.stdout.contains("USAGE:"), "{command}");
            assert!(
                out.stdout.contains(&format!("gource-rs {command} ")),
                "{command} usage line"
            );
            assert!(out.stdout.contains("--help"), "{command}");
        }
    }
}

#[test]
fn help_documents_every_option() {
    let simulate = run(["simulate", "--help"]).ok().stdout;
    for option in [
        "--frames", "--dt", "--seed", "--width", "--height", "--every",
    ] {
        assert!(simulate.contains(option), "simulate help lacks {option}");
    }
    let bench = run(["bench", "--help"]).ok().stdout;
    for option in ["--frames", "--dt", "--json"] {
        assert!(bench.contains(option), "bench help lacks {option}");
    }
    let convert = run(["convert", "--help"]).ok().stdout;
    for needle in ["--from", "git", "custom", "<in>", "<out>"] {
        assert!(convert.contains(needle), "convert help lacks {needle}");
    }
}

#[test]
fn help_wins_over_later_problems() {
    // Not a usage error: the user asked for help.
    run(["simulate", "--frames", "abc", "--help"]).ok();
    run(["convert", "--help", "--from", "nonsense"]).ok();
    run(["bench", "a", "b", "c", "-h"]).ok();
}

#[test]
fn version() {
    for flag in ["--version", "-V"] {
        let out = run([flag]).ok();
        assert_eq!(
            out.stdout,
            format!(
                "gource-rs {} (port of Gource 0.57)\n",
                env!("CARGO_PKG_VERSION")
            )
        );
    }
}

#[test]
fn no_command_is_a_usage_error() {
    let out = run(Vec::<&str>::new()).fails(2, "missing command");
    assert!(
        out.stderr.ends_with("Try 'gource-rs --help' for usage.\n"),
        "{}",
        out.stderr
    );
}

#[test]
fn unknown_command_is_a_usage_error() {
    run(["frobnicate"]).fails(2, "unknown command 'frobnicate'");
    run(["--frobnicate"]).fails(2, "unknown command '--frobnicate'");
    // Options go after the command.
    run(["--frames", "5", "simulate"]).fails(2, "unknown command '--frames'");
    // No abbreviations, no case folding.
    run(["sim"]).fails(2, "unknown command 'sim'");
    run(["Bench"]).fails(2, "unknown command 'Bench'");
}

#[test]
fn errors_are_prefixed_and_point_at_the_right_help() {
    for command in COMMANDS {
        let out = run([command, "--no-such-option"]);
        assert_eq!(out.code, Some(2), "{command}");
        assert!(
            out.stderr
                .starts_with("gource-rs: error: unknown option '--no-such-option'\n"),
            "{}",
            out.stderr
        );
        assert!(
            out.stderr
                .ends_with(&format!("Try 'gource-rs {command} --help' for usage.\n")),
            "{}",
            out.stderr
        );
        assert_eq!(out.stdout, "");
    }
}

#[cfg(unix)]
#[test]
fn paths_need_not_be_unicode() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let dir = common::scratch("paths_need_not_be_unicode");
    let mut name = b"odd-".to_vec();
    name.extend_from_slice(&[0xff, 0xfe]);
    name.extend_from_slice(b".log");
    let path = dir.join(OsString::from_vec(name));
    // APFS refuses names that are not UTF-8 (EILSEQ). On it, assert the
    // actual CLI failure instead of silently omitting file-input coverage.
    // streams' opener-seam test covers successful raw-byte input on every
    // Unix platform; byte-preserving filesystems also run this full conversion.
    match std::fs::write(&path, "1|a|A|/x\n") {
        Ok(()) => {
            let out = run([
                "convert".as_ref(),
                "--from".as_ref(),
                "custom".as_ref(),
                path.as_os_str(),
                "-".as_ref(),
            ])
            .ok();
            assert_eq!(out.stdout, "1|a|A|/x\n");
        }
        Err(e) if cfg!(target_os = "macos") && e.raw_os_error() == Some(92) => {
            let out = run([
                "convert".as_ref(),
                "--from".as_ref(),
                "custom".as_ref(),
                path.as_os_str(),
                "-".as_ref(),
            ])
            .fails(1, "cannot open");
            assert_eq!(out.stdout, "");
            let open_error = std::fs::File::open(&path).unwrap_err();
            assert_eq!(
                out.stderr,
                format!(
                    "gource-rs: error: cannot open '{}': {open_error}\n",
                    path.display()
                )
            );
        }
        Err(e) => panic!("cannot write {}: {e}", path.display()),
    }

    // A non-Unicode word where an option should be is reported, not a panic.
    let bad = OsString::from_vec(vec![b'-', b'-', 0xff]);
    let out = run([OsString::from("simulate"), bad]);
    assert_eq!(out.code, Some(2));
    assert!(out.stderr.contains("unknown option"), "{}", out.stderr);
}

#[test]
fn version_as_json_names_the_attribution_registry() {
    let out = run(["--version", "--json"]).ok();
    assert_eq!(
        out.stdout,
        format!(
            "{{\"version\":\"{}\",\"reference_gource\":\"0.57\",\"registry\":1}}\n",
            env!("CARGO_PKG_VERSION")
        )
    );
}
