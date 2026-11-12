// SPDX-License-Identifier: GPL-3.0-or-later
//! Exercise the actual CLI wrapper, not just the rich writer in isolation.
mod common;

use common::run_with_stdin;
use gource_core::events::{EventLog, PrPayload};
use gource_sources::{CustomLogParser, EventLogSink};

const RICH: &str = "100|Owner|A|/src/.pull-requests/#7 Frozen||ev=pr-opened;pr=7;pr-author=Owner;pr-created=100;pr-state=draft;pr-initial=1;ai=co:claude;pr-log=1,100,120,120,c,c,c\n101|Reviewer|M|/src/.pull-requests/#7 Frozen||ev=review-approved;pr=7;pr-author=Owner;pr-created=100;pr-state=draft;pr-initial=0\n102|Reviewer|M|/src/.pull-requests/#7 Frozen||ev=review-changes-requested;pr=7;pr-author=Owner;pr-created=100;pr-state=draft;pr-initial=0\n103|Reviewer|M|/src/.pull-requests/#7 Frozen||ev=review-commented;pr=7;pr-author=Owner;pr-created=100;pr-state=draft;pr-initial=0\n104|Closer|D|/src/.pull-requests/#7 Frozen||ev=pr-closed;pr=7;pr-author=Owner;pr-created=100;pr-state=draft;pr-initial=0\n104|Reopener|A|/src/.pull-requests/#7 Frozen||ev=pr-reopened;pr=7;pr-author=Owner;pr-created=100;pr-state=draft;pr-initial=0\n105|Ready actor|M|/src/.pull-requests/#7 Frozen||ev=pr-ready;pr=7;pr-author=Owner;pr-created=100;pr-state=draft;pr-initial=0\n106|Requester|M|/src/.pull-requests/#7 Frozen||ev=pr-review-requested;pr=7;pr-author=Owner;pr-created=100;pr-state=draft;pr-initial=0;pr-target=Target\n107|Actual merger|D|/src/.pull-requests/#7 Frozen||ev=pr-merged;pr=7;pr-author=Owner;pr-created=100;pr-state=draft;pr-initial=0\n";

fn parse(bytes: &[u8]) -> EventLog {
    let mut log = EventLog::new();
    let mut parser = CustomLogParser::new();
    let mut sink = EventLogSink::new(&mut log);
    parser.push(bytes, &mut sink);
    parser.finish(&mut sink);
    log
}

#[test]
fn custom_cli_retains_all_rich_kinds_owner_target_seed_coverage_and_f1() {
    let expected = parse(RICH.as_bytes());
    let output = run_with_stdin(["convert", "--from", "custom", "-", "-"], RICH.as_bytes()).ok();
    let actual = parse(output.stdout.as_bytes());
    assert_eq!(actual.pr_coverage(), expected.pr_coverage());
    assert_eq!(actual.commit_count(), 9);
    assert_eq!(actual.pr_changes().len(), 9);
    for i in 0..9 {
        let before = expected.commit(i);
        let after = actual.commit(i);
        assert_eq!(after.timestamp, before.timestamp);
        assert_eq!(actual.str(after.user), expected.str(before.user));
        assert_eq!(after.changes[0].event, before.changes[0].event);
        assert_eq!(after.changes[0].kind, before.changes[0].kind);
        let Some(PrPayload::Lifecycle(pr)) = actual.pr_change(i) else {
            panic!("typed lifecycle lost at change {i}")
        };
        assert_eq!(actual.str(pr.owner), "Owner");
        assert_eq!(pr.created, 100);
        assert_eq!(pr.initial, i == 0);
        assert_eq!(
            pr.target.map(|id| actual.str(id)),
            (i == 7).then_some("Target")
        );
    }
    let credits = actual.attr_set(actual.commit(0).attrs);
    assert_eq!(credits.len(), 1);
    assert_eq!(actual.str(credits[0].id), "claude");
    let second = run_with_stdin(
        ["convert", "--from", "custom", "-", "-"],
        output.stdout.as_bytes(),
    )
    .ok();
    assert_eq!(second.stdout, output.stdout, "canonical export is stable");
}

#[test]
fn custom_cli_keeps_typed_old_event_codes_instead_of_downgrading_to_l0() {
    let input = "100|Owner|A|/src/.pull-requests/#7 Frozen||ev=pr-opened;pr=7;pr-author=Owner;pr-created=100;pr-state=open;pr-initial=1;pr-log=1,100,120,120,c,c,c\n101|Actual merger|D|/src/.pull-requests/#7 Frozen||ev=pr-merged;pr=7;pr-author=Owner;pr-created=100;pr-state=open;pr-initial=0\n";
    let output = run_with_stdin(["convert", "--from", "custom", "-", "-"], input.as_bytes()).ok();
    let log = parse(output.stdout.as_bytes());
    assert_eq!(
        log.pr_changes().len(),
        2,
        "old codes can carry rich typed data"
    );
    assert_eq!(log.pr_coverage(), parse(input.as_bytes()).pr_coverage());
}
