// SPDX-License-Identifier: GPL-3.0-or-later

use gource_sources::{CommitSink, GitRawParser, ParsedCommit};

const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const C: &str = "cccccccccccccccccccccccccccccccccccccccc";

#[derive(Default, Debug, PartialEq)]
struct SourceSink(Vec<(String, String, i64, Vec<String>)>);
impl CommitSink for SourceSink {
    fn commit(&mut self, _: &ParsedCommit<'_>) {
        panic!("source mode must retain SHA");
    }
    fn source_commit(&mut self, commit: &ParsedCommit<'_>, sha: &str) {
        self.0.push((
            sha.to_owned(),
            commit.username.to_owned(),
            commit.timestamp,
            commit.files.iter().map(|f| f.path.to_owned()).collect(),
        ));
    }
}

fn parser(cap: usize) -> GitRawParser {
    let mut parser = GitRawParser::with_commit_limits(10, 1024);
    parser.require_source_ids(cap);
    parser
}

#[test]
fn source_identity_survives_every_chunk_boundary_and_dropped_merge_header() {
    let input = format!(
        "user:merge\n1700000002\nsource:{A}\nuser:author\n1700000000\nsource:{B}\n:000000 100644 0000000 d00491f A\tunicode/ü.txt\n\nuser:last\n-1\nsource:{C}\n:100644 000000 d00491f 0000000 D\told"
    );
    let expected = SourceSink(vec![
        (
            B.into(),
            "author".into(),
            1700000000,
            vec!["/unicode/ü.txt".into()],
        ),
        (C.into(), "last".into(), -1, vec!["/old".into()]),
    ]);
    for split in 0..=input.len() {
        let mut parser = parser(3);
        let mut sink = SourceSink::default();
        parser.push(&input.as_bytes()[..split], &mut sink);
        parser.push(&input.as_bytes()[split..], &mut sink);
        parser.finish(&mut sink);
        assert!(!parser.source_failed(), "split {split}");
        assert_eq!(sink, expected, "split {split}");
    }
}

#[test]
fn source_seen_bound_includes_fileless_headers_and_is_sticky() {
    let mut parser = parser(1);
    let mut sink = SourceSink::default();
    parser.push(
        format!("user:merge\n1\nsource:{A}\n\nuser:second\n2\nsource:{B}\n").as_bytes(),
        &mut sink,
    );
    assert!(parser.source_failed());
    parser.push(
        format!("user:later\n3\nsource:{C}\n:000000 100644 0000000 d00491f A\tx\n").as_bytes(),
        &mut sink,
    );
    parser.finish(&mut sink);
    assert!(parser.source_failed());
    assert!(sink.0.is_empty());
}

#[test]
fn source_header_after_files_never_reassigns_old_spans() {
    let mut parser = parser(3);
    let mut sink = SourceSink::default();
    parser.push(format!("user:first\n1\nsource:{A}\n:000000 100644 0000000 d00491f A\ta\nuser:second\n2\nsource:{B}\n").as_bytes(), &mut sink);
    parser.finish(&mut sink);
    assert!(parser.source_failed());
    assert!(sink.0.is_empty());
}

#[test]
fn source_complete_fileless_input_succeeds_but_every_truncated_header_fails() {
    for input in [
        "user:first".to_owned(),
        "user:first\n1".to_owned(),
        "user:first\n1\nm1:bad".to_owned(),
    ] {
        let mut parser = parser(1);
        let mut sink = SourceSink::default();
        parser.push(input.as_bytes(), &mut sink);
        parser.finish(&mut sink);
        assert!(parser.source_failed(), "truncated header must fail");
        assert!(sink.0.is_empty());
    }
    let mut parser = parser(1);
    let mut sink = SourceSink::default();
    parser.push(format!("user:first\n1\nsource:{A}").as_bytes(), &mut sink);
    parser.finish(&mut sink);
    assert!(!parser.source_failed());
    assert!(sink.0.is_empty());
}
