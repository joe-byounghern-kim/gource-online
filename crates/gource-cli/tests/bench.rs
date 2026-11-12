// SPDX-License-Identifier: GPL-3.0-or-later

//! `gource-rs bench`, run as the built binary.
//!
//! Timings differ from run to run, so these tests check structure and
//! invariants (counts, ordering of percentiles, agreement with `simulate`),
//! never absolute times.

mod common;

use common::{Json, fixture, read, run, run_with_stdin, scratch, write};

const LOG: &str = "visual-hierarchy-activity.log";
const PHASES: [&str; 4] = ["update", "extract_view", "build", "frame"];
const STATS: [&str; 5] = ["total_ms", "mean_ms", "p50_ms", "p95_ms", "max_ms"];

fn bench_json(args: &[&str]) -> Json {
    let log = fixture(LOG);
    let mut all = vec!["bench", log.to_str().unwrap(), "--json"];
    all.extend_from_slice(args);
    let out = run(all).ok();
    assert_eq!(out.stderr, "");
    Json::parse(&out.stdout).unwrap_or_else(|e| panic!("not JSON ({e}):\n{}", out.stdout))
}

#[test]
fn json_output_parses_and_has_the_documented_shape() {
    let doc = bench_json(&["--frames", "40"]);

    assert_eq!(
        doc.keys(""),
        [
            "log", "run", "phases", "final", "entities", "view", "output", "peak"
        ]
    );
    assert_eq!(doc.keys("phases"), PHASES);
    for phase in PHASES {
        assert_eq!(doc.keys(&format!("phases.{phase}")), STATS, "{phase}");
    }
    assert_eq!(
        doc.keys("log"),
        ["path", "bytes", "commits", "changes", "load_ms"]
    );
    assert_eq!(
        doc.keys("run"),
        ["frames", "dt", "seed", "width", "height", "wall_ms"]
    );
    assert_eq!(doc.keys("final"), ["time", "commits_processed", "hash"]);
    assert_eq!(doc.keys("entities"), ["files", "users", "dirs"]);
    assert_eq!(
        doc.keys("view"),
        ["dirs", "files", "users", "actions", "labels"]
    );
    assert_eq!(
        doc.keys("output"),
        ["quads", "bloom_quads", "text_quads", "draw_commands"]
    );
    assert_eq!(
        doc.keys("peak"),
        [
            "files",
            "users",
            "dirs",
            "view_files",
            "view_labels",
            "quads",
            "text_quads"
        ]
    );
    // Peaks bound the last frame's counts.
    for (peak, last) in [
        ("peak.files", "entities.files"),
        ("peak.users", "entities.users"),
        ("peak.dirs", "entities.dirs"),
        ("peak.quads", "output.quads"),
    ] {
        assert!(doc.num(peak) >= doc.num(last), "{peak} < {last}");
    }
}

#[test]
fn gource_options_change_the_run() {
    let base = bench_json(&["--frames", "120"]);
    let fast = bench_json(&["--frames", "120", "--options", "seconds-per-day=0.1"]);
    // Faster playback reaches later in the log within the same frames.
    assert!(fast.num("final.time") > base.num("final.time"));
    let hidden = bench_json(&["--frames", "120", "--options", "hide=files,users"]);
    assert_eq!(hidden.num("view.files"), 0.0);
    assert_eq!(hidden.num("view.users"), 0.0);
    assert!(base.num("view.files") > 0.0);
    // An empty query is no options at all.
    let same = bench_json(&["--frames", "120", "--options", ""]);
    assert_eq!(same.str("final.hash"), base.str("final.hash"));
}

#[test]
fn json_values_are_consistent() {
    let doc = bench_json(&["--frames", "40"]);
    let log = fixture(LOG);

    // The log section describes the file that was loaded.
    assert_eq!(doc.str("log.path"), log.to_str().unwrap());
    assert_eq!(
        doc.num("log.bytes"),
        std::fs::metadata(&log).unwrap().len() as f64
    );
    let lines = read(&log).lines().count() as f64;
    assert_eq!(doc.num("log.changes"), lines);
    assert!(doc.num("log.commits") >= 1.0 && doc.num("log.commits") <= lines);
    assert!(doc.num("log.load_ms") >= 0.0);

    // The defaults of the run.
    assert_eq!(doc.num("run.frames"), 40.0);
    assert_eq!(doc.num("run.width"), 1280.0);
    assert_eq!(doc.num("run.height"), 720.0);
    assert_eq!(doc.num("run.seed"), 31.0);
    assert!(
        (doc.num("run.dt") - 1.0 / 60.0).abs() < 1e-7,
        "{}",
        doc.num("run.dt")
    );
    assert!(doc.num("run.wall_ms") > 0.0);

    // Every phase: ordered statistics, and the frame is the three together.
    let mut sum_of_totals = 0.0;
    for phase in PHASES {
        let stat = |name: &str| doc.num(&format!("phases.{phase}.{name}"));
        for name in STATS {
            assert!(stat(name) >= 0.0, "{phase}.{name}");
        }
        assert!(stat("p50_ms") <= stat("p95_ms"), "{phase}");
        assert!(stat("p95_ms") <= stat("max_ms"), "{phase}");
        assert!(stat("mean_ms") <= stat("max_ms") + 1e-6, "{phase}");
        assert!(stat("total_ms") <= doc.num("run.wall_ms") + 1e-3, "{phase}");
        if phase != "frame" {
            sum_of_totals += stat("total_ms");
        }
    }
    assert!(
        (doc.num("phases.frame.total_ms") - sum_of_totals).abs() < 1e-3,
        "frame total {} vs phases {sum_of_totals}",
        doc.num("phases.frame.total_ms")
    );

    // Entity counts are non-negative integers; a quad has four vertices.
    for path in [
        "final.commits_processed",
        "entities.files",
        "entities.users",
        "entities.dirs",
        "view.dirs",
        "view.files",
        "view.users",
        "view.actions",
        "view.labels",
        "output.quads",
        "output.bloom_quads",
        "output.text_quads",
        "output.draw_commands",
    ] {
        let n = doc.num(path);
        assert!(n >= 0.0 && n.fract() == 0.0, "{path} = {n}");
    }
    assert!(doc.num("final.commits_processed") <= doc.num("log.commits"));
    let hash = doc.str("final.hash");
    assert_eq!(hash.len(), 16);
    assert!(
        hash.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    );
}

#[test]
fn run_options_are_echoed() {
    let doc = bench_json(&[
        "--frames",
        "7",
        "--dt",
        "1/30",
        "--seed",
        "5",
        "--width",
        "640",
        "--height=480",
    ]);
    assert_eq!(doc.num("run.frames"), 7.0);
    assert!(
        (doc.num("run.dt") - 1.0 / 30.0).abs() < 1e-7,
        "{}",
        doc.num("run.dt")
    );
    assert_eq!(doc.num("run.seed"), 5.0);
    assert_eq!(doc.num("run.width"), 640.0);
    assert_eq!(doc.num("run.height"), 480.0);
}

#[test]
fn json_paths_are_escaped() {
    // A file name with quotes, a backslash, a space and non-ASCII text.
    let dir = scratch("json_paths_are_escaped");
    let name = "we\"ird \\ n\u{e4}me.log";
    let path = write(&dir, name, "1|a|A|/x\n");
    let out = run([
        "bench".as_ref(),
        path.as_os_str(),
        "--json".as_ref(),
        "--frames".as_ref(),
        "3".as_ref(),
    ])
    .ok();
    let doc =
        Json::parse(&out.stdout).unwrap_or_else(|e| panic!("not JSON ({e}):\n{}", out.stdout));
    assert_eq!(doc.str("log.path"), path.to_str().unwrap());
}

#[test]
fn table_output_lists_the_phases() {
    let log = fixture(LOG);
    let out = run(["bench", log.to_str().unwrap(), "--frames", "25"]).ok();
    assert_eq!(out.stderr, "");

    let lines: Vec<&str> = out.stdout.lines().collect();
    let header = lines
        .iter()
        .find(|l| l.starts_with("phase"))
        .expect("table header");
    for column in ["total ms", "mean ms", "p50 ms", "p95 ms", "max ms"] {
        assert!(header.contains(column), "{header}");
    }
    for phase in PHASES {
        let row = lines
            .iter()
            .find(|l| l.split_whitespace().next() == Some(phase))
            .unwrap_or_else(|| panic!("no row for {phase} in\n{}", out.stdout));
        // name + five numbers
        let numbers: Vec<f64> = row
            .split_whitespace()
            .skip(1)
            .map(|n| n.parse().unwrap())
            .collect();
        assert_eq!(numbers.len(), 5, "{row}");
        assert!(numbers.iter().all(|n| *n >= 0.0), "{row}");
    }
    for prefix in ["log ", "run ", "last frame", "view ", "built ", "hash "] {
        assert!(
            lines.iter().any(|l| l.starts_with(prefix)),
            "no {prefix:?} line in\n{}",
            out.stdout
        );
    }
    assert!(out.stdout.contains("25 frames"), "{}", out.stdout);
}

#[test]
fn the_final_hash_does_not_depend_on_timing() {
    let first = bench_json(&["--frames", "20"]);
    let second = bench_json(&["--frames", "20"]);
    assert_eq!(first.str("final.hash"), second.str("final.hash"));
    assert_eq!(first.num("final.time"), second.num("final.time"));
}

#[test]
fn standard_input_is_accepted() {
    let log = fixture(LOG);
    let out = run_with_stdin(
        ["bench", "-", "--json", "--frames", "5"],
        read(&log).as_bytes(),
    )
    .ok();
    let doc = Json::parse(&out.stdout).unwrap();
    assert_eq!(doc.str("log.path"), "-");
    assert_eq!(
        doc.num("log.bytes"),
        std::fs::metadata(&log).unwrap().len() as f64
    );
}

#[test]
fn bad_arguments_exit_with_status_2() {
    let log = fixture(LOG);
    let log = log.to_str().unwrap();
    let cases: &[(&[&str], &str)] = &[
        (&["bench"], "missing argument <custom.log>"),
        (&["bench", log, "extra"], "unexpected argument 'extra'"),
        (&["bench", log, "--bogus"], "unknown option '--bogus'"),
        (&["bench", log, "--every", "5"], "unknown option '--every'"),
        (
            &["bench", log, "--frames", "0"],
            "invalid value '0' for --frames",
        ),
        (
            &["bench", log, "--frames", "10000001"],
            "invalid value '10000001' for --frames",
        ),
        (
            &["bench", log, "--frames", "many"],
            "invalid value 'many' for --frames",
        ),
        (&["bench", log, "--dt", "0"], "invalid value '0' for --dt"),
        (
            &["bench", log, "--dt", "-0.5"],
            "invalid value '-0.5' for --dt",
        ),
        (&["bench", log, "--dt"], "option '--dt' requires a value"),
        (
            &["bench", log, "--json=yes"],
            "option '--json' does not take a value",
        ),
        (
            &["bench", log, "--json", "--json"],
            "option '--json' given more than once",
        ),
        (
            &["bench", log, "--width", "0"],
            "invalid value '0' for --width",
        ),
        (
            &["bench", log, "--options", "seconds-per-day"],
            "invalid --options entry 'seconds-per-day'",
        ),
        (
            &["bench", log, "--options", "no-such-option=1"],
            "invalid Gource option 'no-such-option=1'",
        ),
    ];
    for (args, needle) in cases {
        run(*args).fails(2, needle);
    }
}

#[test]
fn missing_log_exits_with_status_1() {
    let dir = scratch("missing_log_exits_with_status_1");
    let missing = dir.join("nope.log");
    run(["bench".as_ref(), missing.as_os_str()]).fails(1, "cannot open");
}

// The tests' own JSON parser must reject what a sloppy writer could emit, or
// the tests above prove nothing.

#[test]
fn test_parser_accepts_valid_json() {
    let doc =
        Json::parse(r#" {"a": [1, -2.5e3, true, false, null, "x\n\u00e9\ud83c\udf32"], "b": {}} "#)
            .unwrap();
    assert_eq!(
        doc.get("a"),
        Some(&Json::Array(vec![
            Json::Num(1.0),
            Json::Num(-2500.0),
            Json::Bool(true),
            Json::Bool(false),
            Json::Null,
            Json::Str("x\n\u{e9}\u{1f332}".into()),
        ]))
    );
    assert_eq!(doc.get("b"), Some(&Json::Object(Vec::new())));
    assert_eq!(doc.get("a.x"), None);
}

#[test]
fn test_parser_rejects_invalid_json() {
    for bad in [
        "",
        "{",
        "[1,]",
        "{\"a\":1,}",
        "01",
        "1.",
        ".5",
        "+1",
        "1e",
        "NaN",
        "inf",
        "-",
        "\"a",
        "\"\n\"",
        "\"\\x\"",
        "{\"a\":1,\"a\":2}",
        "1 2",
        "{\"a\" 1}",
        "tru",
        "'a'",
        "\"\\ud800\"",
        "\"\\u12\"",
    ] {
        assert!(Json::parse(bad).is_err(), "{bad:?} parsed");
    }
}
