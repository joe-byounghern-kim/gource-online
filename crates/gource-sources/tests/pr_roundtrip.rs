// SPDX-License-Identifier: GPL-3.0-or-later

//! Real parser/direct/batch/export invariants for typed PR metadata.

use gource_core::events::{ChangeEvent, EventLog, PrLogCoverage, PrPayload, PrState};
use gource_sources::{
    BatchEncoder, CommitSink, CustomLogParser, CustomLogWriter, EventLogSink, ParsedCommit,
    ParsedFile, apply_batch,
};

fn parse(input: &[u8], chunk: usize) -> EventLog {
    let mut log = EventLog::new();
    let mut parser = CustomLogParser::new();
    let mut sink = EventLogSink::new(&mut log);
    for bytes in input.chunks(chunk) {
        parser.push(bytes, &mut sink);
    }
    parser.finish(&mut sink);
    log
}

#[test]
fn malformed_rich_records_are_not_reinterpreted_as_code_mutations() {
    let input = b"1|actor|A|/src/a.rs\n2|actor|A|/pull requests/#42 Proposal||ev=pr-opened;pr=42;pr-author=owner\n3|actor|M|/src/a.rs\n";
    for chunk in [1, 11, input.len()] {
        let log = parse(input, chunk);
        assert_eq!(log.commit_count(), 2);
        assert_eq!(log.change_count(), 2);
        assert!(log.pr_changes().is_empty());
        assert_eq!(log.pr_coverage(), PrLogCoverage::Unknown);
        for index in 0..log.commit_count() {
            let file = log.commit(index).changes[0];
            assert_eq!(log.str(file.path), "/src/a.rs");
            assert_eq!(file.event, ChangeEvent::Commit);
        }
    }
}

#[test]
fn rich_creation_and_owner_are_typed_and_not_the_action_actor() {
    let input = b"100|reviewer|M|/src/.pull-requests/#42 Proposal||ev=review-approved;pr=42;pr-author=owner;pr-created=-5;pr-state=draft;pr-initial=0\n";
    let log = parse(input, 1);
    assert_eq!(log.commit_count(), 1);
    let commit = log.commit(0);
    assert_eq!(log.str(commit.user), "reviewer");
    assert_eq!(commit.changes[0].event, ChangeEvent::ReviewApproved);
    let Some(PrPayload::Lifecycle(pr)) = log.pr_change(0) else {
        panic!("typed lifecycle missing")
    };
    assert_eq!(pr.number, 42);
    assert_eq!(log.str(pr.owner), "owner");
    assert_eq!(pr.created, -5);
    assert_eq!(pr.state, PrState::Draft);
    assert!(!pr.initial);
}

#[test]
fn bare_commit_number_cannot_create_a_repository_association() {
    let log = parse(b"1|actor|A|/src/a.rs||pr=42;ai=co:claude\n", 7);
    assert_eq!(log.commit_count(), 1);
    assert_eq!(log.commit(0).changes[0].event, ChangeEvent::Commit);
    assert!(log.pr_changes().is_empty());
    assert_eq!(log.pr_coverage(), PrLogCoverage::Absent);
    assert!(!log.commit(0).attrs.is_none());
}

fn batches(input: &[u8], chunk: usize) -> (EventLog, Vec<Vec<u8>>) {
    let mut encoder = BatchEncoder::new();
    let mut parser = CustomLogParser::new();
    let mut log = EventLog::new();
    let mut encoded = Vec::new();
    for bytes in input.chunks(chunk) {
        parser.push(bytes, &mut encoder);
        let batch = encoder.take_batch();
        if !batch.is_empty() {
            apply_batch(&mut log, &batch).unwrap();
            encoded.push(batch);
        }
    }
    parser.finish(&mut encoder);
    let batch = encoder.take_batch();
    if !batch.is_empty() {
        apply_batch(&mut log, &batch).unwrap();
        encoded.push(batch);
    }
    (log, encoded)
}

const RICH: &[u8] = b"1|actor|A|/src/a.rs\n2|owner|A|/src/.pull-requests/#42 Proposal||ev=pr-opened;pr=42;pr-author=owner;pr-created=-5;pr-state=draft;pr-initial=1;ai=co:claude;pr-log=1,0,10,20,c,p,c\n3|reviewer|M|/src/.pull-requests/#42 Proposal||ev=review-approved;pr=42;pr-author=owner;pr-created=-5;pr-state=draft;pr-initial=0\n4|merger|D|/src/.pull-requests/#42 Proposal||ev=pr-merged;pr=42;pr-author=owner;pr-created=-5;pr-state=draft;pr-initial=0\n5|actor|M|/src/a.rs\n";

#[test]
fn rich_worker_batches_match_direct_ids_payloads_and_coverage_in_every_chunking() {
    let direct = parse(RICH, RICH.len());
    for chunk in [1, 7, 41, RICH.len()] {
        let (worker, encoded) = batches(RICH, chunk);
        assert!(encoded.iter().any(|batch| batch[3] == 3));
        assert_eq!(worker.string_count(), direct.string_count());
        for i in 0..direct.string_count() {
            let id = gource_core::events::StrId(i as u32);
            assert_eq!(worker.str(id), direct.str(id));
        }
        assert_eq!(worker.records(), direct.records());
        for i in 0..direct.commit_count() {
            assert_eq!(worker.commit(i).changes, direct.commit(i).changes);
            assert_eq!(
                worker.attr_set(worker.commit(i).attrs),
                direct.attr_set(direct.commit(i).attrs)
            );
        }
        assert_eq!(worker.pr_changes(), direct.pr_changes());
        assert_eq!(worker.pr_coverage(), direct.pr_coverage());
        assert!(matches!(worker.pr_coverage(), PrLogCoverage::Known(_)));
    }
}

#[test]
fn custom_export_import_preserves_typed_owner_creation_seed_and_coverage() {
    let mut parser = CustomLogParser::new();
    let mut writer = CustomLogWriter::new(Vec::new());
    parser.push(RICH, &mut writer);
    parser.finish(&mut writer);
    let encoded = writer.finish().unwrap();
    let restored = parse(&encoded, 1);
    let direct = parse(RICH, RICH.len());
    assert_eq!(restored.commit_count(), direct.commit_count());
    assert_eq!(restored.change_count(), direct.change_count());
    assert_eq!(restored.pr_coverage(), direct.pr_coverage());
    assert_eq!(restored.pr_changes().len(), direct.pr_changes().len());
    for (a, b) in restored.pr_changes().iter().zip(direct.pr_changes()) {
        assert_eq!(a.change_index, b.change_index);
        let (PrPayload::Lifecycle(a), PrPayload::Lifecycle(b)) = (a.payload, b.payload) else {
            panic!("lifecycle required")
        };
        assert_eq!(a.number, b.number);
        assert_eq!(restored.str(a.owner), direct.str(b.owner));
        assert_eq!(
            (a.created, a.state, a.initial),
            (b.created, b.state, b.initial)
        );
    }
}

#[test]
fn sticky_unknown_rich_export_fails_without_delivering_a_successful_partial_artifact() {
    let mut writer = CustomLogWriter::new(Vec::new());
    writer.pr_log(gource_core::events::PrLogUpdate::Unknown);
    let files = [ParsedFile {
        path: "/a",
        kind: gource_core::events::ChangeKind::Add,
        colour: None,
        event: ChangeEvent::Commit,
    }];
    writer.commit(&ParsedCommit {
        timestamp: 1,
        username: "actor",
        files: &files,
        attrs: &[],
        byte_offset: 0,
    });
    assert!(writer.out.is_empty());
    assert!(writer.finish().is_err());
}

#[test]
fn unknown_arriving_after_output_latches_failure_and_stops_later_writes() {
    let mut writer = CustomLogWriter::new(Vec::new());
    let files = [ParsedFile {
        path: "/a",
        kind: gource_core::events::ChangeKind::Add,
        colour: None,
        event: ChangeEvent::Commit,
    }];
    let commit = ParsedCommit {
        timestamp: 1,
        username: "actor",
        files: &files,
        attrs: &[],
        byte_offset: 0,
    };
    writer.commit(&commit);
    let before = writer.out.clone();
    assert!(!before.is_empty());
    writer.pr_log(gource_core::events::PrLogUpdate::Unknown);
    writer.commit(&commit);
    assert_eq!(writer.out, before);
    assert!(writer.finish().is_err());
}

#[test]
fn malformed_optional_carrier_does_not_destroy_valid_mandatory_lifecycle() {
    let input = b"1|owner|A|/pull requests/#42 Proposal||ev=pr-opened;pr=42;pr-author=owner;pr-created=0;pr-state=open;pr-initial=0;pr-log=%ZZ\n";
    let log = parse(input, 1);
    assert_eq!(log.commit_count(), 1);
    assert_eq!(log.pr_changes().len(), 1);
    assert_eq!(log.pr_coverage(), PrLogCoverage::Unknown);
}

#[test]
fn explicit_reduced_retry_preserves_code_and_ai_without_fake_pr_mutations() {
    let mut log = parse(RICH, 1);
    log.update_pr_log(gource_core::events::PrLogUpdate::Unknown);
    let mut rich = CustomLogWriter::new(Vec::new());
    rich.write_event_log(&log);
    assert!(rich.out.is_empty());
    assert!(rich.finish().is_err());
    let mut reduced = CustomLogWriter::new(Vec::new());
    reduced.write_event_log_reduced(&log);
    assert_eq!(reduced.stats().coverage_unknown, 1);
    let restored = parse(&reduced.finish().unwrap(), 1);
    assert_eq!(restored.commit_count(), 2);
    assert_eq!(restored.change_count(), 2);
    assert!(restored.pr_changes().is_empty());
    for i in 0..restored.commit_count() {
        let file = restored.commit(i).changes[0];
        assert_eq!(file.event, ChangeEvent::Commit);
        assert_eq!(restored.str(file.path), "/src/a.rs");
    }
}

#[test]
fn coverage_only_v3_has_no_fake_commit_and_unknown_never_upgrades() {
    use gource_core::events::{PrCompleteness, PrLogMetadata, PrLogUpdate};
    let metadata = PrLogMetadata {
        version: 1,
        from: -100,
        to: 10,
        as_of: -20,
        lifecycle: PrCompleteness::Complete,
        reviews: PrCompleteness::Partial,
        files: PrCompleteness::Unknown,
    };
    let mut encoder = BatchEncoder::new();
    encoder.pr_log(PrLogUpdate::Metadata(metadata));
    let expected = encoder.pending_bytes();
    let batch = encoder.take_batch();
    assert_eq!(batch.len(), expected);
    assert_eq!(batch[3], 3);
    let mut log = EventLog::new();
    apply_batch(&mut log, &batch).unwrap();
    assert_eq!(log.commit_count(), 0);
    assert_eq!(log.string_count(), 0);
    assert_eq!(log.pr_coverage(), PrLogCoverage::Known(metadata));
    let mut exported = EventLog::new();
    apply_batch(&mut exported, &gource_sources::encode_event_log(&log)).unwrap();
    assert_eq!(exported.pr_coverage(), PrLogCoverage::Known(metadata));
    assert_eq!(exported.commit_count(), 0);
    assert_eq!(exported.string_count(), 0);
    encoder.pr_log(PrLogUpdate::Unknown);
    apply_batch(&mut log, &encoder.take_batch()).unwrap();
    encoder.pr_log(PrLogUpdate::Metadata(metadata));
    assert!(encoder.take_batch().is_empty());
    assert_eq!(log.pr_coverage(), PrLogCoverage::Unknown);
    let mut restored = EventLog::new();
    apply_batch(&mut restored, &gource_sources::encode_event_log(&log)).unwrap();
    assert_eq!(restored.pr_coverage(), PrLogCoverage::Unknown);
    assert_eq!(restored.commit_count(), 0);
}

#[test]
fn frozen_export_preserves_known_lifecycle_f1_and_uses_v2_for_plain_logs() {
    let rich = parse(b"1|owner|A|/src/.pull-requests/#8 Example||ev=pr-opened;pr=8;pr-author=owner;pr-created=-86400;pr-state=open;pr-initial=1;ai=co:claude;pr-log=1,0,100000,100000,c,c,c\n2|reviewer|M|/src/.pull-requests/#8 Example||ev=review-approved;pr=8;pr-author=owner;pr-created=-86400;pr-state=open;pr-initial=0\n", 1);
    let batch = gource_sources::encode_event_log(&rich);
    assert_eq!(batch[3], 3);
    let mut restored = EventLog::new();
    apply_batch(&mut restored, &batch).unwrap();
    assert_eq!(restored.records(), rich.records());
    assert_eq!(restored.pr_changes(), rich.pr_changes());
    assert_eq!(restored.pr_coverage(), rich.pr_coverage());
    assert_eq!(
        restored.attr_set(restored.commit(0).attrs),
        rich.attr_set(rich.commit(0).attrs)
    );
    for i in 0..rich.commit_count() {
        assert_eq!(restored.commit(i).changes, rich.commit(i).changes);
    }
    let plain = parse(b"1|coder|A|/src/code.rs\n", 1);
    let bytes = gource_sources::encode_event_log(&plain);
    assert_eq!(bytes[3], 2);
    let mut restored = EventLog::new();
    apply_batch(&mut restored, &bytes).unwrap();
    assert_eq!(restored.records(), plain.records());
    assert_eq!(restored.commit(0).changes, plain.commit(0).changes);
    assert!(!restored.has_pr());
}

#[test]
fn lossless_u64_number_survives_custom_batch_and_writer() {
    let input = b"1|owner|A|/pull requests/#18446744073709551615 Proposal||ev=pr-opened;pr=18446744073709551615;pr-author=owner;pr-created=-9223372036854775808;pr-state=open;pr-initial=0\n";
    let direct = parse(input, 1);
    let (worker, _) = batches(input, 3);
    for log in [&direct, &worker] {
        let Some(PrPayload::Lifecycle(p)) = log.pr_change(0) else {
            panic!("payload required")
        };
        assert_eq!(p.number, u64::MAX);
        assert_eq!(p.created, i64::MIN);
    }
    let mut writer = CustomLogWriter::new(Vec::new());
    writer.write_event_log(&direct);
    let restored = parse(&writer.finish().unwrap(), 1);
    let Some(PrPayload::Lifecycle(p)) = restored.pr_change(0) else {
        panic!("payload required")
    };
    assert_eq!(p.number, u64::MAX);
    assert_eq!(p.created, i64::MIN);
}

#[test]
fn conflicting_frozen_owner_drops_rich_record_and_keeps_code_in_both_sinks() {
    let input = b"1|owner|A|/pull requests/#42 Proposal||ev=pr-opened;pr=42;pr-author=owner;pr-created=0;pr-state=open;pr-initial=0\n2|reviewer|M|/pull requests/#42 Proposal||ev=review-approved;pr=42;pr-author=other;pr-created=0;pr-state=open;pr-initial=0\n2|reviewer|A|/src/a.rs\n";
    let direct = parse(input, input.len());
    let (worker, _) = batches(input, input.len());
    assert_eq!(direct.pr_changes().len(), 1);
    assert_eq!(direct.change_count(), 2);
    assert_eq!(direct.pr_coverage(), PrLogCoverage::Unknown);
    assert_eq!(worker.records(), direct.records());
    assert_eq!(worker.pr_changes(), direct.pr_changes());
    assert_eq!(worker.pr_coverage(), direct.pr_coverage());
    assert_eq!(worker.string_count(), direct.string_count());
}

#[test]
fn exact_association_frozen_v3_export_is_lossless_and_custom_requires_explicit_reduction() {
    use gource_core::events::{AttrKind, ChangeKind};
    use gource_sources::{ParsedAttribution, ParsedPrAssociation, ParsedPrChange, ParsedPrPayload};

    let sha = "0123456789abcdef0123456789abcdef01234567";
    let node = "/pull requests/repo-one/#42 Proposal";
    let files = [
        ParsedFile {
            path: "/src/added.rs",
            kind: ChangeKind::Add,
            colour: None,
            event: ChangeEvent::Commit,
        },
        ParsedFile {
            path: "/src/changed.rs",
            kind: ChangeKind::Modify,
            colour: None,
            event: ChangeEvent::Commit,
        },
        ParsedFile {
            path: "/src/deleted.rs",
            kind: ChangeKind::Delete,
            colour: None,
            event: ChangeEvent::Commit,
        },
    ];
    let attrs = [ParsedAttribution {
        kind: AttrKind::Assist,
        agent: "claude",
        model: Some("Opus"),
    }];
    let commit = ParsedCommit {
        timestamp: 100,
        username: "actor",
        files: &files,
        attrs: &attrs,
        byte_offset: 12,
    };
    let records: Vec<_> = (0..3)
        .map(|file_index| ParsedPrChange {
            file_index,
            payload: ParsedPrPayload::Association(ParsedPrAssociation {
                repository: "public/repo-one",
                number: 42,
                source_sha: sha,
                pr_node: node,
            }),
        })
        .collect();
    let mut direct = EventLog::new();
    EventLogSink::new(&mut direct).commit_pr(&commit, &records);
    let mut encoder = BatchEncoder::new();
    encoder.commit_pr(&commit, &records);
    let expected_bytes = encoder.pending_bytes();
    let bytes = encoder.take_batch();
    assert_eq!(bytes.len(), expected_bytes);
    assert_eq!(bytes[3], 3);
    let mut worker = EventLog::new();
    apply_batch(&mut worker, &bytes).unwrap();
    assert_eq!(worker.records(), direct.records());
    assert_eq!(worker.pr_changes(), direct.pr_changes());
    assert_eq!(worker.commit(0).changes, direct.commit(0).changes);
    assert_eq!(
        worker.attr_set(worker.commit(0).attrs),
        direct.attr_set(direct.commit(0).attrs)
    );
    for index in 0..3 {
        let Some(PrPayload::Association(p)) = worker.pr_change(index) else {
            panic!("association missing")
        };
        assert_eq!(p.number, 42);
        assert_eq!(worker.str(p.repository), "public/repo-one");
        assert_eq!(worker.str(p.source_sha), sha);
        assert_eq!(worker.str(p.pr_node), node);
    }
    let exported = gource_sources::encode_event_log(&worker);
    assert_eq!(exported[3], 3);
    let mut imported = EventLog::new();
    apply_batch(&mut imported, &exported).unwrap();
    assert_eq!(imported.records(), worker.records());
    assert_eq!(imported.pr_changes(), worker.pr_changes());
    assert_eq!(imported.pr_coverage(), worker.pr_coverage());
    assert_eq!(
        imported.attr_set(imported.commit(0).attrs),
        worker.attr_set(worker.commit(0).attrs)
    );
    let mut rich = CustomLogWriter::new(Vec::new());
    rich.write_event_log(&worker);
    assert!(
        rich.out.is_empty(),
        "unsupported association export is atomic"
    );
    assert!(
        rich.finish().is_err(),
        "rich export must not silently drop exact associations"
    );
    {
        let mut writer = CustomLogWriter::new(Vec::new());
        writer.write_event_log_reduced(&worker);
        assert_eq!(writer.stats().association_omitted, 3);
        let restored = parse(&writer.finish().unwrap(), 1);
        assert_eq!(restored.change_count(), 3);
        assert!(restored.pr_changes().is_empty());
        assert_eq!(restored.pr_coverage(), PrLogCoverage::Absent);
        for (file, expected) in restored.commit(0).changes.iter().zip(files) {
            assert_eq!(restored.str(file.path), expected.path);
            assert_eq!(file.kind, expected.kind);
            assert_eq!(file.event, ChangeEvent::Commit);
        }
        let attrs = restored.attr_set(restored.commit(0).attrs);
        assert_eq!(attrs.len(), 1);
        assert_eq!(attrs[0].kind, AttrKind::Assist);
        assert_eq!(restored.str(attrs[0].id), "claude");
        assert_eq!(attrs[0].model.map(|id| restored.str(id)), Some("Opus"));
    }
}
