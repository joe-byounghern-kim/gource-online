// SPDX-License-Identifier: GPL-3.0-or-later
//! Wrappers must preserve typed changes and immutable coverage, even with no L0 events.
use gource_core::events::EventLog;
use gource_sources::pulls::{PrEvent, PullRequestMerger};
use gource_sources::{CustomLogParser, EventLogSink};

const RICH: &[u8] = b"100|Owner|A|/src/.pull-requests/#7 Frozen||ev=pr-opened;pr=7;pr-author=Owner;pr-created=100;pr-state=draft;pr-initial=1;pr-log=1,100,120,120,c,c,c\n101|Reviewer|M|/src/.pull-requests/#7 Frozen||ev=review-approved;pr=7;pr-author=Owner;pr-created=100;pr-state=draft;pr-initial=0\n";

#[test]
fn empty_legacy_merger_is_transparent_to_rich_payloads_and_log_coverage() {
    let mut expected = EventLog::new();
    let mut parser = CustomLogParser::new();
    parser.push(RICH, &mut EventLogSink::new(&mut expected));
    parser.finish(&mut EventLogSink::new(&mut expected));
    let mut actual = EventLog::new();
    let mut merger = PullRequestMerger::new(EventLogSink::new(&mut actual), Vec::new());
    let mut parser = CustomLogParser::new();
    for chunk in RICH.chunks(1) {
        parser.push(chunk, &mut merger);
    }
    parser.finish(&mut merger);
    merger.finish();
    assert_eq!(actual.pr_changes(), expected.pr_changes());
    assert_eq!(actual.pr_coverage(), expected.pr_coverage());
    assert_eq!(actual.records(), expected.records());
    for i in 0..expected.string_count() {
        assert_eq!(
            actual.str(gource_core::events::StrId(i as u32)),
            expected.str(gource_core::events::StrId(i as u32))
        );
    }
}

#[test]
fn typed_commit_at_minimum_timestamp_precedes_equal_time_legacy_event() {
    use gource_core::events::{ChangeEvent, ChangeKind};
    let input = b"-9223372036854775808|Owner|A|/src/.pull-requests/#7 Frozen||ev=pr-opened;pr=7;pr-author=Owner;pr-created=-9223372036854775808;pr-state=open;pr-initial=1\n";
    let event = PrEvent {
        timestamp: i64::MIN,
        author: "legacy".into(),
        path: "/pull requests/#8 Basic".into(),
        kind: ChangeKind::Add,
        colour: [0, 255, 0],
        event: ChangeEvent::PrOpened,
    };
    let mut log = EventLog::new();
    let mut merger = PullRequestMerger::new(EventLogSink::new(&mut log), vec![event]);
    let mut parser = CustomLogParser::new();
    parser.push(input, &mut merger);
    parser.finish(&mut merger);
    merger.finish();
    assert_eq!(log.commit_count(), 2);
    assert_eq!(log.str(log.commit(0).user), "Owner");
    assert_eq!(log.str(log.commit(1).user), "legacy");
    assert_eq!(log.pr_changes().len(), 1);
}
