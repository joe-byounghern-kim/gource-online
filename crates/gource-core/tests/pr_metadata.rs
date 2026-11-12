// SPDX-License-Identifier: GPL-3.0-or-later

//! Frozen PR metadata storage, isolated from simulation and rendering.

use gource_core::events::{
    AttrSetId, ChangeEvent, ChangeKind, EventLog, FileChange, PrChangeRecord, PrCompleteness,
    PrLifecycle, PrLogCoverage, PrLogMetadata, PrLogUpdate, PrPayload, PrState,
};

fn metadata() -> PrLogMetadata {
    PrLogMetadata {
        version: 1,
        from: 0,
        to: 100,
        as_of: 200,
        lifecycle: PrCompleteness::Complete,
        reviews: PrCompleteness::Partial,
        files: PrCompleteness::Unknown,
    }
}

#[test]
fn coverage_equal_claims_are_idempotent_but_conflicts_are_sticky_unknown() {
    let mut log = EventLog::new();
    assert_eq!(log.pr_coverage(), PrLogCoverage::Absent);
    log.update_pr_log(PrLogUpdate::Metadata(metadata()));
    log.update_pr_log(PrLogUpdate::Metadata(metadata()));
    assert_eq!(log.pr_coverage(), PrLogCoverage::Known(metadata()));
    let mut other = metadata();
    other.to += 1;
    log.update_pr_log(PrLogUpdate::Metadata(other));
    assert_eq!(log.pr_coverage(), PrLogCoverage::Unknown);
    log.update_pr_log(PrLogUpdate::Metadata(metadata()));
    assert_eq!(log.pr_coverage(), PrLogCoverage::Unknown);
    assert_eq!(log.clone().pr_coverage(), PrLogCoverage::Unknown);
    log.clear();
    assert_eq!(log.pr_coverage(), PrLogCoverage::Absent);
}

#[test]
fn sparse_payload_indices_are_offset_once_and_survive_clone() {
    let mut log = EventLog::new();
    let user = log.intern("actor");
    let owner = log.intern("owner");
    let path = log.intern("/src/.pull-requests/#42 Proposal");
    let code = log.intern("/src/a.rs");
    let change = |path, event| FileChange {
        path,
        kind: ChangeKind::Add,
        colour: None,
        event,
    };
    log.push_commit(0, user, [change(code, ChangeEvent::Commit)], 0);
    let payload = PrPayload::Lifecycle(PrLifecycle {
        number: 42,
        owner,
        target: None,
        created: -10,
        state: PrState::Draft,
        initial: true,
    });
    log.push_commit_attr_pr(
        1,
        user,
        &[
            change(code, ChangeEvent::Commit),
            change(path, ChangeEvent::PrOpened),
        ],
        10,
        AttrSetId::NONE,
        &[PrChangeRecord {
            change_index: 1,
            payload,
        }],
    )
    .unwrap();
    assert_eq!(log.pr_change(0), None);
    assert_eq!(log.pr_change(1), None);
    assert_eq!(log.pr_change(2), Some(&payload));
    assert_eq!(log.pr_changes()[0].change_index, 2);
    let cloned = log.clone();
    assert_eq!(cloned.pr_changes(), log.pr_changes());
    assert_eq!(cloned.pr_change(2), Some(&payload));
}

#[test]
fn malformed_sparse_indices_or_wrong_event_payload_do_not_append_any_state() {
    let mut log = EventLog::new();
    let user = log.intern("actor");
    let owner = log.intern("owner");
    let path = log.intern("/pull requests/#42 Proposal");
    let lifecycle = PrPayload::Lifecycle(PrLifecycle {
        number: 42,
        owner,
        target: None,
        created: 0,
        state: PrState::Open,
        initial: false,
    });
    let file = FileChange {
        path,
        kind: ChangeKind::Add,
        colour: None,
        event: ChangeEvent::PrOpened,
    };
    for records in [
        vec![PrChangeRecord {
            change_index: 1,
            payload: lifecycle,
        }],
        vec![
            PrChangeRecord {
                change_index: 0,
                payload: lifecycle
            };
            2
        ],
    ] {
        assert!(
            log.push_commit_attr_pr(1, user, &[file], 0, AttrSetId::NONE, &records)
                .is_err()
        );
        assert_eq!(log.commit_count(), 0);
        assert_eq!(log.change_count(), 0);
        assert!(log.pr_changes().is_empty());
    }
    let code = FileChange {
        event: ChangeEvent::Commit,
        ..file
    };
    assert!(
        log.push_commit_attr_pr(
            1,
            user,
            &[code],
            0,
            AttrSetId::NONE,
            &[PrChangeRecord {
                change_index: 0,
                payload: lifecycle
            }],
        )
        .is_err()
    );
    assert_eq!(log.commit_count(), 0);
}
