// SPDX-License-Identifier: GPL-3.0-or-later

//! Independent v3 wire fixtures and all-or-nothing rejection checks.

use gource_core::events::{ChangeEvent, EventLog, PrLogCoverage, PrPayload};
use gource_sources::apply_batch;

fn varint(out: &mut Vec<u8>, mut value: u64) {
    while value >= 128 {
        out.push(value as u8 | 128);
        value >>= 7;
    }
    out.push(value as u8);
}

fn fixture(event: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = b"GEB\x03".to_vec();
    for value in [0u32, 1, 0, 3, 0, 1, 1] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    // state=metadata, version=1, from=0, to=10, asof=20, complete/partial/unknown.
    out.extend_from_slice(&[8, 1, 1, 0, 20, 40, 0, 1, 2]);
    for value in ["reviewer", "/src/.pull-requests/#42 Proposal", "owner"] {
        varint(&mut out, value.len() as u64);
        out.extend_from_slice(value.as_bytes());
    }
    // timestamp=100, user=0, offset=0, one change/no attribution set.
    out.extend_from_slice(&[200, 1, 0, 0, 2]);
    out.extend_from_slice(&[1, 1, event]); // path=1, Modify/no colour, explicit event.
    varint(&mut out, payload.len() as u64);
    out.extend_from_slice(payload);
    out
}

const LIFECYCLE: &[u8] = &[1, 42, 2, 0, 9, 1, 0]; // id42, owner2, no target, created=-5, draft, not seed.

#[test]
fn documented_v3_fixture_applies_typed_payload_and_frozen_coverage() {
    let mut log = EventLog::new();
    let summary = apply_batch(&mut log, &fixture(4, LIFECYCLE)).unwrap();
    assert_eq!(
        (summary.strings, summary.commits, summary.changes),
        (3, 1, 1)
    );
    assert_eq!(log.commit(0).changes[0].event, ChangeEvent::ReviewApproved);
    let Some(PrPayload::Lifecycle(pr)) = log.pr_change(0) else {
        panic!("missing typed lifecycle")
    };
    assert_eq!(pr.number, 42);
    assert_eq!(pr.created, -5);
    assert_eq!(log.str(pr.owner), "owner");
    assert!(matches!(log.pr_coverage(), PrLogCoverage::Known(_)));
}

#[test]
fn malformed_v3_never_mutates_strings_sets_records_payloads_or_coverage() {
    let valid = fixture(4, LIFECYCLE);
    let mut cases = vec![
        fixture(10, LIFECYCLE),
        fixture(4, &[]),
        fixture(4, &[99]),
        fixture(4, &[1, 42, 99, 0, 9, 1, 0]),
        fixture(0, LIFECYCLE),
        fixture(4, &[1, 42, 2, 0, 9, 9, 0]),
        fixture(4, &[1, 42, 2, 0, 9, 1, 2]),
    ];
    let mut trailing = valid.clone();
    trailing.push(0);
    cases.push(trailing);
    for end in 1..valid.len() {
        cases.push(valid[..end].to_vec());
    }
    for bytes in cases {
        let mut log = EventLog::new();
        log.set_total_bytes(99);
        log.set_complete(true);
        assert!(apply_batch(&mut log, &bytes).is_err());
        assert_eq!(log.string_count(), 0);
        assert_eq!(log.attr_set_count(), 1);
        assert_eq!(log.commit_count(), 0);
        assert_eq!(log.change_count(), 0);
        assert!(log.pr_changes().is_empty());
        assert_eq!(log.pr_coverage(), PrLogCoverage::Absent);
        assert_eq!(log.total_bytes(), 99);
        assert!(log.is_complete());
    }
}

fn association_fixture(
    log: &EventLog,
    event: u8,
    payload: &[u8],
    sha: &str,
    node: &str,
) -> Vec<u8> {
    let mut out = b"GEB\x03".to_vec();
    for value in [
        log.string_count() as u32,
        log.attr_set_count() as u32,
        log.commit_count() as u32,
        3,
        0,
        1,
        1,
    ] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.push(0); // no coverage update
    for value in ["public/repo-one", sha, node] {
        varint(&mut out, value.len() as u64);
        out.extend_from_slice(value.as_bytes());
    }
    out.extend_from_slice(&[200, 1, 0, 0, 2, 1, 1, event]);
    varint(&mut out, payload.len() as u64);
    out.extend_from_slice(payload);
    out
}

fn assert_unchanged(log: &EventLog, before: &EventLog) {
    assert_eq!(log.string_count(), before.string_count());
    for index in 0..log.string_count() {
        let id = gource_core::events::StrId(index as u32);
        assert_eq!(log.str(id), before.str(id));
    }
    assert_eq!(log.attr_set_count(), before.attr_set_count());
    for index in 0..log.attr_set_count() {
        let id = gource_core::events::AttrSetId(index as u32);
        assert_eq!(log.attr_set(id), before.attr_set(id));
    }
    assert_eq!(log.records(), before.records());
    assert_eq!(log.change_count(), before.change_count());
    for index in 0..log.commit_count() {
        assert_eq!(log.commit(index).changes, before.commit(index).changes);
    }
    assert_eq!(log.pr_changes(), before.pr_changes());
    assert_eq!(log.pr_coverage(), before.pr_coverage());
    assert_eq!(log.total_bytes(), before.total_bytes());
    assert_eq!(log.is_complete(), before.is_complete());
}

#[test]
fn malformed_association_payloads_reject_atomically_against_nonempty_log() {
    let mut original = EventLog::new();
    apply_batch(&mut original, &fixture(4, LIFECYCLE)).unwrap();
    original.set_total_bytes(99);
    original.set_complete(true);
    let sha = "0123456789abcdef0123456789abcdef01234567";
    let node = "/pull requests/repo-one/#42 Proposal";
    // Existing IDs 0..2, appended repo=3, SHA=4 and target node=5.
    let valid = [2, 3, 42, 4, 5];
    let mut positive = original.clone();
    apply_batch(
        &mut positive,
        &association_fixture(&original, 0, &valid, sha, node),
    )
    .unwrap();
    assert!(matches!(
        positive.pr_change(1),
        Some(PrPayload::Association(_))
    ));
    for (event, payload, sha, node) in [
        (0, vec![2, 99, 42, 4, 5], sha, node),
        (0, vec![2, 3, 42, 99, 5], sha, node),
        (0, vec![2, 3, 42, 4, 99], sha, node),
        (0, vec![2, 3, 0, 4, 5], sha, node),
        (0, valid.to_vec(), "not-a-sha", node),
        (
            0,
            valid.to_vec(),
            "gggggggggggggggggggggggggggggggggggggggg",
            node,
        ),
        (0, valid.to_vec(), sha, "relative-node"),
        (4, valid.to_vec(), sha, node),
    ] {
        let mut log = original.clone();
        assert!(
            apply_batch(
                &mut log,
                &association_fixture(&original, event, &payload, sha, node)
            )
            .is_err()
        );
        assert_unchanged(&log, &original);
    }
}

#[test]
fn conflicting_cross_batch_frozen_definition_rejects_all_state_atomically() {
    let mut log = EventLog::new();
    apply_batch(&mut log, &fixture(4, LIFECYCLE)).unwrap();
    let before = log.clone();
    let mut out = b"GEB\x03".to_vec();
    for value in [3u32, 1, 1, 1, 0, 1, 1] {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out.push(0);
    let owner = "different-owner";
    varint(&mut out, owner.len() as u64);
    out.extend_from_slice(owner.as_bytes());
    out.extend_from_slice(&[200, 1, 0, 0, 2, 1, 1, 4]);
    let conflicting = [1, 42, 3, 0, 9, 1, 0];
    varint(&mut out, conflicting.len() as u64);
    out.extend_from_slice(&conflicting);
    assert_eq!(
        apply_batch(&mut log, &out).unwrap_err().to_string(),
        "invalid event batch: conflicting PR definition"
    );
    assert_unchanged(&log, &before);
}
