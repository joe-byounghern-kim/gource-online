// SPDX-License-Identifier: GPL-3.0-or-later

//! `gource-rs convert`, run as the built binary.

mod common;

use common::{fixture, fixtures, read, run, run_with_stdin, scratch, write};

/// `git log --reverse --raw --encoding=UTF-8 --no-renames --no-show-signature
/// --pretty=format:user:%aN%n%ct` output with the cases that matter: an add,
/// a C-quoted path, a path with `|`, a modify, a merge header (no files) right
/// before the next commit, a delete, and no trailing newline as git emits it.
const GIT_RAW: &str = "\
user:Jane Doe
1700000000
:000000 100644 0000000 d00491f A\tsrc/main.rs
:000000 100644 0000000 587be6b A\t\"sub/\\303\\274 \\303\\261.txt\"
:000000 100644 0000000 718f4d2 A\tdocs/a|b.md

user:John Roe
1700000100
:100644 100644 d00491f 1191247 M\tsrc/main.rs

user:Merger
1700000200
user:Jane Doe
1700000300
:100644 000000 1191247 0000000 D\tsrc/main.rs";

const GIT_EXPECTED: &str = "\
1700000000|Jane Doe|A|/src/main.rs
1700000000|Jane Doe|A|/sub/\u{fc} \u{f1}.txt
1700000000|Jane Doe|A|/docs/a_b.md
1700000100|John Roe|M|/src/main.rs
1700000300|Jane Doe|D|/src/main.rs
";

#[test]
fn git_raw_file_to_custom_file() {
    let dir = scratch("git_raw_file_to_custom_file");
    let input = write(&dir, "history.raw", GIT_RAW);
    let output = dir.join("history.log");

    let run = run([
        "convert".as_ref(),
        "--from".as_ref(),
        "git".as_ref(),
        input.as_os_str(),
        output.as_os_str(),
    ])
    .ok();

    assert_eq!(read(&output), GIT_EXPECTED);
    assert_eq!(run.stdout, "", "the log went to the file");
    assert_eq!(
        run.stderr,
        "gource-rs: converted 3 commits, 5 file changes from git input\n"
    );
}

#[test]
fn git_raw_stdin_to_stdout() {
    let run = run_with_stdin(["convert", "--from", "git", "-", "-"], GIT_RAW.as_bytes()).ok();
    assert_eq!(run.stdout, GIT_EXPECTED);
}

/// The clone service's format with AI detection (an `m1:` line after the
/// timestamp: raw identities and trailers split by 0x1f, 0x1e, subject),
/// with a merge header whose credits must not reach the next commit.
const GIT_RAW_AI: &str = "\
user:jane
1790942378
m1:jane\x1fjane@example.com\x1fjane\x1fjane@example.com\x1fCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\x1fCo-authored-by: Bob <bob@example.com>\x1efeat:\twith a tab
:000000 100644 0000000 d00491f A\tsrc/a.rs
:000000 100644 0000000 d00491f A\tsrc/b.rs

user:Cursor Agent
1790942500
m1:Cursor Agent\x1fcursoragent@cursor.com\x1fCursor Agent\x1fcursoragent@cursor.com\x1f\x1eagent
:000000 100644 0000000 d00491f A\tx.ts

user:jane
1790942600
m1:jane\x1fjane@example.com\x1fjane\x1fjane@example.com\x1fMade-with: Cursor\x1emerge
user:jane
1790942700
m1:jane\x1fjane@example.com\x1fjane\x1fjane@example.com\x1fCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>\x1fCo-authored-by: Codex <noreply@openai.com>\x1efix
:100644 100644 d00491f 1191247 M\ta.rs

user:bob
1790942800
m1:bob\x1fbob@example.com\x1fbob\x1fbob@example.com\x1f\x1eplain
:100644 100644 d00491f 1191247 M\ta.rs
";

#[test]
fn git_raw_with_ai_detection_carries_attributions() {
    let run = run_with_stdin(
        ["convert", "--from", "git", "-", "-"],
        GIT_RAW_AI.as_bytes(),
    )
    .ok();
    assert_eq!(
        run.stdout,
        "\
1790942378|jane|A|/src/a.rs||ai=co:claude:Claude Opus 5.5
1790942378|jane|A|/src/b.rs
1790942500|Cursor Agent|A|/x.ts||ai=au:cursor
1790942700|jane|M|/a.rs||ai=co:claude:Claude Opus 5.5,co:codex
1790942800|bob|M|/a.rs
"
    );
    assert!(!run.stdout.contains('@'));

    // and the custom log reads back with the same credits
    let again = run_with_stdin(
        ["convert", "--from", "custom", "-", "-"],
        run.stdout.as_bytes(),
    )
    .ok();
    assert_eq!(again.stdout, run.stdout);
}

#[test]
fn git_raw_input_does_not_depend_on_read_boundaries() {
    // Much more than one read buffer, so the parser sees many chunks and
    // lines split across them.
    let mut big = String::new();
    let mut expected = String::new();
    for i in 0..5000 {
        big.push_str(&format!(
            "user:User {}\n{}\n:000000 100644 0000000 d00491f A\tdir{}/file{i}.rs\n\n",
            i % 7,
            1_700_000_000 + i,
            i % 13
        ));
        expected.push_str(&format!(
            "{}|User {}|A|/dir{}/file{i}.rs\n",
            1_700_000_000 + i,
            i % 7,
            i % 13
        ));
    }
    assert!(big.len() > 4 * 64 * 1024);
    let run = run_with_stdin(["convert", "--from", "git", "-", "-"], big.as_bytes()).ok();
    assert_eq!(run.stdout, expected);
}

#[test]
fn custom_log_is_normalised() {
    // CRLF line ends, an ISO date, `#`-prefixed and lower-case colours, an
    // empty username and action, extra columns, and two lines that are not
    // records (an action other than A/D/M, a timestamp that is not one).
    let input = "\
2010-01-01T03:04:05Z|alice|A|src/a.rs\r
1|bob|M|/b.rs|#abcdef
2|bob|X|/odd action.rs
3||D|/gone/
4|carol|M|/has|more|columns
not-a-timestamp|zed|A|/skipped.rs
5|dave||/after.rs
6|erin|A|/colour.rs|0a0B0c
";
    let expected = "\
1262315045|alice|A|/src/a.rs
1|bob|M|/b.rs|ABCDEF
3|Unknown|D|/gone/
4|carol|M|/has
5|dave|A|/after.rs
6|erin|A|/colour.rs|0A0B0C
";
    let run = run_with_stdin(["convert", "--from", "custom", "-", "-"], input.as_bytes()).ok();
    assert_eq!(run.stdout, expected);
    assert_eq!(
        run.stderr,
        "gource-rs: converted 6 commits, 6 file changes from custom input (2 lines skipped)\n"
    );
}

#[test]
fn custom_to_custom_round_trips_every_fixture() {
    let dir = scratch("custom_to_custom_round_trips_every_fixture");
    for fixture in fixtures() {
        let name = fixture.file_name().unwrap().to_str().unwrap().to_string();
        let once = dir.join(format!("{name}.1"));
        let twice = dir.join(format!("{name}.2"));

        run([
            "convert".as_ref(),
            "--from".as_ref(),
            "custom".as_ref(),
            fixture.as_os_str(),
            once.as_os_str(),
        ])
        .ok();
        run([
            "convert".as_ref(),
            "--from".as_ref(),
            "custom".as_ref(),
            once.as_os_str(),
            twice.as_os_str(),
        ])
        .ok();

        let first = read(&once);
        assert_eq!(first, read(&twice), "{name}: converting twice changes it");
        // Every output line is a well-formed record.
        for line in first.lines() {
            let fields: Vec<&str> = line.split('|').collect();
            assert!((4..=6).contains(&fields.len()), "{name}: {line:?}");
            // a sixth field is AI credits, after an explicit colour slot
            if fields.len() == 6 {
                assert!(fields[5].starts_with("ai="), "{name}: {line:?}");
            }
            assert!(fields[0].parse::<i64>().is_ok(), "{name}: {line:?}");
            assert!(["A", "M", "D"].contains(&fields[2]), "{name}: {line:?}");
            assert!(fields[3].starts_with('/'), "{name}: {line:?}");
        }
    }
}

#[test]
fn clean_fixture_only_gains_leading_slashes() {
    // Every line of this fixture is well formed, so the conversion is the
    // identity apart from the rooted path.
    let input = read(&fixture("visual-hierarchy-activity.log"));
    let expected: String = input
        .lines()
        .map(|line| {
            let mut fields: Vec<String> = line.split('|').map(str::to_string).collect();
            if !fields[3].starts_with('/') {
                fields[3].insert(0, '/');
            }
            fields.join("|") + "\n"
        })
        .collect();

    let run = run([
        "convert".as_ref(),
        "--from".as_ref(),
        "custom".as_ref(),
        fixture("visual-hierarchy-activity.log").as_os_str(),
        "-".as_ref(),
    ])
    .ok();
    assert_eq!(run.stdout, expected);
}

#[test]
fn empty_input_gives_empty_output() {
    for from in ["git", "custom"] {
        let run = run_with_stdin(["convert", "--from", from, "-", "-"], b"").ok();
        assert_eq!(run.stdout, "");
        assert!(
            run.stderr.contains("converted 0 commits, 0 file changes"),
            "{}",
            run.stderr
        );
    }
}

#[test]
fn output_file_is_replaced() {
    let dir = scratch("output_file_is_replaced");
    let output = write(
        &dir,
        "out.log",
        "stale contents that are longer than the result\n",
    );
    let input = write(&dir, "in.log", "1|a|A|/x\n");
    run([
        "convert".as_ref(),
        "--from".as_ref(),
        "custom".as_ref(),
        input.as_os_str(),
        output.as_os_str(),
    ])
    .ok();
    assert_eq!(read(&output), "1|a|A|/x\n");
}

#[test]
fn input_and_output_must_differ() {
    let dir = scratch("input_and_output_must_differ");
    let file = write(&dir, "same.log", "1|a|A|/x\n");
    // The same file through a different spelling of its path.
    let alias = dir.join(".").join("same.log");
    for output in [&file, &alias] {
        run([
            "convert".as_ref(),
            "--from".as_ref(),
            "custom".as_ref(),
            file.as_os_str(),
            output.as_os_str(),
        ])
        .fails(1, "input and output are the same file");
        assert_eq!(read(&file), "1|a|A|/x\n", "the input was clobbered");
    }
}

#[test]
fn unreadable_input_and_unwritable_output_fail_with_status_1() {
    let dir = scratch("unreadable_input_and_unwritable_output");
    let missing = dir.join("missing.log");
    run([
        "convert".as_ref(),
        "--from".as_ref(),
        "custom".as_ref(),
        missing.as_os_str(),
        "-".as_ref(),
    ])
    .fails(1, "cannot open");

    let input = write(&dir, "in.log", "1|a|A|/x\n");
    let nowhere = dir.join("no-such-dir").join("out.log");
    run([
        "convert".as_ref(),
        "--from".as_ref(),
        "custom".as_ref(),
        input.as_os_str(),
        nowhere.as_os_str(),
    ])
    .fails(1, "cannot create");

    // A directory is not a log.
    run([
        "convert".as_ref(),
        "--from".as_ref(),
        "custom".as_ref(),
        dir.as_os_str(),
        "-".as_ref(),
    ])
    .fails(1, "cannot read");
}

#[test]
fn from_is_required_and_checked() {
    run(["convert", "in.log", "out.log"]).fails(2, "missing required option '--from'");
    run(["convert", "--from", "svn", "in.log", "out.log"]).fails(
        2,
        "invalid value 'svn' for --from: expected 'git' or 'custom'",
    );
    run(["convert", "--from"]).fails(2, "option '--from' requires a value");
    run(["convert", "--from", "git", "--from", "custom", "a", "b"])
        .fails(2, "option '--from' given more than once");
}

#[test]
fn positionals_are_checked() {
    run(["convert", "--from", "git"]).fails(2, "missing argument <in>");
    run(["convert", "--from", "git", "in.log"]).fails(2, "missing argument <out>");
    run(["convert", "--from", "git", "a", "b", "c"]).fails(2, "unexpected argument 'c'");
}

// --- The clone service's pipeline (limits, pull requests, progress) ---

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn convert_git(extra: &[&str]) -> common::Run {
    let mut args = vec!["convert", "--from", "git"];
    args.extend_from_slice(extra);
    args.extend_from_slice(&["-", "-"]);
    run_with_stdin(args, GIT_RAW.as_bytes())
}

#[test]
fn limits_not_reached_change_nothing() {
    let run = convert_git(&[
        "--max-bytes",
        "1000000",
        "--max-changes",
        "1000",
        "--max-commit-files",
        "1000",
    ])
    .ok();
    assert_eq!(run.stdout, GIT_EXPECTED);
}

#[test]
fn limit_on_log_bytes_stops_with_status_3() {
    let run = convert_git(&["--max-bytes", "40"]);
    assert_eq!(run.code, Some(3), "{run:#?}");
    assert!(
        run.stderr
            .contains("gource-rs: error: limit exceeded: log_bytes 40"),
        "{run:#?}"
    );
    assert_eq!(run.stdout, "", "the first commit is refused whole");
}

#[test]
fn limit_on_changes() {
    let run = convert_git(&["--max-changes", "4"]);
    assert_eq!(run.code, Some(3), "{run:#?}");
    assert!(run.stderr.contains("limit exceeded: changes 4"), "{run:#?}");
}

#[test]
fn limit_on_commit_files() {
    let run = convert_git(&["--max-commit-files", "2"]);
    assert_eq!(run.code, Some(3), "{run:#?}");
    assert!(
        run.stderr.contains("limit exceeded: commit_files 2"),
        "{run:#?}"
    );
    assert_eq!(run.stdout, "", "nothing of the oversized commit is written");
}

#[test]
fn max_commit_files_is_for_git_input_only() {
    run_with_stdin(
        [
            "convert",
            "--from",
            "custom",
            "--max-commit-files",
            "2",
            "-",
            "-",
        ],
        b"1|a|A|/x\n",
    )
    .fails(2, "--max-commit-files applies to git input only");
}

const PULLS: &str = "7\t1700000050\t1700000250\t1700000250\talice\tFix the seek bar\n";

#[test]
fn pull_requests_are_merged_by_time() {
    let dir = scratch("pull_requests_are_merged_by_time");
    let pulls = write(&dir, "pulls.tsv", PULLS);
    let run = run_with_stdin(
        [
            "convert".as_ref(),
            "--from".as_ref(),
            "git".as_ref(),
            "--pulls".as_ref(),
            pulls.as_os_str(),
            "-".as_ref(),
            "-".as_ref(),
        ],
        GIT_RAW.as_bytes(),
    )
    .ok();
    assert_eq!(
        run.stdout,
        "\
1700000000|Jane Doe|A|/src/main.rs
1700000000|Jane Doe|A|/sub/\u{fc} \u{f1}.txt
1700000000|Jane Doe|A|/docs/a_b.md
1700000050|alice|A|/pull requests/#7 Fix the seek bar|3FB950|ev=pr-opened
1700000100|John Roe|M|/src/main.rs
1700000250|alice|D|/pull requests/#7 Fix the seek bar|A371F7|ev=pr-merged
1700000300|Jane Doe|D|/src/main.rs
"
    );
    assert_eq!(
        run.stderr,
        "gource-rs: converted 3 commits, 5 file changes from git input, 2 pull request events\n"
    );
}

#[test]
fn pull_requests_outside_the_window_are_dropped_or_clamped() {
    let dir = scratch("pull_requests_window");
    let pulls = write(&dir, "pulls.tsv", PULLS);
    let run = run_with_stdin(
        [
            "convert".as_ref(),
            "--from".as_ref(),
            "git".as_ref(),
            "--pulls".as_ref(),
            pulls.as_os_str(),
            "--pulls-since".as_ref(),
            "1700000060".as_ref(),
            "--pulls-until".as_ref(),
            "1700000299".as_ref(),
            "-".as_ref(),
            "-".as_ref(),
        ],
        GIT_RAW.as_bytes(),
    )
    .ok();
    // Opened before the window and merged inside it: opened at the window's start.
    assert!(
        run.stdout.contains(
            "1700000060|alice|A|/pull requests/#7 Fix the seek bar|3FB950|ev=pr-opened\n"
        ),
        "{run:#?}"
    );
    assert!(run.stdout.contains("1700000250|alice|D|"), "{run:#?}");
}

#[test]
fn pulls_window_needs_pulls() {
    convert_git(&["--pulls-since", "1"]).fails(2, "--pulls-since and --pulls-until need --pulls");
}

#[test]
fn a_bad_pulls_file_names_the_line() {
    let dir = scratch("a_bad_pulls_file_names_the_line");
    let pulls = write(&dir, "pulls.tsv", "x\t1\t-\t-\ta\tt\n");
    run_with_stdin(
        [
            "convert".as_ref(),
            "--from".as_ref(),
            "git".as_ref(),
            "--pulls".as_ref(),
            pulls.as_os_str(),
            "-".as_ref(),
            "-".as_ref(),
        ],
        GIT_RAW.as_bytes(),
    )
    .fails(1, "line 1: number is not an integer");
}

#[test]
fn progress_json_reports_on_stderr() {
    let run = convert_git(&["--progress-json"]).ok();
    assert_eq!(run.stdout, GIT_EXPECTED);
    let last_json = run
        .stderr
        .lines()
        .rfind(|l| l.starts_with('{'))
        .expect("a progress line");
    // Four headers (the merge's included); every byte of the log.
    assert_eq!(
        last_json,
        format!("{{\"commits\":4,\"bytes\":{}}}", GIT_EXPECTED.len())
    );
}

#[test]
fn nothing_but_the_log_leaves_the_parser() {
    // The clone service's privacy rule:
    // no email address, trailer text or subject reaches stdout or stderr.
    let run = run_with_stdin(
        ["convert", "--from", "git", "--progress-json", "-", "-"],
        GIT_RAW_AI.as_bytes(),
    )
    .ok();
    for text in [&run.stdout, &run.stderr] {
        assert!(!text.contains('@'), "{text}");
        assert!(
            !text.contains("Co-authored-by") && !text.contains("Co-Authored-By"),
            "{text}"
        );
        assert!(!text.contains("Made-with"), "{text}");
        assert!(
            !text.contains("with a tab") && !text.contains("merge"),
            "{text}"
        );
    }
}

fn spawn_convert() -> std::process::Child {
    Command::new(env!("CARGO_BIN_EXE_gource-rs"))
        .args(["convert", "--from", "git", "-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("gource-rs runs")
}

#[test]
fn output_is_flushed_per_chunk() {
    let mut child = spawn_convert();
    let mut stdin = child.stdin.take().unwrap();
    // One complete commit, then the next header so the parser emits it.
    stdin
        .write_all(b"user:A\n1700000000\n:000000 100644 0000000 d00491f A\ta.txt\n\nuser:B\n")
        .unwrap();
    stdin.flush().unwrap();

    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        BufReader::new(stdout).read_line(&mut line).unwrap();
        tx.send(line).unwrap();
    });
    let line = rx
        .recv_timeout(Duration::from_secs(10))
        .expect("the first commit arrives while input is still open");
    assert_eq!(line, "1700000000|A|A|/a.txt\n");
    drop(stdin);
    assert!(child.wait().unwrap().success());
}

#[test]
fn stops_when_the_reader_goes_away() {
    let mut child = spawn_convert();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = child.stdout.take().unwrap();
    let writer = std::thread::spawn(move || {
        for i in 0..200_000u64 {
            let commit = format!(
                "user:U\n{}\n:000000 100644 0000000 d00491f A\tdir/file{i}.rs\n\n",
                1_700_000_000 + i
            );
            if stdin.write_all(commit.as_bytes()).is_err() {
                return; // the converter is gone: expected
            }
        }
    });
    let mut first = [0u8; 100];
    stdout.read_exact(&mut first).unwrap();
    drop(stdout); // the reader goes away

    let deadline = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "gource-rs did not stop after its reader left"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert!(matches!(status.code(), Some(0) | Some(1)), "{status:?}");
    writer.join().unwrap();
}
