// SPDX-License-Identifier: GPL-3.0-or-later

//! Encoded-field budgets and faithful rich custom export preflight.

use gource_core::events::{
    ChangeEvent, ChangeKind, PrCompleteness, PrLogMetadata, PrLogUpdate, PrState,
};
use gource_sources::{
    CommitSink, CustomLogParser, CustomLogWriter, EventLogSink, ParsedAttribution, ParsedCommit,
    ParsedFile, ParsedPrChange, ParsedPrLifecycle, ParsedPrPayload,
};

fn emit(owner: &str, coverage: bool, ai: bool) -> CustomLogWriter<Vec<u8>> {
    let files = [ParsedFile {
        path: "/pull requests/#42 Proposal",
        kind: ChangeKind::Add,
        colour: None,
        event: ChangeEvent::PrOpened,
    }];
    let attrs = [ParsedAttribution {
        kind: gource_core::events::AttrKind::Assist,
        agent: "claude",
        model: Some("Claude Opus 5.5"),
    }];
    let commit = ParsedCommit {
        timestamp: 1,
        username: "owner",
        files: &files,
        attrs: if ai { &attrs } else { &[] },
        byte_offset: 0,
    };
    let records = [ParsedPrChange {
        file_index: 0,
        payload: ParsedPrPayload::Lifecycle(ParsedPrLifecycle {
            number: 42,
            owner,
            target: None,
            created: 0,
            state: PrState::Open,
            initial: false,
        }),
    }];
    let mut writer = CustomLogWriter::new(Vec::new());
    if coverage {
        writer.pr_log(PrLogUpdate::Metadata(PrLogMetadata {
            version: 1,
            from: 0,
            to: 10,
            as_of: 20,
            lifecycle: PrCompleteness::Complete,
            reviews: PrCompleteness::Complete,
            files: PrCompleteness::Complete,
        }));
    }
    writer.commit_pr(&commit, &records);
    writer
}

#[test]
fn mandatory_encoded_overflow_rejects_before_writing_any_bytes_or_trimming_ai() {
    let writer = emit(&";".repeat(150), false, true);
    assert!(writer.out.is_empty());
    assert_eq!(writer.stats().meta_trimmed, 0);
    assert!(writer.finish().is_err());
}

#[test]
fn optional_carrier_overflow_warns_but_keeps_every_mandatory_field_and_ai() {
    let writer = emit(&"x".repeat(400), true, true);
    assert!(writer.error.is_none());
    assert_eq!(writer.stats().coverage_unknown, 1);
    assert_eq!(writer.stats().meta_trimmed, 0);
    let output = writer.finish().unwrap();
    let field = output.split(|&b| b == b'|').nth(5).unwrap();
    assert!(field.len() - 1 <= 512);
    assert!(!String::from_utf8_lossy(field).contains("pr-log="));
    let mut parser = CustomLogParser::new();
    let mut log = gource_core::events::EventLog::new();
    let mut sink = EventLogSink::new(&mut log);
    parser.push(&output, &mut sink);
    parser.finish(&mut sink);
    assert_eq!(log.pr_changes().len(), 1);
    assert!(!log.commit(0).attrs.is_none());
}

#[test]
fn labels_that_custom_decoding_would_change_are_rejected_not_silently_normalized() {
    let writer = emit(" owner ", false, false);
    assert!(writer.out.is_empty());
    assert!(writer.finish().is_err());
}

#[test]
fn known_coverage_without_rich_carrier_keeps_free_l0_bytes_and_warns() {
    let input = b"1|owner|A|/pull requests/#42 Proposal||ev=pr-opened\n";
    let mut log = gource_core::events::EventLog::new();
    let mut parser = CustomLogParser::new();
    let mut sink = EventLogSink::new(&mut log);
    parser.push(input, &mut sink);
    parser.finish(&mut sink);
    log.update_pr_log(PrLogUpdate::Metadata(PrLogMetadata {
        version: 1,
        from: 0,
        to: 10,
        as_of: 20,
        lifecycle: PrCompleteness::Complete,
        reviews: PrCompleteness::Complete,
        files: PrCompleteness::Complete,
    }));
    let mut writer = CustomLogWriter::new(Vec::new());
    writer.write_event_log(&log);
    assert_eq!(writer.stats().coverage_unknown, 1);
    assert_eq!(writer.finish().unwrap(), input);
}

fn known_update() -> PrLogUpdate {
    PrLogUpdate::Metadata(PrLogMetadata {
        version: 1,
        from: 0,
        to: 10,
        as_of: 20,
        lifecycle: PrCompleteness::Complete,
        reviews: PrCompleteness::Complete,
        files: PrCompleteness::Complete,
    })
}

#[test]
fn old_commit_seam_rejects_new_events_without_payload_before_any_write_and_stops() {
    for code in 4..=9 {
        let event = ChangeEvent::from_code(code).unwrap();
        let files = [
            ParsedFile {
                path: "/src/real.rs",
                kind: ChangeKind::Modify,
                colour: None,
                event: ChangeEvent::Commit,
            },
            ParsedFile {
                path: "/pull requests/#42 Proposal",
                kind: event.pr_kind().unwrap(),
                colour: None,
                event,
            },
        ];
        let commit = ParsedCommit {
            timestamp: 1,
            username: "actor",
            files: &files,
            attrs: &[],
            byte_offset: 0,
        };
        let mut writer = CustomLogWriter::new(Vec::new());
        writer.commit(&commit);
        assert!(writer.out.is_empty(), "code {code} wrote before validation");
        assert!(writer.error.is_some());
        writer.commit(&ParsedCommit {
            files: &files[..1],
            ..commit
        });
        assert!(writer.out.is_empty());
        assert!(writer.finish().is_err());
    }
}

#[test]
fn streaming_known_without_carrier_warns_for_no_records_and_legacy_records() {
    for emit_legacy in [false, true] {
        let mut writer = CustomLogWriter::new(Vec::new());
        writer.pr_log(known_update());
        let expected = if emit_legacy {
            let files = [ParsedFile {
                path: "/pull requests/#42 Proposal",
                kind: ChangeKind::Add,
                colour: None,
                event: ChangeEvent::PrOpened,
            }];
            writer.commit(&ParsedCommit {
                timestamp: 1,
                username: "owner",
                files: &files,
                attrs: &[],
                byte_offset: 0,
            });
            b"1|owner|A|/pull requests/#42 Proposal||ev=pr-opened\n".as_slice()
        } else {
            b"".as_slice()
        };
        assert_eq!(writer.stats().coverage_unknown, 1);
        assert_eq!(writer.finish().unwrap(), expected);
    }
}

#[test]
fn opening_before_coverage_does_not_consume_the_later_known_carrier() {
    let mut writer = emit("owner", false, false);
    writer.pr_log(known_update());
    assert_eq!(writer.stats().coverage_unknown, 1);
    let files = [ParsedFile {
        path: "/pull requests/#43 Proposal",
        kind: ChangeKind::Add,
        colour: None,
        event: ChangeEvent::PrOpened,
    }];
    writer.commit_pr(
        &ParsedCommit {
            timestamp: 2,
            username: "owner",
            files: &files,
            attrs: &[],
            byte_offset: 0,
        },
        &[ParsedPrChange {
            file_index: 0,
            payload: ParsedPrPayload::Lifecycle(ParsedPrLifecycle {
                number: 43,
                owner: "owner",
                target: None,
                created: 0,
                state: PrState::Open,
                initial: false,
            }),
        }],
    );
    assert_eq!(writer.stats().coverage_unknown, 0);
    let output = writer.finish().unwrap();
    assert_eq!(
        String::from_utf8_lossy(&output).matches("pr-log=").count(),
        1
    );
}
