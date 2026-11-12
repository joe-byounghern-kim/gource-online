// SPDX-License-Identifier: GPL-3.0-or-later

//! Opt-in, server-facing source receipts. These assertions describe final writes,
//! not independent proof that an arbitrary supplied stream came from a repository.
mod common;

use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use common::{Json, read, run, run_with_stdin, scratch, write};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccc";
const D: &str = "dddddddddddddddddddddddddddddddddddddddd";
const SECRET: &str = "private-person@example.test";
const TOKEN: &str = "ghp_SEEDED_NOT_A_REAL_CREDENTIAL";

fn raw(sha: &str, time: i64, files: &str) -> String {
    format!("user:{SECRET}\n{time}\nsource:{sha}\n{files}")
}
fn file(path: &str) -> String {
    format!(":000000 100644 0000000 d00491f A\t{path}\n")
}
fn convert(dir: &Path, input: &str, actors: &str) -> common::Run {
    let map = write(dir, "actors.tsv", actors);
    let receipts = dir.join("receipts.jsonl");
    run_with_stdin(
        [
            OsStr::new("convert"),
            OsStr::new("--from"),
            OsStr::new("git"),
            OsStr::new("--source-actors"),
            map.as_os_str(),
            OsStr::new("--source-receipts"),
            receipts.as_os_str(),
            OsStr::new("-"),
            OsStr::new("-"),
        ],
        input.as_bytes(),
    )
}
fn receipts(dir: &Path) -> Vec<Json> {
    read(&dir.join("receipts.jsonl"))
        .lines()
        .map(|line| Json::parse(line).unwrap())
        .collect()
}
fn assert_private(run: &common::Run) {
    for output in [&run.stdout, &run.stderr] {
        assert!(!output.contains(SECRET), "raw identity escaped: {output}");
        assert!(!output.contains(TOKEN), "raw token escaped: {output}");
    }
}

#[test]
fn source_labels_and_exact_final_spans_replace_only_the_actor() {
    let dir = scratch("source_labels_and_exact_final_spans");
    let input = format!(
        "{}\n{}\n{}\n{}",
        raw(A, 1700000002, &(file("src/a.rs") + &file("src/b.rs"))),
        raw(B, 1700000001, ""),
        raw(C, 1700000000, &file("old.rs")),
        raw(D, 1700000002, &file("src/a.rs"))
    );
    let result = convert(
        &dir,
        &input,
        &format!(
            "{A}\tcontributor_1\n{B}\tunknown_merge\n{C}\tcontributor_1\n{D}\tunknown_author\n"
        ),
    )
    .ok();
    assert_private(&result);
    assert_eq!(
        result.stdout,
        "1700000002|contributor_1|A|/src/a.rs\n1700000002|contributor_1|A|/src/b.rs\n1700000000|contributor_1|A|/old.rs\n1700000002|unknown_author|A|/src/a.rs\n"
    );
    let rows = receipts(&dir);
    assert_eq!(
        rows.len(),
        3,
        "fileless headers must not produce zero-length receipts"
    );
    for (row, (sha, label, start, end)) in rows.iter().zip([
        (A, "contributor_1", 0, 2),
        (C, "contributor_1", 2, 3),
        (D, "unknown_author", 3, 4),
    ]) {
        assert_eq!(
            row.keys(""),
            ["schema", "kind", "sha", "label", "start", "end"]
        );
        assert_eq!(row.num("schema"), 1.0);
        assert_eq!(row.str("kind"), "final-custom-line-span");
        assert_eq!(row.str("sha"), sha);
        assert_eq!(row.str("label"), label);
        assert_eq!(row.num("start"), start as f64);
        assert_eq!(row.num("end"), end as f64);
    }
}

#[test]
fn source_actor_replacement_happens_after_f1_and_mailmap_classification() {
    let dir = scratch("source_post_f1_mailmap");
    let input = format!(
        "user:Cursor Agent\n1700000000\nm1:Cursor Agent\x1fcursoragent@cursor.com\x1fCursor Agent\x1fcursoragent@cursor.com\x1f\x1e{SECRET} {TOKEN}\nsource:{A}\n{}\nuser:Mapped Human\n1700000001\nm1:Cursor Agent\x1fcursoragent@cursor.com\x1fCursor Agent\x1fcursoragent@cursor.com\x1fCo-authored-by: Claude <noreply@anthropic.com>\x1eunexported\nsource:{B}\n{}",
        file("a.rs"),
        file("b.rs")
    );
    let normal_input = input
        .lines()
        .filter(|line| !line.starts_with("source:"))
        .collect::<Vec<_>>()
        .join("\n");
    let ordinary = run_with_stdin(
        ["convert", "--from", "git", "-", "-"],
        normal_input.as_bytes(),
    )
    .ok();
    let result = convert(
        &dir,
        &input,
        &format!("{A}\topaque_agent\n{B}\topaque_human\n"),
    )
    .ok();
    let expected = ordinary
        .stdout
        .replace("|Cursor Agent|", "|opaque_agent|")
        .replace("|Mapped Human|", "|opaque_human|");
    assert_eq!(
        result.stdout, expected,
        "opaque labels must not turn an agent verdict into assist"
    );
    assert!(result.stdout.contains("ai=au:cursor"));
    assert!(result.stdout.contains("ai=co:"));
    assert_private(&result);
}

#[test]
fn source_mode_rejects_ambiguous_or_truncated_headers_without_echoing_input() {
    let malformed = [
        format!("user:{SECRET}\n1700000000\n{}", file("a")),
        format!("user:{SECRET}"),
        format!("user:{SECRET}\n1700000000"),
        format!("user:{SECRET}\n1700000000\nm1:{TOKEN}"),
        format!("{}source:{A}\n", raw(A, 1700000000, &file("a"))),
        format!("user:{SECRET}\n1700000000\nsource:{TOKEN}\n{}", file("a")),
        format!(
            "{}user:{TOKEN}\n1700000001\nsource:{B}\n{}",
            raw(A, 1700000000, &file("a")),
            file("b")
        ),
        format!(
            "{}\n{}",
            raw(A, 1700000000, &file("a")),
            raw(A, 1700000001, &file("b"))
        ),
        raw(A, 1700000000, "not-a-git-record\tpath\n"),
        raw(A, 1700000000, ":000000 100644 bogus d00491f A\tpath\n"),
        raw(
            A,
            1700000000,
            ":000000 100644 0000000 d00491f R100\told\tnew\n",
        ),
        format!(
            "user:{SECRET}\n1700000000garbage\nsource:{A}\n{}",
            file("a")
        ),
        raw(A, 0, &file("a")),
        raw(A, 1700000000, "\"garbage\"\tpath\n"),
        format!(
            "user:{SECRET}\n1700000000\nsource:{A}\nm1:misplaced\n{}",
            file("a")
        ),
    ];
    for (i, input) in malformed.iter().enumerate() {
        let dir = scratch(&format!("source_bad_header_{i}"));
        let result = convert(&dir, input, &format!("{A}\topaque\n{B}\topaque2\n"));
        assert_eq!(result.code, Some(1), "case {i}: {result:#?}");
        assert_private(&result);
        assert!(
            !dir.join("receipts.jsonl").exists(),
            "failed conversion leaves no valid receipt file"
        );
    }
}

#[test]
fn source_actor_map_is_bounded_strict_ascii_and_requires_full_unique_shas() {
    let malformed = [
        format!("\u{feff}{A}\topaque\n"),
        format!("{A}\topaque\r\n"),
        format!("{A}\topaque\n\n"),
        format!("{A}\topaque\textra\n"),
        format!("{A}\topaque"),
        format!("{}\topaque\n", A.to_uppercase()),
        "aaaaaaa\topaque\n".to_owned(),
        format!("{A}\topaque\n{A}\tother\n"),
        format!("{A}\t{SECRET}\n"),
        format!("{A}\t{TOKEN}\n"),
        format!("{A}\t{}\n", "x".repeat(129)),
    ];
    for (i, map) in malformed.iter().enumerate() {
        let dir = scratch(&format!("source_bad_map_{i}"));
        let result = convert(&dir, &raw(A, 1700000000, &file("a")), map);
        assert_eq!(result.code, Some(1), "case {i}: {result:#?}");
        assert_private(&result);
        assert!(!dir.join("receipts.jsonl").exists());
    }
}

#[test]
fn source_mode_requires_paired_flags_git_input_and_separate_regular_outputs() {
    let dir = scratch("source_usage");
    let map = write(&dir, "actors.tsv", format!("{A}\topaque\n"));
    let receipt = dir.join("receipts.jsonl");
    for args in [
        vec![
            "convert",
            "--from",
            "git",
            "--source-actors",
            map.to_str().unwrap(),
            "-",
            "-",
        ],
        vec![
            "convert",
            "--from",
            "custom",
            "--source-actors",
            map.to_str().unwrap(),
            "--source-receipts",
            receipt.to_str().unwrap(),
            "-",
            "-",
        ],
        vec![
            "convert",
            "--from",
            "git",
            "--source-actors",
            map.to_str().unwrap(),
            "--source-receipts",
            "-",
            "-",
            "-",
        ],
        vec![
            "convert",
            "--from",
            "git",
            "--source-actors",
            map.to_str().unwrap(),
            "--source-receipts",
            receipt.to_str().unwrap(),
            "--pulls",
            map.to_str().unwrap(),
            "-",
            "-",
        ],
    ] {
        let result = run(args);
        assert_eq!(result.code, Some(2));
        assert!(result.stderr.contains("source"));
        assert!(!receipt.exists());
    }
    let input = write(&dir, "input.raw", raw(A, 1700000000, &file("a")));
    let original = read(&input);
    let output = dir.join("new.log");
    let result = run([
        OsStr::new("convert"),
        OsStr::new("--from"),
        OsStr::new("git"),
        OsStr::new("--source-actors"),
        map.as_os_str(),
        OsStr::new("--source-receipts"),
        output.as_os_str(),
        input.as_os_str(),
        output.as_os_str(),
    ]);
    assert_eq!(
        result.code,
        Some(2),
        "new output/receipt aliases must fail before creation"
    );
    assert!(!output.exists());
    assert_eq!(read(&input), original);
    assert_eq!(read(&map), format!("{A}\topaque\n"));
}

#[test]
fn source_log_limit_does_not_leave_accepted_receipts() {
    let dir = scratch("source_log_limit");
    let map = write(&dir, "actors.tsv", format!("{A}\topaque\n"));
    let receipts = dir.join("receipts.jsonl");
    let result = run_with_stdin(
        [
            OsStr::new("convert"),
            OsStr::new("--from"),
            OsStr::new("git"),
            OsStr::new("--source-actors"),
            map.as_os_str(),
            OsStr::new("--source-receipts"),
            receipts.as_os_str(),
            OsStr::new("--max-bytes"),
            OsStr::new("1"),
            OsStr::new("-"),
            OsStr::new("-"),
        ],
        raw(A, 1700000000, &file("a")).as_bytes(),
    );
    assert_eq!(result.code, Some(3), "{result:#?}");
    assert!(result.stdout.is_empty());
    assert!(!receipts.exists());
}

#[test]
fn source_closed_stdout_is_failure_while_ordinary_pipe_behavior_is_preserved() {
    // More output than a pipe holds. A parallel test's child can briefly inherit
    // the read end between fork and exec; a small write would then succeed into
    // the pipe buffer. A full pipe blocks the writer until that copy closes too.
    let files: String = (0..10_000)
        .map(|i| file(&format!("dir/file-{i:05}")))
        .collect();
    let input = raw(A, 1700000000, &files);
    let control = convert(
        &scratch("source_closed_stdout_control"),
        &input,
        &format!("{A}\topaque\n"),
    );
    assert_eq!(control.code, Some(0), "{control:#?}");
    assert!(
        control.stdout.len() > 256 * 1024,
        "{}",
        control.stdout.len()
    );
    let dir = scratch("source_closed_stdout");
    let map = write(&dir, "actors.tsv", format!("{A}\topaque\n"));
    let receipt = dir.join("receipts.jsonl");
    let mut child = Command::new(env!("CARGO_BIN_EXE_gource-rs"))
        .args(["convert", "--from", "git", "--source-actors"])
        .arg(&map)
        .arg("--source-receipts")
        .arg(&receipt)
        .args(["-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    use std::io::Write;
    // The child stops reading once its output fails, so this write may fail too.
    let _ = child.stdin.take().unwrap().write_all(input.as_bytes());
    let result = child.wait_with_output().unwrap();
    assert_eq!(
        result.status.code(),
        Some(1),
        "source BrokenPipe must not count as successful acquisition"
    );
    assert!(!receipt.exists());
    assert!(!String::from_utf8_lossy(&result.stderr).contains(SECRET));
}

#[test]
fn source_paths_do_not_export_private_identity_or_credentials() {
    for (i, path) in [
        TOKEN,
        "../escape",
        "a|b",
        "tab\tname",
        r#""ghp_\123EEDED_NOT_A_REAL_CREDENTIAL""#,
        r#""tab\011name""#,
        r#""back\\slash""#,
        "bidi\u{202e}name",
    ]
    .iter()
    .enumerate()
    {
        let dir = scratch(&format!("source_private_path_{i}"));
        let result = convert(
            &dir,
            &raw(A, 1700000000, &file(path)),
            &format!("{A}\topaque\n"),
        );
        assert_eq!(result.code, Some(1), "unsafe path {i}: {result:#?}");
        assert_private(&result);
        assert!(!dir.join("receipts.jsonl").exists());
    }
}

#[test]
fn source_paths_keep_at_signs_while_the_author_stays_a_label() {
    let dir = scratch("source_path_at_signs");
    let files = [
        "logo@2x.png",
        "node_modules/@types/a.d.ts",
        r#""quoted\100name.tsx""#,
    ]
    .map(file)
    .concat();
    let result = convert(&dir, &raw(A, 1700000000, &files), &format!("{A}\topaque\n")).ok();
    assert_eq!(
        result.stdout,
        "1700000000|opaque|A|/logo@2x.png\n\
         1700000000|opaque|A|/node_modules/@types/a.d.ts\n\
         1700000000|opaque|A|/quoted@name.tsx\n"
    );
    assert_private(&result);
    assert_eq!(receipts(&dir)[0].str("label"), "opaque");

    let dir = scratch("source_label_at_sign");
    let refused = convert(
        &dir,
        &raw(A, 1700000000, &file("logo@2x.png")),
        &format!("{A}\t{SECRET}\n"),
    );
    assert_eq!(refused.code, Some(1), "{refused:#?}");
    assert_private(&refused);
    assert!(!dir.join("receipts.jsonl").exists());
}

#[test]
fn source_f1_models_cannot_export_known_token_markers() {
    let dir = scratch("source_private_aider_model");
    let input = format!(
        "user:Human\n1700000000\nm1:Human\x1f{SECRET}\x1fHuman\x1f{SECRET}\x1fCo-authored-by: aider ({TOKEN}) <aider@aider.chat>\x1eignored\nsource:{A}\n{}",
        file("a")
    );
    let ordinary = run_with_stdin(
        ["convert", "--from", "git", "-", "-"],
        input
            .lines()
            .filter(|line| !line.starts_with("source:"))
            .collect::<Vec<_>>()
            .join("\n")
            .as_bytes(),
    )
    .ok();
    assert!(
        ordinary.stdout.contains(TOKEN),
        "fixture must exercise a model accepted by normal F1 classification"
    );
    let result = convert(&dir, &input, &format!("{A}\topaque\n"));
    assert_eq!(result.code, Some(1));
    assert_private(&result);
    assert!(!dir.join("receipts.jsonl").exists());
}

#[test]
fn source_unicode_negative_time_and_sha256_are_supported_without_ai_metadata() {
    let dir = scratch("source_unicode_sha256");
    let sha = "e".repeat(64);
    let label = "x".repeat(128);
    let input = raw(&sha, -1, &file(r#""unicode/\303\274.txt""#));
    let result = convert(&dir, &input, &format!("{sha}\t{label}\n")).ok();
    assert_eq!(result.stdout, format!("-1|{label}|A|/unicode/ü.txt\n"));
    assert_eq!(receipts(&dir)[0].str("sha"), sha);
}

#[test]
fn source_invalid_utf8_path_is_rejected_before_lossy_filtering() {
    let dir = scratch("source_invalid_utf8");
    let map = write(&dir, "actors.tsv", format!("{A}\topaque\n"));
    let receipt = dir.join("receipts.jsonl");
    let mut input = raw(A, 1700000000, "").into_bytes();
    input.extend_from_slice(b":000000 100644 0000000 d00491f A\tbad\xffname\n");
    let result = run_with_stdin(
        [
            OsStr::new("convert"),
            OsStr::new("--from"),
            OsStr::new("git"),
            OsStr::new("--source-actors"),
            map.as_os_str(),
            OsStr::new("--source-receipts"),
            receipt.as_os_str(),
            OsStr::new("-"),
            OsStr::new("-"),
        ],
        &input,
    );
    assert_eq!(result.code, Some(1));
    assert!(result.stdout.is_empty());
    assert_private(&result);
    assert!(!receipt.exists());
}

#[test]
fn source_malformed_git_quoted_paths_fail_instead_of_silently_disappearing() {
    for (i, path) in [
        r#""""#,
        "\"",
        "\"unfinished",
        "unopened\"",
        r#""unescaped"quote""#,
        r#""unknown\qescape""#,
    ]
    .into_iter()
    .enumerate()
    {
        let dir = scratch(&format!("source_bad_quote_{i}"));
        let result = convert(
            &dir,
            &raw(A, 1700000000, &file(path)),
            &format!("{A}\topaque\n"),
        );
        assert_eq!(result.code, Some(1), "quoted case {i}: {result:#?}");
        assert!(!dir.join("receipts.jsonl").exists());
    }
}

#[test]
fn source_valid_quoted_embedded_quotes_and_source_like_paths_remain_data() {
    let dir = scratch("source_valid_quote");
    let input = raw(
        A,
        1700000000,
        &format!(
            "{}{}",
            file(r#""embedded\"quote.txt""#),
            file("source:filename.txt")
        ),
    );
    let result = convert(&dir, &input, &format!("{A}\topaque\n")).ok();
    assert!(result.stdout.contains("|/embedded\"quote.txt\n"));
    assert!(result.stdout.contains("|/source:filename.txt\n"));
    let rows = receipts(&dir);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].num("end"), 2.0);
}

#[test]
fn ordinary_option_like_filename_or_value_does_not_enable_strict_source_errors() {
    use std::io::Write;
    let dir = scratch("ordinary_source_option_filename");
    let input = "user:ordinary\n1700000000\n:000000 100644 0000000 d00491f A\ta\n";
    write(&dir, "--source-actors", input);
    write(&dir, "--source-receipts=x", input);
    for name in ["--source-actors", "--source-receipts=x"] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_gource-rs"))
            .current_dir(&dir)
            .args(["convert", "--from", "git", "--", name, "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        drop(child.stdout.take());
        let result = child.wait_with_output().unwrap();
        assert!(
            result.status.success(),
            "ordinary literal filename must retain BrokenPipe success"
        );
    }
    write(&dir, "--source-actors", "");
    let mut child = Command::new(env!("CARGO_BIN_EXE_gource-rs"))
        .current_dir(&dir)
        .args([
            "convert",
            "--from",
            "git",
            "--pulls",
            "--source-actors",
            "-",
            "-",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    assert!(
        child.wait_with_output().unwrap().status.success(),
        "ordinary option value must not opt into source mode"
    );
}

#[cfg(unix)]
#[test]
fn source_symlink_and_hardlink_aliases_are_refused_before_truncation() {
    use std::os::unix::fs::symlink;
    for (i, hard) in [false, true].into_iter().enumerate() {
        let dir = scratch(&format!("source_alias_{i}"));
        let map = write(&dir, "actors.tsv", format!("{A}\topaque\n"));
        let input = write(&dir, "input.raw", raw(A, 1700000000, &file("a")));
        let alias = dir.join("alias");
        if hard {
            fs::hard_link(&map, &alias).unwrap();
        } else {
            symlink(&map, &alias).unwrap();
        }
        let receipt = dir.join("receipts.jsonl");
        let result = run([
            OsStr::new("convert"),
            OsStr::new("--from"),
            OsStr::new("git"),
            OsStr::new("--source-actors"),
            map.as_os_str(),
            OsStr::new("--source-receipts"),
            receipt.as_os_str(),
            input.as_os_str(),
            alias.as_os_str(),
        ]);
        assert_eq!(result.code, Some(2));
        assert_eq!(read(&map), format!("{A}\topaque\n"));
        assert!(!receipt.exists());
    }
}

#[test]
fn source_receipts_never_overwrite_existing_files() {
    let dir = scratch("source_existing_receipt");
    let map = write(&dir, "actors.tsv", format!("{A}\topaque\n"));
    let receipt = write(&dir, "receipts.jsonl", "existing sentinel\n");
    let output = dir.join("log");
    let result = run_with_stdin(
        [
            OsStr::new("convert"),
            OsStr::new("--from"),
            OsStr::new("git"),
            OsStr::new("--source-actors"),
            map.as_os_str(),
            OsStr::new("--source-receipts"),
            receipt.as_os_str(),
            OsStr::new("-"),
            output.as_os_str(),
        ],
        raw(A, 1700000000, &file("a")).as_bytes(),
    );
    assert_eq!(result.code, Some(1));
    assert_eq!(read(&receipt), "existing sentinel\n");
    assert!(!output.exists());
}

#[test]
fn source_new_destinations_reject_filesystem_case_and_unicode_aliases() {
    for (i, (receipt_name, output_name)) in [
        ("Receipt.LOG", "receipt.log"),
        ("receipts-é.log", "receipts-e\u{0301}.log"),
    ]
    .into_iter()
    .enumerate()
    {
        let dir = scratch(&format!("source_new_alias_{i}"));
        let receipt = write(&dir, receipt_name, "probe");
        let output = dir.join(output_name);
        let aliases = output.exists();
        fs::remove_file(&receipt).unwrap();
        if !aliases {
            eprintln!("filesystem distinguishes alias case {i}, not applicable");
            continue;
        }
        let map = write(&dir, "actors.tsv", format!("{A}\topaque\n"));
        let result = run_with_stdin(
            [
                OsStr::new("convert"),
                OsStr::new("--from"),
                OsStr::new("git"),
                OsStr::new("--source-actors"),
                map.as_os_str(),
                OsStr::new("--source-receipts"),
                receipt.as_os_str(),
                OsStr::new("-"),
                output.as_os_str(),
            ],
            raw(A, 1700000000, &file("a")).as_bytes(),
        );
        assert_eq!(result.code, Some(2), "new alias case {i}: {result:#?}");
        assert!(!receipt.exists());
        assert!(!output.exists());
        assert_eq!(read(&map), format!("{A}\topaque\n"));
    }
}

#[test]
fn source_map_and_fileless_header_caps_are_independent_and_include_merges() {
    let dir = scratch("source_map_cap");
    let map = (0..150001)
        .map(|i| format!("{i:040x}\topaque\n"))
        .collect::<String>();
    let result = convert(&dir, &raw(A, 1700000000, &file("a")), &map);
    assert_eq!(result.code, Some(1));
    assert!(!dir.join("receipts.jsonl").exists());
    let dir = scratch("source_fileless_cap");
    let input = (0..150001)
        .map(|i| format!("user:unused\n1700000000\nsource:{i:040x}\n\n"))
        .collect::<String>();
    let result = convert(&dir, &input, "");
    assert_eq!(
        result.code,
        Some(1),
        "empty actor map must not permit unlimited fileless headers"
    );
    assert!(result.stdout.is_empty());
    assert!(!dir.join("receipts.jsonl").exists());
}

fn pressure_child(
    dir: &Path,
) -> (
    std::process::Child,
    std::thread::JoinHandle<std::io::Result<()>>,
) {
    use std::io::Write;
    let map = write(dir, "actors.tsv", format!("{A}\topaque\n"));
    let receipt = dir.join("receipts.jsonl");
    let mut child = Command::new(env!("CARGO_BIN_EXE_gource-rs"))
        .args(["convert", "--from", "git", "--source-actors"])
        .arg(map)
        .arg("--source-receipts")
        .arg(receipt)
        .args(["-", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let feed = std::thread::spawn(move || {
        input.write_all(raw(A, 1700000000, "").as_bytes())?;
        for i in 0..50000 {
            input.write_all(file(&format!("src/{i}.rs")).as_bytes())?;
        }
        Ok(())
    });
    (child, feed)
}

#[test]
fn source_large_commit_streams_through_slow_consumer_and_exact_receipt() {
    use std::io::Read;
    let dir = scratch("source_backpressure");
    let (mut child, feed) = pressure_child(&dir);
    let mut output = child.stdout.take().unwrap();
    let mut bytes = Vec::new();
    let mut piece = [0; 1024];
    loop {
        let n = output.read(&mut piece).unwrap();
        if n == 0 {
            break;
        }
        bytes.extend_from_slice(&piece[..n]);
        std::thread::sleep(std::time::Duration::from_micros(50));
    }
    feed.join().unwrap().unwrap();
    let result = child.wait_with_output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(bytes).unwrap();
    assert_eq!(text.lines().count(), 50000);
    assert!(text.starts_with("1700000000|opaque|A|/src/0.rs\n"));
    assert!(text.ends_with("1700000000|opaque|A|/src/49999.rs\n"));
    let rows = receipts(&dir);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].num("start"), 0.0);
    assert_eq!(rows[0].num("end"), 50000.0);
}

#[test]
fn source_cancel_while_stdout_is_backpressured_cannot_report_success() {
    use std::io::Read;
    let dir = scratch("source_backpressure_cancel");
    let (mut child, feed) = pressure_child(&dir);
    let mut output = child.stdout.take().unwrap();
    let mut first = [0; 1];
    output.read_exact(&mut first).unwrap();
    assert_eq!(first[0], b'1');
    assert!(
        child.try_wait().unwrap().is_none(),
        "large stdout cannot finish without being drained"
    );
    child.kill().unwrap();
    let result = child.wait_with_output().unwrap();
    let _ = feed.join().unwrap();
    assert!(!result.status.success());
    // SIGKILL may leave partial staging. The owning pipeline must fence and
    // discard it, never infer success from the presence of a receipt file.
    if dir.join("receipts.jsonl").exists() {
        fs::remove_file(dir.join("receipts.jsonl")).unwrap();
    }
}

fn git(dir: &Path, args: &[&str], time: i64, author: &str, email: &str) -> String {
    let result = Command::new("git")
        .current_dir(dir)
        .args(["-c", "commit.gpgsign=false", "-c", "core.quotepath=off"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", author)
        .env("GIT_AUTHOR_EMAIL", email)
        .env("GIT_COMMITTER_NAME", "Different Committer")
        .env("GIT_COMMITTER_EMAIL", SECRET)
        .env("GIT_AUTHOR_DATE", format!("{} +0000", time - 100))
        .env("GIT_COMMITTER_DATE", format!("{time} +0000"))
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}

#[test]
fn actual_git_root_merge_rename_clocks_and_pinned_mailmap_keep_f1_and_sha_spans() {
    let dir = scratch("source_actual_git");
    git(
        &dir,
        &["init", "-q", "--initial-branch=main"],
        1700000000,
        "Human",
        SECRET,
    );
    write(&dir, "old.rs", "root\n");
    git(&dir, &["add", "old.rs"], 1700000000, "Human", SECRET);
    git(
        &dir,
        &["commit", "-q", "-m", "root"],
        1700000002,
        "Cursor Agent",
        "cursoragent@cursor.com",
    );
    git(
        &dir,
        &["checkout", "-q", "-b", "side"],
        1700000000,
        "Human",
        SECRET,
    );
    write(&dir, "side.rs", "branch\n");
    git(&dir, &["add", "side.rs"], 1700000000, "Human", SECRET);
    git(
        &dir,
        &[
            "commit",
            "-q",
            "-m",
            "branch\n\nCo-authored-by: Claude <noreply@anthropic.com>",
        ],
        1700000000,
        "Human",
        SECRET,
    );
    git(
        &dir,
        &["checkout", "-q", "main"],
        1700000000,
        "Human",
        SECRET,
    );
    fs::rename(dir.join("old.rs"), dir.join("renamed.rs")).unwrap();
    write(
        &dir,
        ".mailmap",
        "Mapped Human <mapped@example.test> Cursor Agent <cursoragent@cursor.com>\n",
    );
    git(&dir, &["add", "-A"], 1700000000, "Human", SECRET);
    git(
        &dir,
        &["commit", "-q", "-m", "rename\n\nMade-with: Cursor"],
        1700000001,
        "Human",
        SECRET,
    );
    git(
        &dir,
        &["merge", "-q", "--no-ff", "side", "-m", "merge"],
        1700000003,
        "Human",
        SECRET,
    );
    git(
        &dir,
        &["commit", "-q", "--allow-empty", "-m", "empty"],
        1700000004,
        "Human",
        SECRET,
    );
    let map_oid = git(
        &dir,
        &["rev-parse", "HEAD:.mailmap"],
        1700000000,
        "Human",
        SECRET,
    )
    .trim()
    .to_owned();
    let pretty = "--pretty=format:user:%aN%n%ct%nm1:%an%x1f%ae%x1f%cn%x1f%ce%x1f%(trailers:key=Co-authored-by,key=Made-with,unfold,separator=%x1f)%x1e%s%nsource:%H";
    let input = git(
        &dir,
        &[
            "-c",
            &format!("mailmap.blob={map_oid}"),
            "log",
            "--reverse",
            "--raw",
            "--no-renames",
            "--encoding=UTF-8",
            "--no-show-signature",
            pretty,
            "HEAD",
        ],
        1700000000,
        "Human",
        SECRET,
    );
    let shas = input
        .lines()
        .filter_map(|line| line.strip_prefix("source:"))
        .collect::<Vec<_>>();
    let actors = shas
        .iter()
        .enumerate()
        .map(|(i, sha)| format!("{sha}\topaque_{i}\n"))
        .collect::<String>();
    let result = convert(&dir, &input, &actors).ok();
    assert_private(&result);
    assert!(!read(&dir.join("receipts.jsonl")).contains(SECRET));
    assert!(result.stdout.contains("1700000002|"));
    assert!(
        result.stdout.contains("1700000000|"),
        "playback must use ct, not author time"
    );
    assert!(!result.stdout.contains("16999999"));
    assert!(result.stdout.contains("|D|/old.rs\n"));
    assert!(result.stdout.contains("|A|/renamed.rs\n"));
    assert!(
        result.stdout.contains("ai=co:cursor"),
        "pinned mailmap must demote raw agent verdict before actor replacement"
    );
    assert!(result.stdout.contains("ai=co:claude"));
    let lines = result.stdout.lines().collect::<Vec<_>>();
    let rows = receipts(&dir);
    assert_eq!(
        rows.len(),
        3,
        "root, branch, rename only, no merge/empty receipts"
    );
    let mut next = 0;
    for row in rows {
        let start = row.num("start") as usize;
        let end = row.num("end") as usize;
        assert_eq!(start, next);
        assert!(end > start);
        for line in &lines[start..end] {
            assert_eq!(line.split('|').nth(1).unwrap(), row.str("label"));
        }
        next = end;
    }
    assert_eq!(next, lines.len());
}
