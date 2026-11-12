// SPDX-License-Identifier: GPL-3.0-or-later

//! Approved PR event and rich metadata regressions, independent of simulation.

use gource_core::events::ChangeEvent;
use gource_sources::meta;

#[test]
fn six_new_event_codes_follow_the_unchanged_legacy_codes() {
    assert_eq!(ChangeEvent::Commit.code(), 0);
    assert_eq!(ChangeEvent::PrOpened.code(), 1);
    assert_eq!(ChangeEvent::PrMerged.code(), 2);
    assert_eq!(ChangeEvent::PrClosed.code(), 3);
    for (name, code) in [
        ("review-approved", 4),
        ("review-changes-requested", 5),
        ("review-commented", 6),
        ("pr-reopened", 7),
        ("pr-ready", 8),
        ("pr-review-requested", 9),
    ] {
        let event = ChangeEvent::from_meta(name).expect("approved event kind must be supported");
        assert_eq!(event.code(), code);
        assert_eq!(ChangeEvent::from_code(code), Some(event));
        assert_eq!(event.as_meta(), Some(name));
    }
    assert_eq!(ChangeEvent::from_code(10), None);
}

#[test]
fn malformed_mandatory_rich_metadata_is_not_classified_as_a_lifecycle_event() {
    for field in [
        "ev=pr-opened;pr=42;pr-author=alice",
        "ev=pr-opened;pr=0;pr-author=alice;pr-created=1;pr-state=open;pr-initial=0",
        "ev=pr-opened;pr=42;pr-author=;pr-created=1;pr-state=open;pr-initial=0",
        "ev=pr-merged;pr=42;pr-author=alice;pr-created=bad;pr-state=open;pr-initial=0",
        "ev=pr-opened;pr=42;pr-author=alice;pr-created=1;pr-state=future;pr-initial=0",
        "ev=pr-opened;pr=42;pr-author=alice;pr-created=1;pr-state=open;pr-initial=2",
        "ev=pr-opened;pr=42;pr=43;pr-author=alice;pr-created=1;pr-state=open;pr-initial=0",
    ] {
        assert_eq!(meta::parse_event(field.as_bytes()), ChangeEvent::Commit);
    }
}

#[test]
fn legacy_ev_only_metadata_retains_l0_semantics() {
    assert_eq!(meta::parse_event(b"ev=pr-opened"), ChangeEvent::PrOpened);
    assert_eq!(meta::parse_event(b"ev=pr-merged"), ChangeEvent::PrMerged);
    assert_eq!(meta::parse_event(b"ev=pr-closed"), ChangeEvent::PrClosed);
    assert_eq!(meta::parse_event(b"ev=future-event"), ChangeEvent::Commit);
}
