// SPDX-License-Identifier: GPL-3.0-or-later

//! `gource-rs simulate`, run as the built binary.
//!
//! These tests hold for any core behind the contract: they check the report
//! format, the schedule of reports and determinism, never the values the
//! simulation produces.

mod common;

use common::{Json, field, fields, fixture, fixtures, read, run, run_with_stdin, scratch, write};

const LOG: &str = "visual-hierarchy-activity.log";

fn simulate(args: &[&str]) -> String {
    let log = fixture(LOG);
    let mut all = vec!["simulate", log.to_str().unwrap()];
    all.extend_from_slice(args);
    run(all).ok().stdout
}

fn frames_of(output: &str) -> Vec<u32> {
    output
        .lines()
        .map(|line| field(line, "frame").parse().unwrap())
        .collect()
}

#[test]
fn two_runs_print_identical_output() {
    for log in fixtures() {
        let args = [
            "simulate".as_ref(),
            log.as_os_str(),
            "--frames".as_ref(),
            "240".as_ref(),
            "--every".as_ref(),
            "20".as_ref(),
            "--seed".as_ref(),
            "9".as_ref(),
        ];
        let first = run(args).ok();
        let second = run(args).ok();
        assert!(!first.stdout.is_empty(), "{log:?} printed nothing");
        assert_eq!(first.stdout, second.stdout, "{log:?} is not deterministic");
        assert_eq!(first.stderr, "", "{log:?}");
        assert_eq!(first.stdout.lines().count(), 12, "{log:?}");
    }
}

#[test]
fn default_run_reports_once_a_second_for_ten_seconds() {
    let out = simulate(&[]);
    assert_eq!(
        frames_of(&out),
        (1..=10).map(|n| n * 60).collect::<Vec<_>>()
    );
}

#[test]
fn report_lines_have_the_documented_fields() {
    let out = simulate(&["--frames", "3", "--every", "1"]);
    assert_eq!(out.lines().count(), 3);
    for line in out.lines() {
        let keys: Vec<&str> = fields(line).into_iter().map(|(k, _)| k).collect();
        assert_eq!(
            keys,
            ["frame", "time", "files", "users", "dirs", "hash"],
            "{line}"
        );
        assert!(field(line, "time").parse::<i64>().is_ok(), "{line}");
        for count in ["files", "users", "dirs"] {
            assert!(field(line, count).parse::<usize>().is_ok(), "{line}");
        }
        let hash = field(line, "hash");
        assert_eq!(hash.len(), 16, "{line}");
        assert!(
            hash.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "{line}"
        );
    }
}

#[test]
fn reports_come_every_k_frames_and_after_the_last() {
    assert_eq!(
        frames_of(&simulate(&["--frames", "10", "--every", "4"])),
        [4, 8, 10]
    );
    // No duplicate when the last frame is a multiple of K.
    assert_eq!(
        frames_of(&simulate(&["--frames", "8", "--every", "4"])),
        [4, 8]
    );
    assert_eq!(
        frames_of(&simulate(&["--frames", "3", "--every", "1"])),
        [1, 2, 3]
    );
    // K larger than the run: only the final report.
    assert_eq!(
        frames_of(&simulate(&["--frames", "5", "--every", "100"])),
        [5]
    );
    assert_eq!(frames_of(&simulate(&["--frames", "1"])), [1]);
}

#[test]
fn reporting_does_not_change_the_run() {
    // Every frame is simulated and extracted whether or not it is reported,
    // so a frame has the same line however often reports are printed.
    let dense = simulate(&["--frames", "40", "--every", "1"]);
    let dense: Vec<&str> = dense.lines().collect();
    assert_eq!(dense.len(), 40);

    let sparse = simulate(&["--frames", "40", "--every", "10"]);
    for line in sparse.lines() {
        let frame: usize = field(line, "frame").parse().unwrap();
        assert_eq!(line, dense[frame - 1]);
    }
}

#[test]
fn viewport_is_part_of_the_state() {
    let hash_of = |args: &[&str]| {
        let out = simulate(args);
        field(out.lines().last().unwrap(), "hash").to_string()
    };
    let base = hash_of(&["--frames", "5"]);
    assert_eq!(
        base,
        hash_of(&["--frames", "5", "--width", "1280", "--height", "720"])
    );
    assert_ne!(base, hash_of(&["--frames", "5", "--width", "800"]));
    assert_ne!(base, hash_of(&["--frames", "5", "--height", "700"]));
}

#[test]
fn frame_time_spellings() {
    let hash_of = |args: &[&str]| {
        let out = simulate(args);
        field(out.lines().last().unwrap(), "hash").to_string()
    };
    // The default is 1/60 and every spelling of the same f32 agrees.
    assert_eq!(
        simulate(&["--frames", "30"]),
        simulate(&["--frames", "30", "--dt", "1/60"])
    );
    assert_eq!(
        hash_of(&["--frames", "5", "--dt", "0.5"]),
        hash_of(&["--frames", "5", "--dt", "1/2"])
    );
    assert_eq!(
        hash_of(&["--frames", "5", "--dt=0.25"]),
        hash_of(&["--frames", "5", "--dt", ".25"])
    );
}

/// Two reports of one run, or `None` while the core is the contract stub:
/// its clock never moves, so there is no state for an option to change.
fn last_report(args: &[&str]) -> Option<String> {
    let out = simulate(args);
    let line = out.lines().last().unwrap();
    (field(line, "time") != "0").then(|| line.to_string())
}

#[test]
fn dt_and_seed_reach_the_simulation() {
    let run = |extra: &[&str]| {
        let mut args = vec!["--frames", "300", "--every", "300"];
        args.extend_from_slice(extra);
        last_report(&args)
    };
    let Some(base) = run(&[]) else {
        eprintln!("the core does not simulate yet: nothing for --dt and --seed to change");
        return;
    };
    // Gource caps one simulation step at 1/60 s, so only a smaller step
    // can differ from the default.
    assert_ne!(base, run(&["--dt", "1/120"]).unwrap(), "--dt is ignored");
    assert_ne!(base, run(&["--seed", "1"]).unwrap(), "--seed is ignored");
    assert_eq!(
        base,
        run(&["--seed", "31"]).unwrap(),
        "31 is the default seed"
    );
}

#[test]
fn options_may_come_before_the_log() {
    let log = fixture(LOG);
    let before = run([
        "simulate",
        "--frames",
        "6",
        "--every",
        "3",
        log.to_str().unwrap(),
    ])
    .ok();
    assert_eq!(before.stdout, simulate(&["--frames", "6", "--every", "3"]));
}

#[test]
fn bench_ends_on_the_state_simulate_reports_for_that_frame() {
    let log = fixture(LOG);
    let log = log.to_str().unwrap();
    for options in [
        &[][..],
        &["--seed", "11"],
        &["--width", "500", "--height", "300"],
        &["--dt", "1/30"],
    ] {
        let mut bench_args = vec!["bench", log, "--json", "--frames", "37"];
        bench_args.extend_from_slice(options);
        let bench = Json::parse(&run(bench_args).ok().stdout).unwrap();

        let mut sim_args = vec!["simulate", log, "--frames", "37", "--every", "37"];
        sim_args.extend_from_slice(options);
        let sim = run(sim_args).ok().stdout;
        let line = sim.lines().last().unwrap();

        assert_eq!(bench.str("final.hash"), field(line, "hash"), "{options:?}");
        assert_eq!(
            bench.num("final.time"),
            field(line, "time").parse::<f64>().unwrap()
        );
        assert_eq!(
            bench.num("entities.files"),
            field(line, "files").parse::<f64>().unwrap()
        );
        assert_eq!(
            bench.num("entities.users"),
            field(line, "users").parse::<f64>().unwrap()
        );
        assert_eq!(
            bench.num("entities.dirs"),
            field(line, "dirs").parse::<f64>().unwrap()
        );
    }
}

#[test]
fn standard_input_gives_the_same_result_as_the_file() {
    let log = fixture(LOG);
    let args = ["--frames", "12", "--every", "6", "--seed", "3"];
    let from_file = simulate(&args);
    let mut stdin_args = vec!["simulate", "-"];
    stdin_args.extend_from_slice(&args);
    let from_stdin = run_with_stdin(stdin_args, read(&log).as_bytes()).ok();
    assert_eq!(from_stdin.stdout, from_file);
}

#[test]
fn an_empty_or_malformed_log_still_runs() {
    let dir = scratch("an_empty_or_malformed_log_still_runs");
    for (name, contents) in [
        ("empty.log", ""),
        ("garbage.log", "this is not a log\nneither is this\n"),
        ("one-line-no-newline.log", "1|a|A|/x"),
    ] {
        let path = write(&dir, name, contents);
        let out = run([
            "simulate".as_ref(),
            path.as_os_str(),
            "--frames".as_ref(),
            "4".as_ref(),
            "--every".as_ref(),
            "2".as_ref(),
        ])
        .ok();
        assert_eq!(frames_of(&out.stdout), [2, 4], "{name}");
    }
}

#[test]
fn a_closed_pipe_ends_the_run_quietly() {
    use std::process::{Command, Stdio};

    // Far more frames than could ever finish; the reader quits after one line.
    let mut child = Command::new(env!("CARGO_BIN_EXE_gource-rs"))
        .args(["simulate", fixture(LOG).to_str().unwrap()])
        .args(["--frames", "4000000000", "--every", "1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let mut first = [0u8; 16];
    std::io::Read::read_exact(&mut stdout, &mut first).unwrap();
    drop(stdout);
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8_lossy(&output.stderr), "");
}

#[test]
fn bad_arguments_exit_with_status_2() {
    let log = fixture(LOG);
    let log = log.to_str().unwrap();
    let cases: &[(&[&str], &str)] = &[
        (&["simulate"], "missing argument <custom.log>"),
        (&["simulate", log, "extra"], "unexpected argument 'extra'"),
        (&["simulate", log, "--bogus"], "unknown option '--bogus'"),
        (
            &["simulate", log, "--frames"],
            "option '--frames' requires a value",
        ),
        (
            &["simulate", log, "--frames", "abc"],
            "invalid value 'abc' for --frames",
        ),
        (
            &["simulate", log, "--frames", "0"],
            "invalid value '0' for --frames",
        ),
        (
            &["simulate", log, "--frames", "-5"],
            "invalid value '-5' for --frames",
        ),
        (
            &["simulate", log, "--frames", "1.5"],
            "invalid value '1.5' for --frames",
        ),
        (
            &["simulate", log, "--every", "0"],
            "invalid value '0' for --every",
        ),
        (
            &["simulate", log, "--dt", "0"],
            "invalid value '0' for --dt",
        ),
        (
            &["simulate", log, "--dt", "-1"],
            "invalid value '-1' for --dt",
        ),
        (
            &["simulate", log, "--dt", "1/0"],
            "invalid value '1/0' for --dt",
        ),
        (
            &["simulate", log, "--dt", "fast"],
            "invalid value 'fast' for --dt",
        ),
        (
            &["simulate", log, "--dt", "7200"],
            "invalid value '7200' for --dt",
        ),
        (
            &["simulate", log, "--seed", "-1"],
            "invalid value '-1' for --seed",
        ),
        (
            &["simulate", log, "--seed", "4294967296"],
            "invalid value '4294967296' for --seed",
        ),
        (
            &["simulate", log, "--width", "0"],
            "invalid value '0' for --width",
        ),
        (
            &["simulate", log, "--height", "100000"],
            "invalid value '100000' for --height",
        ),
        (
            &["simulate", log, "--frames", "5", "--frames", "6"],
            "option '--frames' given more than once",
        ),
        (&["simulate", log, "--json"], "unknown option '--json'"),
    ];
    for (args, needle) in cases {
        run(*args).fails(2, needle);
    }
}

#[test]
fn missing_log_exits_with_status_1() {
    let dir = scratch("missing_log_exits_with_status_1");
    let missing = dir.join("nope.log");
    run(["simulate".as_ref(), missing.as_os_str()]).fails(1, "cannot open");
}

#[test]
fn usage_errors_point_at_the_help() {
    let out = run(["simulate", "--bogus"]);
    assert!(
        out.stderr
            .ends_with("Try 'gource-rs simulate --help' for usage.\n"),
        "{}",
        out.stderr
    );
}
