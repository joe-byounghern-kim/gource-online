// SPDX-License-Identifier: GPL-3.0-or-later

//! `GitRawParser` against `formats/git.cpp` semantics.

use gource_core::events::ChangeKind;
use gource_sources::{CommitSink, GitRawParser, ParsedCommit};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Commit {
    timestamp: i64,
    username: String,
    files: Vec<(ChangeKind, String)>,
    byte_offset: u64,
}

#[derive(Default)]
struct Collect(Vec<Commit>);

impl CommitSink for Collect {
    fn commit(&mut self, c: &ParsedCommit<'_>) {
        assert!(c.files.iter().all(|f| f.colour.is_none()));
        self.0.push(Commit {
            timestamp: c.timestamp,
            username: c.username.to_string(),
            files: c
                .files
                .iter()
                .map(|f| (f.kind, f.path.to_string()))
                .collect(),
            byte_offset: c.byte_offset,
        });
    }
}

fn parse_chunked(input: &[u8], chunk: usize) -> (Vec<Commit>, GitRawParser) {
    let mut parser = GitRawParser::new();
    let mut sink = Collect::default();
    for part in input.chunks(chunk.max(1)) {
        parser.push(part, &mut sink);
    }
    parser.finish(&mut sink);
    (sink.0, parser)
}

fn parse(input: &[u8]) -> Vec<Commit> {
    parse_chunked(input, input.len().max(1)).0
}

use ChangeKind::{Add as A, Delete as D, Modify as M};

fn c(timestamp: i64, user: &str, files: &[(ChangeKind, &str)], byte_offset: u64) -> Commit {
    Commit {
        timestamp,
        username: user.into(),
        files: files.iter().map(|&(k, p)| (k, p.to_string())).collect(),
        byte_offset,
    }
}

/// Real `git log` output (git 2.43) for a repository with a unicode path, a
/// path with a tab, a merge and a delete. No trailing newline, as git emits it.
const REAL: &[u8] = b"user:A B\n\
1790860357\n\
:000000 100644 0000000 d00491f A\ta.txt\n\
:000000 100644 0000000 587be6b A\t\"sub/\\303\\274 \\303\\261.txt\"\n\
:000000 100644 0000000 718f4d2 A\t\"tab\\tq.txt\"\n\
\n\
user:A B\n\
1790860358\n\
:100644 100644 d00491f 1191247 M\ta.txt\n\
\n\
user:C D\n\
1790860359\n\
:000000 100644 0000000 00750ed A\tb\n\
\n\
user:Merger\n\
1790860360\n\
user:A B\n\
1790860361\n\
:100644 000000 00750ed 0000000 D\tb";

fn offset_of(input: &[u8], needle: &str, nth: usize) -> u64 {
    let n = needle.as_bytes();
    input
        .windows(n.len())
        .enumerate()
        .filter(|(_, w)| *w == n)
        .nth(nth)
        .map(|(i, _)| i as u64)
        .unwrap()
}

fn real_expected() -> Vec<Commit> {
    vec![
        c(
            1790860357,
            "A B",
            &[(A, "/a.txt"), (A, "/sub/ü ñ.txt"), (A, "/tab\tq.txt")],
            0,
        ),
        c(
            1790860358,
            "A B",
            &[(M, "/a.txt")],
            offset_of(REAL, "user:", 1),
        ),
        c(1790860359, "C D", &[(A, "/b")], offset_of(REAL, "user:", 2)),
        // the merge header is replaced by the following one
        c(1790860361, "A B", &[(D, "/b")], offset_of(REAL, "user:", 4)),
    ]
}

#[test]
fn real_git_output() {
    let (commits, parser) = parse_chunked(REAL, REAL.len());
    assert_eq!(commits, real_expected());
    let stats = parser.stats();
    assert_eq!(stats.commits, 4);
    assert_eq!(stats.files, 6);
    assert_eq!(stats.bytes, REAL.len() as u64);
    assert_eq!(stats.skipped_lines, 0);
    assert_eq!(parser.headers_seen(), 5);
}

#[test]
fn every_chunking_is_identical() {
    let expected = real_expected();
    for chunk in 1..=REAL.len() {
        let (commits, parser) = parse_chunked(REAL, chunk);
        assert_eq!(commits, expected, "chunk size {chunk}");
        assert_eq!(parser.stats().lines, 19, "chunk size {chunk}");
    }
}

#[test]
fn trailing_newline_and_crlf() {
    let mut with_nl = REAL.to_vec();
    with_nl.extend_from_slice(b"\n\n");
    assert_eq!(parse(&with_nl), real_expected());

    let crlf: Vec<u8> = REAL
        .split(|&b| b == b'\n')
        .flat_map(|l| l.iter().copied().chain(*b"\r\n"))
        .collect();
    let commits = parse(&crlf);
    assert_eq!(commits.len(), 4);
    assert_eq!(commits[0].files, real_expected()[0].files);
    assert_eq!(commits[3].username, "A B");
}

#[test]
fn status_letters() {
    let input = b"user:x\n5\n\
:100644 100644 a b M\tm\n\
:100644 120000 a b T\tt\n\
:000000 000000 a b U\tu\n\
:000000 000000 a b X\tx\n\
:000000 100644 a b A\ta\n\
:100644 000000 a b D\td\n";
    assert_eq!(
        parse(input),
        vec![c(
            5,
            "x",
            &[
                (M, "/m"),
                (M, "/t"),
                (M, "/u"),
                (M, "/x"),
                (A, "/a"),
                (D, "/d")
            ],
            0
        )]
    );
}

#[test]
fn zero_timestamp_aborts_commit() {
    // atol("garbage") == 0: the commit is abandoned and its raw lines are
    // skipped (username empty) until the next header.
    let input = b"user:bad\ngarbage\n:0 0 a b A\tlost1\n:0 0 a b A\tlost2\n\nuser:good\n7\n:0 0 a b A\tkept\n";
    let (commits, parser) = parse_chunked(input, input.len());
    assert_eq!(
        commits,
        vec![c(
            7,
            "good",
            &[(A, "/kept")],
            offset_of(input, "user:good", 0)
        )]
    );
    assert_eq!(parser.stats().skipped_lines, 3);
    assert_eq!(parser.headers_seen(), 2);
}

#[test]
fn blank_line_after_header_is_the_timestamp() {
    // getNextLine after "user:" consumes the next line whatever it is.
    let input = b"user:a\n\n:0 0 a b A\tx\n\nuser:b\n9\n:0 0 a b A\ty\n";
    assert_eq!(parse(input), vec![c(9, "b", &[(A, "/y")], 22)]);
}

#[test]
fn files_before_user_and_empty_user_are_skipped() {
    let input = b":0 0 a b A\torphan\n\nuser:\n3\n:0 0 a b A\tanon\n\nuser:z\n4\n:0 0 a b A\tok\n";
    let (commits, parser) = parse_chunked(input, 7);
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].files, vec![(A, "/ok".to_string())]);
    assert_eq!(parser.stats().skipped_lines, 2);
}

#[test]
fn malformed_raw_lines_are_skipped() {
    let input = b"user:u\n10\nno tab here\n\tstarts with tab\nends with tab\t\n:0 0 a b A\t\"\"\n:0 0 a b A\t\"\n:0 0 a b A\tgood\n";
    let (commits, parser) = parse_chunked(input, input.len());
    assert_eq!(commits, vec![c(10, "u", &[(A, "/good")], 0)]);
    assert_eq!(parser.stats().skipped_lines, 5);
}

#[test]
fn quoted_paths_are_decoded() {
    let input = b"user:q\n1\n\
:0 0 a b A\t\"with \\\"quote\\\"\"\n\
:0 0 a b A\t\"back\\\\slash\"\n\
:0 0 a b A\t\"new\\nline\"\n\
:0 0 a b A\t\"\\344\\270\\255\\346\\226\\207/\\360\\237\\230\\200\"\n\
:0 0 a b A\tplain \"inner\" quotes\n";
    let commits = parse(input);
    let paths: Vec<&str> = commits[0].files.iter().map(|f| f.1.as_str()).collect();
    assert_eq!(
        paths,
        [
            "/with \"quote\"",
            "/back\\slash",
            "/new\nline",
            "/中文/😀",
            "/plain \"inner\" quotes"
        ]
    );
}

#[test]
fn invalid_utf8_is_replaced() {
    let input = b"user:Jos\xe9\n1\n:0 0 a b A\tbad\xff/name\xc3\n:0 0 a b A\t\"oct\\377al\"\n";
    let commits = parse(input);
    assert_eq!(commits[0].username, "Jos?");
    assert_eq!(commits[0].files[0].1, "/bad?/name?");
    assert_eq!(commits[0].files[1].1, "/oct?al");
}

#[test]
fn utf8_split_across_chunks() {
    let input = "user:Ünïcødé\n1\n:0 0 a b A\tdir/日本語.txt\n".as_bytes();
    for chunk in 1..=input.len() {
        let (commits, _) = parse_chunked(input, chunk);
        assert_eq!(commits, vec![c(1, "Ünïcødé", &[(A, "/dir/日本語.txt")], 0)]);
    }
}

#[test]
fn rooted_paths_keep_one_slash() {
    let commits = parse(b"user:r\n1\n:0 0 a b A\t/already/rooted\n");
    assert_eq!(commits[0].files[0].1, "/already/rooted");
}

#[test]
fn header_without_timestamp_at_eof_is_dropped() {
    let (commits, parser) = parse_chunked(b"user:a\n1\n:0 0 a b A\tx\n\nuser:b", 3);
    assert_eq!(commits.len(), 1);
    assert_eq!(parser.headers_seen(), 2);
}

#[test]
fn merges_only_and_empty_input() {
    assert!(parse(b"").is_empty());
    assert!(parse(b"\n\n\n").is_empty());
    let (commits, parser) = parse_chunked(b"user:m\n1\nuser:n\n2\n", 64);
    assert!(commits.is_empty());
    assert_eq!(parser.headers_seen(), 2);
}

#[test]
fn negative_timestamps() {
    assert_eq!(
        parse(b"user:old\n-86400\n:0 0 a b M\tf\n")[0].timestamp,
        -86400
    );
}

#[test]
fn files_survive_a_second_header_in_one_block() {
    // parseCommit keeps accumulating files across `user:` lines inside one
    // block; the last header names the commit.
    let input = b"user:a\n1\n:0 0 a b A\tx\nuser:b\n2\n:0 0 a b A\ty\n";
    assert_eq!(parse(input), vec![c(2, "b", &[(A, "/x"), (A, "/y")], 0)]);
}

#[test]
fn large_input_many_commits() {
    let mut input = Vec::new();
    for i in 0..5000u32 {
        input.extend_from_slice(format!("user:user{}\n{}\n", i % 7, 1000 + i).as_bytes());
        if i % 10 == 9 {
            continue; // merge
        }
        for j in 0..3 {
            input.extend_from_slice(
                format!(":100644 100644 a b M\tdir{}/file{j}.rs\n", i % 13).as_bytes(),
            );
        }
        input.push(b'\n');
    }
    let (commits, parser) = parse_chunked(&input, 4093);
    assert_eq!(commits.len(), 4500);
    assert_eq!(parser.headers_seen(), 5000);
    assert_eq!(parser.stats().files, 13500);
    assert!(
        commits
            .windows(2)
            .all(|w| w[0].byte_offset < w[1].byte_offset)
    );
    assert_eq!(&input[commits[1].byte_offset as usize..][..5], b"user:");
}

fn parse_limited(input: &[u8], chunk: usize, files: usize, bytes: usize) -> (Vec<Commit>, bool) {
    let mut parser = GitRawParser::with_commit_limits(files, bytes);
    let mut sink = Collect::default();
    for part in input.chunks(chunk.max(1)) {
        parser.push(part, &mut sink);
    }
    parser.finish(&mut sink);
    (sink.0, parser.limit_exceeded())
}

#[test]
fn commit_limits_not_hit_change_nothing() {
    for chunk in [1, 7, REAL.len()] {
        let (commits, exceeded) = parse_limited(REAL, chunk, 3, 200);
        assert!(!exceeded);
        assert_eq!(commits, real_expected(), "chunk {chunk}");
    }
    assert!(!GitRawParser::new().limit_exceeded());
}

#[test]
fn file_limit_stops_the_parser() {
    // The first commit of REAL has three files.
    for chunk in [1, 5, REAL.len()] {
        let (commits, exceeded) = parse_limited(REAL, chunk, 2, usize::MAX);
        assert!(exceeded, "chunk {chunk}");
        assert!(commits.is_empty(), "chunk {chunk}: {commits:?}");
    }
    // Commits before the oversized one are kept; nothing after it is read.
    let mut input = b"user:X\n1\n:0 0 0 0 A\tok\n\n".to_vec();
    input.extend_from_slice(b"user:Y\n2\n");
    for i in 0..10 {
        input.extend_from_slice(format!(":0 0 0 0 A\tf{i}\n").as_bytes());
    }
    input.extend_from_slice(b"\nuser:Z\n3\n:0 0 0 0 A\tlater\n");
    let (commits, exceeded) = parse_limited(&input, 3, 9, usize::MAX);
    assert!(exceeded);
    assert_eq!(commits, [c(1, "X", &[(A, "/ok")], 0)]);
    let (commits, exceeded) = parse_limited(&input, 3, 10, usize::MAX);
    assert!(!exceeded);
    assert_eq!(commits.len(), 3);
}

#[test]
fn byte_limit_bounds_paths_and_partial_lines() {
    // A commit's paths over the limit.
    let input = b"user:X\n1\n:0 0 0 0 A\taaaaaaaaaa\n:0 0 0 0 A\tbbbbbbbbbb\n";
    let (_, exceeded) = parse_limited(input, input.len(), usize::MAX, 15);
    assert!(exceeded);
    let (commits, exceeded) = parse_limited(input, input.len(), usize::MAX, 22);
    assert!(!exceeded);
    assert_eq!(commits[0].files.len(), 2);

    // One endless line split over many chunks never grows past the limit.
    let mut parser = GitRawParser::with_commit_limits(usize::MAX, 1000);
    let mut sink = Collect::default();
    parser.push(b"user:X\n1\n:0 0 0 0 A\t", &mut sink);
    for _ in 0..1000 {
        parser.push(&[b'x'; 100], &mut sink);
    }
    assert!(parser.limit_exceeded());
    parser.finish(&mut sink);
    assert!(sink.0.is_empty());
}
