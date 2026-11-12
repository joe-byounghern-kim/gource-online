// SPDX-License-Identifier: GPL-3.0-or-later

//! Pull request events (`ev=` in the custom log's meta field) through every
//! path: the custom-log parser, the event-log sink, the worker batch codec
//! and the writer.

use gource_core::events::{ChangeEvent, ChangeKind, EventLog};
use gource_sources::{
    BatchEncoder, CustomLogParser, CustomLogWriter, EventLogSink, apply_batch, meta,
};

const LOG: &str = "\
1|alice|A|/pull requests/#1 One|3FB950|ev=pr-opened
2|bob|M|/src/a.rs
3|alice|D|/pull requests/#1 One|A371F7|ev=pr-merged
4|carol|D|/pull requests/#2 Two||ev=pr-closed
5|dave|A|/x||ev=ci-pass
6|erin|A|/y||ai=co:claude;ev=pr-opened
";

/// `(path, kind, event)` of every change, in order.
fn changes(log: &EventLog) -> Vec<(String, ChangeKind, ChangeEvent)> {
    (0..log.commit_count())
        .flat_map(|i| log.commit(i).changes.to_vec())
        .map(|c| (log.str(c.path).to_string(), c.kind, c.event))
        .collect()
}

fn parsed() -> (EventLog, u64) {
    let mut log = EventLog::new();
    let mut parser = CustomLogParser::new();
    let mut sink = EventLogSink::new(&mut log);
    parser.push(LOG.as_bytes(), &mut sink);
    parser.finish(&mut sink);
    (log, parser.stats().bad_meta)
}

#[test]
fn the_parser_reads_event_kinds() {
    let (log, bad) = parsed();
    assert_eq!(bad, 0, "an unknown ev value is not an error");
    assert_eq!(
        changes(&log),
        [
            (
                "/pull requests/#1 One".into(),
                ChangeKind::Add,
                ChangeEvent::PrOpened
            ),
            ("/src/a.rs".into(), ChangeKind::Modify, ChangeEvent::Commit),
            (
                "/pull requests/#1 One".into(),
                ChangeKind::Delete,
                ChangeEvent::PrMerged
            ),
            (
                "/pull requests/#2 Two".into(),
                ChangeKind::Delete,
                ChangeEvent::PrClosed
            ),
            ("/x".into(), ChangeKind::Add, ChangeEvent::Commit),
            ("/y".into(), ChangeKind::Add, ChangeEvent::PrOpened),
        ]
    );
    // The ai pair next to ev still credits the commit.
    assert!(!log.commit(5).attrs.is_none());
    assert_eq!(log.commit(0).changes[0].colour, Some([0x3F, 0xB9, 0x50]));
}

#[test]
fn batches_carry_event_kinds_in_any_chunking() {
    let (direct, _) = parsed();
    for chunk in [1, 3, 17, LOG.len()] {
        let mut parser = CustomLogParser::new();
        let mut enc = BatchEncoder::new();
        let mut log = EventLog::new();
        for c in LOG.as_bytes().chunks(chunk) {
            parser.push(c, &mut enc);
            apply_batch(&mut log, &enc.take_batch()).unwrap();
        }
        parser.finish(&mut enc);
        apply_batch(&mut log, &enc.take_batch()).unwrap();
        assert_eq!(changes(&log), changes(&direct), "chunk {chunk}");
    }
}

#[test]
fn a_batch_with_an_unknown_event_code_is_refused() {
    let mut enc = BatchEncoder::new();
    let mut parser = CustomLogParser::new();
    parser.push(b"1|a|A|/p||ev=pr-merged\n", &mut enc);
    parser.finish(&mut enc);
    let mut batch = enc.take_batch();
    // The last byte is the change's flags: add (0) | colour bit off | event 2.
    let last = batch.len() - 1;
    assert_eq!(batch[last], 2 << 3);
    batch[last] = 7 << 3;
    let mut log = EventLog::new();
    assert!(apply_batch(&mut log, &batch).is_err());
    assert_eq!(log.commit_count(), 0);
    batch[last] = 1 << 6;
    assert!(apply_batch(&mut log, &batch).is_err());
}

#[test]
fn the_writer_round_trips_event_kinds() {
    let mut writer = CustomLogWriter::new(Vec::new());
    let mut parser = CustomLogParser::new();
    parser.push(LOG.as_bytes(), &mut writer);
    parser.finish(&mut writer);
    let text = String::from_utf8(writer.finish().unwrap()).unwrap();
    assert!(
        text.contains("1|alice|A|/pull requests/#1 One|3FB950|ev=pr-opened\n"),
        "{text}"
    );
    assert!(
        text.contains("4|carol|D|/pull requests/#2 Two||ev=pr-closed\n"),
        "{text}"
    );
    assert!(text.contains("2|bob|M|/src/a.rs\n"), "{text}");
    // The unknown kind was read as a commit change, so nothing is written.
    assert!(text.contains("5|dave|A|/x\n"), "{text}");

    let mut again = EventLog::new();
    let mut p = CustomLogParser::new();
    let mut sink = EventLogSink::new(&mut again);
    p.push(text.as_bytes(), &mut sink);
    p.finish(&mut sink);
    let (direct, _) = parsed();
    assert_eq!(changes(&again), changes(&direct));
}

#[test]
fn meta_event_values() {
    assert_eq!(meta::parse_event(b"ev=pr-closed"), ChangeEvent::PrClosed);
    assert_eq!(meta::parse_event(b"ev=pr%2Dmerged"), ChangeEvent::PrMerged);
    assert_eq!(
        meta::parse_event(b"x-a=1;ev=pr-opened"),
        ChangeEvent::PrOpened
    );
    assert_eq!(meta::parse_event(b"ev=ci-fail"), ChangeEvent::Commit);
    assert_eq!(meta::parse_event(b""), ChangeEvent::Commit);
    // The first of two pairs with the same key wins.
    assert_eq!(
        meta::parse_event(b"ev=pr-opened;ev=pr-closed"),
        ChangeEvent::PrOpened
    );
    // A malformed value is a bad pair and leaves the line a commit change.
    let (_, bad) = meta::parse("ev=pr-opened\u{200e}".as_bytes());
    assert_eq!(bad, 1);
    assert_eq!(
        meta::parse_event("ev=pr-opened\u{200e}".as_bytes()),
        ChangeEvent::Commit
    );
}
