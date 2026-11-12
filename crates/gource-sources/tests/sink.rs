// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! `EventLogSink`: parsed commits appended straight into an `EventLog`.

mod common;

use common::*;
use gource_core::events::{ChangeEvent, ChangeKind, EventLog, StrId};
use gource_sources::{CommitSink, CustomLogParser, EventLogSink, ParsedCommit, ParsedFile};

/// Parse into an `EventLog` through `EventLogSink`, pushing `chunk` bytes at
/// a time.
fn direct(bytes: &[u8], chunk: usize) -> EventLog {
    let mut log = EventLog::new();
    let mut parser = CustomLogParser::new();
    {
        let mut sink = EventLogSink::new(&mut log);
        for piece in bytes.chunks(chunk.max(1)) {
            parser.push(piece, &mut sink);
        }
        parser.finish(&mut sink);
    }
    log
}

#[test]
fn the_direct_sink_ignores_chunking_too() {
    for (_name, bytes) in corpus() {
        let whole = direct(&bytes, bytes.len().max(1));
        for chunk in [1, 5, 64] {
            let chunked = direct(&bytes, chunk);
            assert_logs_equal(&whole, &chunked);
        }
    }
}

#[test]
fn the_log_holds_what_the_parser_emitted() {
    let bytes = read_test_file("fixtures/custom-log/duplicates.log");
    let log = direct(&bytes, 1000);
    assert_eq!(log.commit_count(), 2);
    let c0 = log.commit(0);
    assert_eq!(c0.timestamp, 1_700_000_200);
    assert_eq!(log.str(c0.user), "alice");
    assert_eq!(c0.changes.len(), 3);
    assert_eq!(log.str(c0.changes[0].path), "/src/a.rs");
    assert_eq!(c0.changes[0].kind, ChangeKind::Add);
    assert_eq!(c0.byte_offset, 0);
    // identical paths share one string
    assert_eq!(c0.changes[0].path, c0.changes[2].path);
    assert_eq!(log.commit(1).changes[0].kind, ChangeKind::Modify);
    // "alice", "/src/a.rs", "bob"
    assert_eq!(log.string_count(), 3);
}

#[test]
fn string_ids_come_in_first_seen_order_user_before_paths() {
    let log = direct(b"1|zed|A|/b\n1|zed|A|/a\n2|amy|A|/b\n", 100);
    let strings: Vec<&str> = (0..log.string_count())
        .map(|i| log.str(StrId(i as u32)))
        .collect();
    assert_eq!(strings, ["zed", "/b", "/a", "amy"]);
}

#[test]
fn large_commits_take_the_heap_path_and_keep_their_order() {
    // more files than the sink converts on the stack
    let mut text = String::new();
    for i in 0..100 {
        text.push_str(&format!(
            "7|big|{}|/dir/file{i}.rs|{:02x}0000\n",
            ["A", "M", "D"][i % 3],
            i
        ));
    }
    let log = direct(text.as_bytes(), 4096);
    assert_eq!(log.commit_count(), 1);
    let commit = log.commit(0);
    assert_eq!(commit.changes.len(), 100);
    for (i, change) in commit.changes.iter().enumerate() {
        assert_eq!(log.str(change.path), format!("/dir/file{i}.rs"));
        assert_eq!(change.colour, Some([i as u8, 0, 0]));
        assert_eq!(
            change.kind,
            [ChangeKind::Add, ChangeKind::Modify, ChangeKind::Delete][i % 3]
        );
    }
    // 100 paths and one user
    assert_eq!(log.string_count(), 101);
}

#[test]
fn commits_without_files_leave_no_trace() {
    let mut log = EventLog::new();
    EventLogSink::new(&mut log).commit(&ParsedCommit {
        timestamp: 5,
        username: "ghost",
        files: &[],
        attrs: &[],
        byte_offset: 0,
    });
    assert_eq!(log.commit_count(), 0);
    assert_eq!(log.string_count(), 0);
}

#[test]
fn the_sink_can_be_built_with_a_struct_literal_too() {
    // `log` is a public field, so the sink works in a struct literal
    let mut log = EventLog::new();
    let mut sink = EventLogSink { log: &mut log };
    sink.commit(&ParsedCommit {
        timestamp: 1,
        username: "u",
        files: &[ParsedFile {
            path: "/f",
            kind: ChangeKind::Add,
            colour: Some([1, 2, 3]),
            event: ChangeEvent::Commit,
        }],
        attrs: &[],
        byte_offset: 9,
    });
    assert_eq!(log.commit(0).byte_offset, 9);
    assert_eq!(log.commit(0).changes[0].colour, Some([1, 2, 3]));
}

#[test]
fn every_repository_log_loads() {
    for (name, bytes) in corpus() {
        let log = direct(&bytes, 1 << 16);
        let (commits, stats) = parse_whole(&bytes);
        assert_eq!(log.commit_count(), commits.len(), "{name}");
        assert_eq!(log.change_count() as u64, stats.files, "{name}");
        for (i, c) in commits.iter().enumerate() {
            let got = log.commit(i);
            assert_eq!(got.timestamp, c.timestamp, "{name} commit {i}");
            assert_eq!(log.str(got.user), c.username, "{name} commit {i}");
            assert_eq!(got.byte_offset, c.byte_offset, "{name} commit {i}");
            assert_eq!(got.changes.len(), c.files.len(), "{name} commit {i}");
            for (change, (kind, path, colour)) in got.changes.iter().zip(&c.files) {
                assert_eq!(log.str(change.path), path, "{name} commit {i}");
                assert_eq!(
                    (change.kind, change.colour),
                    (*kind, *colour),
                    "{name} commit {i}"
                );
            }
        }
    }
}
