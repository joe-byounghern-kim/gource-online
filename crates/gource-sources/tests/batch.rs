// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! `EventLogSink` against `BatchEncoder` + `apply_batch`, and the batch
//! codec's format, ordering and error handling.

mod common;

use common::*;
use gource_core::events::{ChangeEvent, ChangeKind, EventLog, StrId};
use gource_sources::{
    BatchEncoder, BatchSummary, CommitSink, CustomLogParser, EventLogSink, ParsedCommit,
    ParsedFile, apply_batch,
};

// --- helpers ----------------------------------------------------------------

/// Parse into an `EventLog` through `EventLogSink`.
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

/// Parse through a `BatchEncoder`, taking a batch after every push (as the
/// worker does) and, in addition, whenever `every` commits are pending.
fn encode(bytes: &[u8], chunk: usize, every: usize) -> Vec<Vec<u8>> {
    struct Batcher {
        encoder: BatchEncoder,
        every: usize,
        batches: Vec<Vec<u8>>,
    }
    impl CommitSink for Batcher {
        fn commit(&mut self, c: &ParsedCommit<'_>) {
            self.encoder.commit(c);
            if self.encoder.pending_commits() >= self.every {
                self.batches.push(self.encoder.take_batch());
            }
        }
    }

    let mut sink = Batcher {
        encoder: BatchEncoder::new(),
        every,
        batches: Vec::new(),
    };
    let mut parser = CustomLogParser::new();
    for piece in bytes.chunks(chunk.max(1)) {
        parser.push(piece, &mut sink);
        let batch = sink.encoder.take_batch();
        if !batch.is_empty() {
            sink.batches.push(batch);
        }
    }
    parser.finish(&mut sink);
    let last = sink.encoder.take_batch();
    if !last.is_empty() {
        sink.batches.push(last);
    }
    sink.batches.retain(|b| !b.is_empty());
    sink.batches
}

fn apply_all(batches: &[Vec<u8>]) -> EventLog {
    let mut log = EventLog::new();
    for (i, b) in batches.iter().enumerate() {
        apply_batch(&mut log, b).unwrap_or_else(|e| panic!("batch {i}: {e}"));
    }
    log
}

fn varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn zigzag(out: &mut Vec<u8>, v: i64) {
    varint(out, ((v << 1) ^ (v >> 63)) as u64);
}

/// A batch assembled by hand from the format description.
struct Hand {
    first_string: u32,
    strings: Vec<&'static str>,
    first_set: u32,
    set_count: u32,
    sets: Vec<u8>,
    first_commit: u32,
    commit_count: u32,
    change_count: u32,
    commits: Vec<u8>,
}

impl Hand {
    fn bytes(&self) -> Vec<u8> {
        let mut out = b"GEB\x02".to_vec();
        for n in [
            self.first_string,
            self.first_set,
            self.first_commit,
            self.strings.len() as u32,
            self.set_count,
            self.commit_count,
            self.change_count,
        ] {
            out.extend_from_slice(&n.to_le_bytes());
        }
        for s in &self.strings {
            varint(&mut out, s.len() as u64);
            out.extend_from_slice(s.as_bytes());
        }
        out.extend_from_slice(&self.sets);
        out.extend_from_slice(&self.commits);
        out
    }
}

/// One commit, timestamp 100, byte offset 40, user string 0, one change
/// (modify) on string 1 with colour 1,2,3.
fn one_commit_batch() -> Hand {
    let mut commits = Vec::new();
    zigzag(&mut commits, 100);
    varint(&mut commits, 0);
    zigzag(&mut commits, 40);
    varint(&mut commits, 1 << 1); // one change, no attribution set
    varint(&mut commits, 1);
    commits.extend_from_slice(&[1 | 4, 1, 2, 3]);
    Hand {
        first_string: 0,
        strings: vec!["alice", "/a.rs"],
        first_set: 1,
        set_count: 0,
        sets: Vec::new(),
        first_commit: 0,
        commit_count: 1,
        change_count: 1,
        commits,
    }
}

// --- sink and codec agree ----------------------------------------------------

#[test]
fn batches_rebuild_exactly_what_the_direct_sink_builds() {
    for (name, bytes) in corpus() {
        let expected = direct(&bytes, bytes.len().max(1));
        for (chunk, every) in [
            (usize::MAX, usize::MAX),
            (4096, 1000),
            (64, 3),
            (7, 1),
            (1, 5),
        ] {
            let chunk = chunk.min(bytes.len().max(1));
            let batches = encode(&bytes, chunk, every);
            let rebuilt = apply_all(&batches);
            assert_logs_equal(&expected, &rebuilt);
            if expected.commit_count() > 0 {
                assert!(!batches.is_empty(), "{name}");
            }
        }
    }
}

#[test]
fn string_ids_come_in_first_seen_order_user_before_paths() {
    let bytes = b"1|zed|A|/b\n1|zed|A|/a\n2|amy|A|/b\n";
    let log = direct(bytes, 100);
    let strings: Vec<&str> = (0..log.string_count())
        .map(|i| log.str(StrId(i as u32)))
        .collect();
    assert_eq!(strings, ["zed", "/b", "/a", "amy"]);

    let rebuilt = apply_all(&encode(bytes, 100, usize::MAX));
    assert_logs_equal(&log, &rebuilt);
}

#[test]
fn colours_and_kinds_survive() {
    let bytes = b"1|a|A|/x|ff8800\n1|a|M|/y|#000001\n1|a|D|/z\n";
    let log = direct(bytes, 100);
    let rebuilt = apply_all(&encode(bytes, 100, usize::MAX));
    assert_logs_equal(&log, &rebuilt);
    let ch = rebuilt.commit(0).changes;
    assert_eq!(ch[0].colour, Some([0xff, 0x88, 0x00]));
    assert_eq!(ch[1].colour, Some([0, 0, 1]));
    assert_eq!(ch[2].colour, None);
    assert_eq!(
        [ch[0].kind, ch[1].kind, ch[2].kind],
        [ChangeKind::Add, ChangeKind::Modify, ChangeKind::Delete]
    );
}

#[test]
fn extreme_values_round_trip() {
    let files: Vec<ParsedFile<'_>> = (0..70)
        .map(|_| ParsedFile {
            path: "/p",
            kind: ChangeKind::Modify,
            colour: Some([255, 0, 128]),
            event: ChangeEvent::Commit,
        })
        .collect();
    let commits = [
        (i64::MIN, "", u64::MAX, &files[..1]),
        (i64::MAX, "u", 0, &files[..2]),
        (0, "u", 1, &files[..]),
        (-1, "\u{1f332}", u64::MAX / 2, &files[..3]),
        (i64::MAX, "u", u64::MAX, &files[..1]),
        (i64::MIN, "u", 0, &files[..1]),
    ];

    let mut log_a = EventLog::new();
    let mut log_b = EventLog::new();
    let mut encoder = BatchEncoder::new();
    {
        let mut sink = EventLogSink::new(&mut log_a);
        for (timestamp, username, byte_offset, files) in commits {
            let commit = ParsedCommit {
                timestamp,
                username,
                files,
                attrs: &[],
                byte_offset,
            };
            sink.commit(&commit);
            encoder.commit(&commit);
        }
    }
    let batch = encoder.take_batch();
    let summary = apply_batch(&mut log_b, &batch).unwrap();
    assert_eq!(summary.commits, 6);
    assert_eq!(summary.changes, 1 + 2 + 70 + 3 + 1 + 1);
    assert_logs_equal(&log_a, &log_b);
    assert_eq!(log_b.commit(0).timestamp, i64::MIN);
    assert_eq!(log_b.commit(0).byte_offset, u64::MAX);
    assert_eq!(log_b.commit(2).changes.len(), 70);
    assert_eq!(log_b.str(log_b.commit(0).user), "");
}

#[test]
fn commits_without_files_are_ignored_by_both_sinks() {
    let empty = ParsedCommit {
        timestamp: 5,
        username: "ghost",
        files: &[],
        attrs: &[],
        byte_offset: 0,
    };
    let mut log = EventLog::new();
    EventLogSink::new(&mut log).commit(&empty);
    assert_eq!(log.commit_count(), 0);
    assert_eq!(log.string_count(), 0);

    let mut encoder = BatchEncoder::new();
    encoder.commit(&empty);
    assert_eq!(encoder.pending_commits(), 0);
    assert!(encoder.take_batch().is_empty());
    assert_eq!(encoder.string_count(), 0);
}

// --- the format ----------------------------------------------------------------

#[test]
fn encoder_output_matches_the_documented_format_byte_for_byte() {
    let mut encoder = BatchEncoder::new();
    encoder.commit(&ParsedCommit {
        timestamp: 100,
        username: "alice",
        files: &[ParsedFile {
            path: "/a.rs",
            kind: ChangeKind::Modify,
            colour: Some([1, 2, 3]),
            event: ChangeEvent::Commit,
        }],
        attrs: &[],
        byte_offset: 40,
    });
    assert_eq!(encoder.pending_commits(), 1);
    let bytes = encoder.take_batch();
    assert_eq!(bytes, one_commit_batch().bytes());
    assert_eq!(&bytes[..4], b"GEB\x02");
    assert_eq!(encoder.pending_commits(), 0);
    assert!(encoder.take_batch().is_empty());
}

#[test]
fn a_hand_built_batch_applies() {
    let mut log = EventLog::new();
    let summary = apply_batch(&mut log, &one_commit_batch().bytes()).unwrap();
    assert_eq!(
        summary,
        BatchSummary {
            strings: 2,
            sets: 0,
            commits: 1,
            changes: 1
        }
    );
    let c = log.commit(0);
    assert_eq!((c.timestamp, c.byte_offset), (100, 40));
    assert_eq!(log.str(c.user), "alice");
    assert_eq!(log.str(c.changes[0].path), "/a.rs");
    assert_eq!(c.changes[0].kind, ChangeKind::Modify);
    assert_eq!(c.changes[0].colour, Some([1, 2, 3]));
}

#[test]
fn timestamps_and_offsets_are_deltas_that_restart_in_every_batch() {
    let mut encoder = BatchEncoder::new();
    let file = [ParsedFile {
        path: "/f",
        kind: ChangeKind::Add,
        colour: None,
        event: ChangeEvent::Commit,
    }];
    let commit = |timestamp, byte_offset| ParsedCommit {
        timestamp,
        username: "u",
        files: &file,
        attrs: &[],
        byte_offset,
    };
    encoder.commit(&commit(1_700_000_000, 1_000_000));
    encoder.commit(&commit(1_700_000_001, 1_000_050));
    let first = encoder.take_batch();
    encoder.commit(&commit(1_700_000_002, 1_000_100));
    let second = encoder.take_batch();

    // The second batch has no new strings and its deltas start from zero
    // again: the absolute timestamp (5 bytes), user id, offset (3 bytes),
    // change count, path id and flags follow the 32 header bytes.
    assert_eq!(second.len(), 32 + 5 + 1 + 3 + 1 + 1 + 1);
    let mut log = EventLog::new();
    apply_batch(&mut log, &first).unwrap();
    apply_batch(&mut log, &second).unwrap();
    let times: Vec<i64> = log.records().iter().map(|r| r.timestamp).collect();
    assert_eq!(times, [1_700_000_000, 1_700_000_001, 1_700_000_002]);
    let offsets: Vec<u64> = log.records().iter().map(|r| r.byte_offset).collect();
    assert_eq!(offsets, [1_000_000, 1_000_050, 1_000_100]);
}

#[test]
fn only_new_strings_are_sent() {
    let bytes = b"1|alice|A|/x\n2|alice|M|/x\n3|bob|M|/x\n";
    // one commit per batch
    let batches = encode(bytes, 100, 1);
    assert_eq!(batches.len(), 3);
    let mut log = EventLog::new();
    let counts: Vec<u32> = batches
        .iter()
        .map(|b| apply_batch(&mut log, b).unwrap().strings)
        .collect();
    assert_eq!(counts, [2, 0, 1]);
    // header field first_string: 0, 2, 2
    let firsts: Vec<u32> = batches
        .iter()
        .map(|b| u32::from_le_bytes([b[4], b[5], b[6], b[7]]))
        .collect();
    assert_eq!(firsts, [0, 2, 2]);
    // first_set: 1 (no sets yet) in all three
    let firsts: Vec<u32> = batches
        .iter()
        .map(|b| u32::from_le_bytes([b[8], b[9], b[10], b[11]]))
        .collect();
    assert_eq!(firsts, [1, 1, 1]);
    // and first_commit: 0, 1, 2
    let firsts: Vec<u32> = batches
        .iter()
        .map(|b| u32::from_le_bytes([b[12], b[13], b[14], b[15]]))
        .collect();
    assert_eq!(firsts, [0, 1, 2]);
    // the second batch is small: header + one commit
    assert!(batches[1].len() < 32 + 12);
}

#[test]
fn the_encoding_is_compact() {
    let mut rng = Rng(77);
    let bytes = generate_log(&mut rng, 5000);
    let batches = encode(&bytes, 1 << 16, usize::MAX);
    let encoded: usize = batches.iter().map(Vec::len).sum();
    assert!(
        encoded * 2 < bytes.len(),
        "{encoded} bytes encoded from {} bytes of log",
        bytes.len()
    );
}

#[test]
fn pending_bytes_is_the_size_of_the_next_batch() {
    let bytes = read_test_file("fixtures/custom-log/visual-hierarchy-activity.log");
    let mut encoder = BatchEncoder::new();
    assert_eq!(encoder.pending_bytes(), 0);
    let mut parser = CustomLogParser::new();
    parser.push(&bytes, &mut encoder);
    parser.finish(&mut encoder);
    let expected = encoder.pending_bytes();
    assert!(expected > 32);
    assert_eq!(encoder.take_batch().len(), expected);
    assert_eq!(encoder.pending_bytes(), 0);
}

// --- ordering ----------------------------------------------------------------

fn three_batches() -> Vec<Vec<u8>> {
    // distinct users and paths in each, so every batch carries strings
    let bytes = b"1|a|A|/1\n2|b|A|/2\n3|c|A|/3\n";
    let batches = encode(bytes, 1000, 1);
    assert_eq!(batches.len(), 3);
    batches
}

#[test]
fn out_of_order_batches_are_rejected_and_change_nothing() {
    let batches = three_batches();

    // the second batch first
    let mut log = EventLog::new();
    let err = apply_batch(&mut log, &batches[1]).unwrap_err();
    assert!(err.0.contains("out of order"), "{err}");
    assert_logs_equal(&log, &EventLog::new());

    // the third after the first: a gap
    apply_batch(&mut log, &batches[0]).unwrap();
    let before = log.clone();
    let err = apply_batch(&mut log, &batches[2]).unwrap_err();
    assert!(err.0.contains("out of order"), "{err}");
    assert_logs_equal(&log, &before);

    // in order after all
    apply_batch(&mut log, &batches[1]).unwrap();
    apply_batch(&mut log, &batches[2]).unwrap();

    // a repeat
    let before = log.clone();
    assert!(apply_batch(&mut log, &batches[2]).is_err());
    assert!(apply_batch(&mut log, &batches[0]).is_err());
    assert_logs_equal(&log, &before);
}

#[test]
fn a_dropped_batch_without_new_strings_is_still_noticed() {
    // all three batches reuse the strings of the first
    let bytes = b"1|a|A|/1\n2|a|M|/1\n3|a|M|/1\n";
    let batches = encode(bytes, 1000, 1);
    assert_eq!(batches.len(), 3);

    let mut log = EventLog::new();
    apply_batch(&mut log, &batches[0]).unwrap();
    // batches[1] and [2] both have first_string == 2 == log.string_count();
    // only the commit index tells them apart
    let err = apply_batch(&mut log, &batches[2]).unwrap_err();
    assert!(err.0.contains("commit"), "{err}");
    assert_eq!(log.commit_count(), 1);
    apply_batch(&mut log, &batches[1]).unwrap();
    apply_batch(&mut log, &batches[2]).unwrap();
    assert_eq!(log.commit_count(), 3);
}

#[test]
fn batches_apply_on_top_of_nothing_else() {
    // a log that already holds strings does not accept a stream starting at 0
    let batches = three_batches();
    let mut log = EventLog::new();
    log.intern("something else");
    assert!(apply_batch(&mut log, &batches[0]).is_err());
    assert_eq!(log.string_count(), 1);
    assert_eq!(log.commit_count(), 0);
}

#[test]
fn empty_input_is_a_no_op() {
    let mut log = EventLog::new();
    assert_eq!(apply_batch(&mut log, &[]).unwrap(), BatchSummary::default());
    assert_eq!(log.string_count(), 0);
}

// --- malformed input -------------------------------------------------------------

#[test]
fn every_truncation_is_rejected_and_leaves_the_log_alone() {
    for (name, bytes) in corpus().into_iter().filter(|(_, b)| b.len() < 700) {
        let batches = encode(&bytes, 1000, usize::MAX);
        for (n, batch) in batches.iter().enumerate() {
            for cut in 1..batch.len() {
                let mut log = apply_all(&batches[..n]);
                let before = log.clone();
                assert!(
                    apply_batch(&mut log, &batch[..cut]).is_err(),
                    "{name}: batch {n} cut at {cut}"
                );
                assert_logs_equal(&log, &before);
            }
        }
    }
}

#[test]
fn corrupted_batches_never_panic_and_failures_change_nothing() {
    let mut rng = Rng(0xBAD_C0DE);
    for (name, bytes) in corpus().into_iter().filter(|(_, b)| b.len() < 700) {
        let batches = encode(&bytes, 1000, usize::MAX);
        let Some(batch) = batches.first() else {
            continue;
        };
        for _ in 0..300 {
            let mut broken = batch.clone();
            for _ in 0..1 + rng.below(3) {
                let at = rng.below(broken.len());
                broken[at] = match rng.below(4) {
                    0 => 0,
                    1 => 0xff,
                    2 => broken[at] ^ (1 << rng.below(8)),
                    _ => rng.next() as u8,
                };
            }
            let mut log = EventLog::new();
            let before = log.clone();
            match apply_batch(&mut log, &broken) {
                Ok(summary) => {
                    // a corruption that still describes a consistent batch
                    assert_eq!(log.commit_count(), summary.commits as usize, "{name}");
                }
                Err(_) => assert_logs_equal(&log, &before),
            }
        }
    }
}

#[test]
fn specific_malformations_are_rejected() {
    let good = one_commit_batch();
    let apply = |bytes: &[u8]| {
        let mut log = EventLog::new();
        let before = log.clone();
        let r = apply_batch(&mut log, bytes);
        if r.is_err() {
            assert_logs_equal(&log, &before);
        }
        r
    };
    assert!(apply(&good.bytes()).is_ok());

    let mut b = good.bytes();
    b[0] = b'X';
    assert!(apply(&b).unwrap_err().0.contains("magic"));

    // version 1 batches (no set table) are refused
    let mut b = good.bytes();
    b[3] = 1;
    assert!(apply(&b).unwrap_err().0.contains("version"));

    // more strings announced than present
    let mut hand = one_commit_batch();
    let mut b = hand.bytes();
    b[16] = 3;
    assert!(apply(&b).is_err());

    // commit count too high
    hand.commit_count = 2;
    assert!(apply(&hand.bytes()).is_err());

    // change count that does not add up
    let mut hand = one_commit_batch();
    hand.change_count = 2;
    assert!(apply(&hand.bytes()).unwrap_err().0.contains("change count"));

    // trailing bytes
    let mut b = good.bytes();
    b.push(0);
    assert!(apply(&b).unwrap_err().0.contains("trailing"));

    // invalid UTF-8 in a string
    let mut b = good.bytes();
    let at = b.windows(5).position(|w| w == b"alice").unwrap();
    b[at] = 0xff;
    assert!(apply(&b).unwrap_err().0.contains("UTF-8"));

    // user id out of range
    let mut hand = one_commit_batch();
    assert_eq!(hand.commits[..2], [0xc8, 0x01]); // zigzag(100) takes two bytes
    hand.commits[2] = 2; // the user: ids 0 and 1 exist
    let b = hand.bytes();
    assert!(apply(&b).unwrap_err().0.contains("user id"));

    // reserved flag bit and unknown kind
    for flags in [0b1000u8, 0b11, 0b1_0000_0000u32 as u8 | 0x80] {
        let mut b = one_commit_batch().bytes();
        let at = b.len() - 4;
        b[at] = flags;
        assert!(apply(&b).is_err(), "flags {flags:#b}");
    }

    // a commit without changes
    let mut hand = one_commit_batch();
    hand.commits = Vec::new();
    zigzag(&mut hand.commits, 1);
    varint(&mut hand.commits, 0);
    zigzag(&mut hand.commits, 0);
    varint(&mut hand.commits, 0);
    hand.change_count = 0;
    assert!(
        apply(&hand.bytes())
            .unwrap_err()
            .0
            .contains("without changes")
    );

    // a varint that does not end
    let mut hand = one_commit_batch();
    hand.commits = vec![0x80; 11];
    assert!(apply(&hand.bytes()).is_err());
    // and one that overflows 64 bits in its tenth byte
    let mut hand = one_commit_batch();
    hand.commits = [vec![0xff; 9], vec![0x02]].concat();
    assert!(apply(&hand.bytes()).is_err());

    // string ids and counts that would overflow u32
    let mut hand = one_commit_batch();
    hand.first_string = 0;
    let mut b = hand.bytes();
    b[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(apply(&b).is_err());

    // header cut short
    assert!(apply(&good.bytes()[..10]).is_err());
    assert!(apply(b"GEB").is_err());
    assert!(apply(b"G").is_err());
}

#[test]
fn a_hostile_header_does_not_allocate_by_its_counts() {
    // counts near u32::MAX with almost no payload must fail fast
    let mut b = b"GEB\x02".to_vec();
    for n in [0u32, 1, 0, u32::MAX - 1, 0, u32::MAX, u32::MAX] {
        b.extend_from_slice(&n.to_le_bytes());
    }
    let mut log = EventLog::new();
    let started = std::time::Instant::now();
    assert!(apply_batch(&mut log, &b).is_err());
    assert!(started.elapsed().as_secs() < 5);
}

// --- attribution sets ---------------------------------------------------------

use gource_core::events::{AttrKind, AttrSetId, Attribution, MAX_ATTR_SETS};

/// One commit of "alice" on "/a.rs" with the set {assist claude "Claude Opus 5.5"}.
fn one_set_batch() -> Hand {
    let mut sets = Vec::new();
    varint(&mut sets, 1); // one entry
    sets.push(0); // assist
    varint(&mut sets, 2); // "claude"
    varint(&mut sets, 3 + 1); // "Claude Opus 5.5"
    let mut commits = Vec::new();
    zigzag(&mut commits, 100);
    varint(&mut commits, 0);
    zigzag(&mut commits, 0);
    varint(&mut commits, (1 << 1) | 1);
    varint(&mut commits, 1); // set 1
    varint(&mut commits, 1);
    commits.push(0);
    Hand {
        first_string: 0,
        strings: vec!["alice", "/a.rs", "claude", "Claude Opus 5.5"],
        first_set: 1,
        set_count: 1,
        sets,
        first_commit: 0,
        commit_count: 1,
        change_count: 1,
        commits,
    }
}

#[test]
fn a_hand_built_batch_with_a_set_applies() {
    let mut log = EventLog::new();
    let summary = apply_batch(&mut log, &one_set_batch().bytes()).unwrap();
    assert_eq!((summary.sets, summary.commits), (1, 1));
    let c = log.commit(0);
    assert_eq!(c.attrs, AttrSetId(1));
    let set = log.attr_set(c.attrs);
    assert_eq!(set.len(), 1);
    assert_eq!(set[0].kind, AttrKind::Assist);
    assert_eq!(log.str(set[0].id), "claude");
    assert_eq!(log.str(set[0].model.unwrap()), "Claude Opus 5.5");
    assert_eq!(log.attr_commit_counts(), &[0, 1]);
}

#[test]
fn attribution_strings_are_interned_between_user_and_paths() {
    let bytes = b"1|zed|A|/b||ai=co:claude:Claude Opus 5.5\n1|zed|A|/a\n2|amy|A|/b||ai=au:cursor,co:claude:Claude Opus 5.5\n3|zed|M|/a||ai=co:claude:Claude Opus 5.5\n";
    let log = direct(bytes, 100);
    let strings: Vec<&str> = (0..log.string_count())
        .map(|i| log.str(StrId(i as u32)))
        .collect();
    assert_eq!(
        strings,
        [
            "zed",
            "claude",
            "Claude Opus 5.5",
            "/b",
            "/a",
            "amy",
            "cursor"
        ]
    );
    let sets: Vec<u32> = log.records().iter().map(|r| r.attrs.0).collect();
    assert_eq!(sets, [1, 2, 1]);
    assert_eq!(log.attr_set(AttrSetId(2))[0].kind, AttrKind::Agent);
    assert_eq!(log.attr_commit_counts(), &[0, 2, 1]);
    for every in [1, 2, usize::MAX] {
        assert_logs_equal(&log, &apply_all(&encode(bytes, 100, every)));
    }
}

#[test]
fn sets_are_sent_once_and_numbered_across_batches() {
    let bytes = b"1|a|A|/x||ai=co:codex\n2|a|M|/x||ai=co:codex\n3|b|M|/x||ai=au:cursor\n4|b|M|/y\n";
    let batches = encode(bytes, 1000, 1);
    assert_eq!(batches.len(), 4);
    let header = |b: &[u8], at: usize| u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]);
    let first_sets: Vec<u32> = batches.iter().map(|b| header(b, 8)).collect();
    let set_counts: Vec<u32> = batches.iter().map(|b| header(b, 20)).collect();
    assert_eq!(first_sets, [1, 2, 2, 3]);
    assert_eq!(set_counts, [1, 0, 1, 0]);
    let log = apply_all(&batches);
    let sets: Vec<u32> = log.records().iter().map(|r| r.attrs.0).collect();
    assert_eq!(sets, [1, 1, 2, 0]);
    assert_logs_equal(&direct(bytes, 1000), &log);

    // a batch whose first_set does not follow the log is refused
    let mut log = EventLog::new();
    apply_batch(&mut log, &batches[0]).unwrap();
    apply_batch(&mut log, &batches[1]).unwrap();
    let mut wrong = batches[2].clone();
    wrong[8] = 1;
    let before = log.clone();
    let err = apply_batch(&mut log, &wrong).unwrap_err();
    assert!(err.0.contains("attribution set"), "{err}");
    assert_logs_equal(&log, &before);
}

#[test]
fn malformed_sets_are_rejected() {
    let apply = |hand: &Hand| {
        let mut log = EventLog::new();
        let r = apply_batch(&mut log, &hand.bytes());
        if r.is_err() {
            assert_logs_equal(&log, &EventLog::new());
        }
        r
    };
    assert!(apply(&one_set_batch()).is_ok());

    // entry count 0 and 5
    for n in [0u8, 5] {
        let mut hand = one_set_batch();
        hand.sets[0] = n;
        let err = apply(&hand).unwrap_err();
        assert!(err.0.contains("set size"), "{n}: {err}");
    }
    // unknown kind; kind 2 (bot) is reserved but accepted
    let mut hand = one_set_batch();
    hand.sets[1] = 3;
    assert!(apply(&hand).unwrap_err().0.contains("kind"));
    let mut hand = one_set_batch();
    hand.sets[1] = 2;
    assert!(apply(&hand).is_ok());
    // agent and model string ids out of range
    let mut hand = one_set_batch();
    hand.sets[2] = 4;
    assert!(apply(&hand).unwrap_err().0.contains("string id"));
    let mut hand = one_set_batch();
    hand.sets[3] = 5;
    assert!(apply(&hand).unwrap_err().0.contains("string id"));
    // a commit naming set 2 (only 1 exists) or set 0 explicitly
    let at = 5; // zigzag(100) 2 bytes, user, offset, head
    let mut hand = one_set_batch();
    assert_eq!(hand.commits[at], 1);
    hand.commits[at] = 2;
    assert!(apply(&hand).unwrap_err().0.contains("set id"));
    let mut hand = one_set_batch();
    hand.commits[at] = 0;
    assert!(apply(&hand).unwrap_err().0.contains("set id 0"));
    // a set announced but missing
    let mut hand = one_set_batch();
    hand.set_count = 2;
    assert!(apply(&hand).is_err());
    // more sets than the cap
    let mut hand = one_set_batch();
    hand.set_count = MAX_ATTR_SETS as u32 + 1;
    assert!(apply(&hand).unwrap_err().0.contains("too many"));
}

#[test]
fn a_log_with_more_distinct_sets_than_the_cap_loads_completely() {
    let mut text = String::new();
    for i in 0..=MAX_ATTR_SETS {
        text.push_str(&format!("{i}|u|M|/f||ai=co:claude:m{i}\n"));
    }
    let bytes = text.as_bytes();
    let mut parser = CustomLogParser::new();
    let mut encoder = BatchEncoder::new();
    parser.push(bytes, &mut encoder);
    parser.finish(&mut encoder);
    assert_eq!(parser.stats().bad_meta, 1);
    let mut log = EventLog::new();
    apply_batch(&mut log, &encoder.take_batch()).unwrap();
    assert_eq!(log.commit_count(), MAX_ATTR_SETS + 1);
    assert_eq!(log.attr_set_count(), MAX_ATTR_SETS + 1);
    assert_eq!(log.commit(MAX_ATTR_SETS).attrs, AttrSetId::NONE);
    assert_eq!(
        log.commit(MAX_ATTR_SETS - 1).attrs,
        AttrSetId(MAX_ATTR_SETS as u32)
    );
    assert_logs_equal(&direct(bytes, bytes.len()), &log);
}

#[test]
fn sink_sets_match_core_interning() {
    let mut log = EventLog::new();
    let file = [ParsedFile {
        path: "/x",
        kind: ChangeKind::Add,
        colour: None,
        event: ChangeEvent::Commit,
    }];
    let attrs = [gource_sources::ParsedAttribution {
        kind: AttrKind::Agent,
        agent: "copilot",
        model: None,
    }];
    EventLogSink::new(&mut log).commit(&ParsedCommit {
        timestamp: 1,
        username: "u",
        files: &file,
        attrs: &attrs,
        byte_offset: 0,
    });
    let copilot = log.lookup("copilot").unwrap();
    assert_eq!(
        log.attr_set(AttrSetId(1)),
        &[Attribution {
            kind: AttrKind::Agent,
            id: copilot,
            model: None
        }]
    );
}
