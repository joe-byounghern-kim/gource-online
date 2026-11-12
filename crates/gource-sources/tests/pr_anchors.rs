// SPDX-License-Identifier: GPL-3.0-or-later

use gource_core::events::{ChangeEvent, ChangeKind, EventLog, PrCompleteness};
use gource_sources::rich_pulls::{
    AnchorPlacement, FrozenPrAnchors, MAX_CHANGED_PATHS, MAX_CODE_PATHS, MAX_NODE_PATH_BYTES,
    MAX_PATH_BYTES, MAX_PULL_REQUESTS, MAX_TITLE_INPUT_BYTES, MAX_TOTAL_CHANGED_PATHS,
    PrAnchorError, PrAnchorInput,
};
use gource_sources::{CustomLogParser, EventLogSink, pulls};

fn input<'a>(number: u64, title: &'a str, changed_paths: &'a [&'a str]) -> PrAnchorInput<'a> {
    PrAnchorInput {
        number,
        title,
        changed_paths,
        completeness: PrCompleteness::Complete,
    }
}

#[test]
fn deepest_strict_majority_counts_all_directory_ancestors() {
    let paths = [
        "src/a/deep/x.rs",
        "src/a/deep/y.rs",
        "src/a/z.rs",
        "tests/t.rs",
    ];
    let anchors = FrozenPrAnchors::new(&[input(12, "Deep change", &paths)], &paths).unwrap();
    let pr = anchors.get(12).unwrap();
    // Exactly half in deep is not a majority, but three of four live in a.
    assert_eq!(pr.anchor, "/src/a");
    assert_eq!(pr.placement, AnchorPlacement::StrictMajority);
    assert_eq!(pr.path, "/src/a/.pull-requests/#12 Deep change");
}

#[test]
fn nested_majority_can_ignore_minority_in_another_top_level_directory() {
    let paths = ["src/a/x", "src/a/y", "tests/t"];
    let anchors = FrozenPrAnchors::new(&[input(1, "", &paths)], &[]).unwrap();
    assert_eq!(anchors.get(1).unwrap().anchor, "/src/a");
    assert_eq!(anchors.get(1).unwrap().path, "/src/a/.pull-requests/#1");
}

#[test]
fn sibling_tie_uses_shared_directory_ancestor_not_either_child() {
    let paths = ["src/a/x", "src/b/y"];
    let anchors = FrozenPrAnchors::new(&[input(2, "Tie", &paths)], &[]).unwrap();
    assert_eq!(anchors.get(2).unwrap().anchor, "/src");
    assert_eq!(anchors.get(2).unwrap().path, "/src/.pull-requests/#2 Tie");
}

#[test]
fn top_level_tie_and_root_files_use_region_fallback() {
    for paths in [&["src/x", "tests/y"][..], &["README.md"][..]] {
        let anchors = FrozenPrAnchors::new(&[input(3, "Root", paths)], &[]).unwrap();
        let pr = anchors.get(3).unwrap();
        assert_eq!(pr.anchor, "/pull requests");
        assert_eq!(pr.placement, AnchorPlacement::RootFallback);
        assert_eq!(pr.path, "/pull requests/#3 Root");
    }
}

#[test]
fn incomplete_unknown_and_empty_data_never_guess_a_code_area() {
    for (completeness, placement) in [
        (PrCompleteness::Partial, AnchorPlacement::IncompleteFallback),
        (PrCompleteness::Unknown, AnchorPlacement::UnknownFallback),
    ] {
        let pr = PrAnchorInput {
            completeness,
            ..input(4, "Gap", &["src/x"])
        };
        let anchors = FrozenPrAnchors::new(&[pr], &[]).unwrap();
        assert_eq!(anchors.get(4).unwrap().path, "/pull requests/#4 Gap");
        assert_eq!(anchors.get(4).unwrap().placement, placement);
    }
    let anchors = FrozenPrAnchors::new(&[input(4, "Gap", &[])], &[]).unwrap();
    assert_eq!(
        anchors.get(4).unwrap().placement,
        AnchorPlacement::UnknownFallback
    );
}

#[test]
fn normalized_duplicates_cannot_manufacture_a_majority() {
    let paths = ["src/a/x", "/src/a/x", "//src/a/x", "other/y"];
    let anchors = FrozenPrAnchors::new(&[input(5, "Dedup", &paths)], &[]).unwrap();
    assert_eq!(anchors.get(5).unwrap().path, "/pull requests/#5 Dedup");
}

#[test]
fn unicode_titles_are_delimiter_path_and_control_safe() {
    let anchors = FrozenPrAnchors::new(
        &[input(
            6,
            "  日本|語/\n\\\t\0\u{202e}\u{2028} title  ",
            &["src/x"],
        )],
        &[],
    )
    .unwrap();
    let pr = anchors.get(6).unwrap();
    assert_eq!(pr.title, "日本-語-- title");
    assert_eq!(pr.path, "/src/.pull-requests/#6 日本-語-- title");
    assert!(
        !pr.title
            .chars()
            .any(|c| c.is_control() || matches!(c, '|' | '/' | '\\'))
    );
}

#[test]
fn title_character_limit_includes_ellipsis_and_never_splits_unicode() {
    let title = "🦀".repeat(61);
    let anchors = FrozenPrAnchors::new(&[input(7, &title, &["src/x"])], &[]).unwrap();
    assert_eq!(
        anchors.get(7).unwrap().title,
        format!("{}…", "🦀".repeat(59))
    );
    assert_eq!(
        anchors.get(7).unwrap().title.chars().count(),
        pulls::MAX_TITLE_CHARS
    );
    let exact = "界".repeat(60);
    let anchors = FrozenPrAnchors::new(&[input(7, &exact, &["src/x"])], &[]).unwrap();
    assert_eq!(anchors.get(7).unwrap().title, exact);
}

#[test]
fn blank_or_only_unsafe_title_does_not_add_a_trailing_space() {
    let anchors = FrozenPrAnchors::new(&[input(u64::MAX, "\n\0\t", &["src/x"])], &[]).unwrap();
    assert_eq!(
        anchors.get(u64::MAX).unwrap().path,
        format!("/src/.pull-requests/#{}", u64::MAX)
    );
    assert_eq!(anchors.get(u64::MAX).unwrap().title, "");
}

#[test]
fn collisions_include_future_code_even_when_not_in_changed_paths() {
    let all_code = [
        "src/x",
        "src/.pull-requests/future.rs",
        "src/.pull-requests-2",
        "src/.pull-requests-3/#8 Change/real.rs",
    ];
    let anchors = FrozenPrAnchors::new(&[input(8, "Change", &["src/x"])], &all_code).unwrap();
    assert_eq!(
        anchors.get(8).unwrap().path,
        "/src/.pull-requests-4/#8 Change"
    );
}

#[test]
fn exact_files_directory_prefixes_and_directory_deletes_reserve_synthetic_directories() {
    for collision in [
        "src/.pull-requests",
        "src/.pull-requests/",
        "src/.pull-requests/#9 Exact",
        "src/.pull-requests/#9 Exact/child",
        "src/.pull-requests/different-file",
    ] {
        let anchors = FrozenPrAnchors::new(&[input(9, "Exact", &["src/x"])], &[collision]).unwrap();
        assert_eq!(
            anchors.get(9).unwrap().path,
            "/src/.pull-requests-2/#9 Exact"
        );
    }
}

#[test]
fn string_prefix_without_a_separator_is_not_a_collision() {
    let code = ["src/.pull-requests-extra/f", "src2/.pull-requests/f"];
    let anchors = FrozenPrAnchors::new(&[input(10, "Prefix", &["src/x"])], &code).unwrap();
    assert_eq!(
        anchors.get(10).unwrap().path,
        "/src/.pull-requests/#10 Prefix"
    );
}

#[test]
fn real_file_at_or_above_anchor_degrades_to_region_not_an_infinite_suffix_loop() {
    for code_file in ["src", "src/a"] {
        let anchors =
            FrozenPrAnchors::new(&[input(11, "Blocked", &["src/a/x"])], &[code_file]).unwrap();
        assert_eq!(anchors.get(11).unwrap().path, "/pull requests/#11 Blocked");
        assert_eq!(
            anchors.get(11).unwrap().placement,
            AnchorPlacement::CodeFileFallback
        );
    }
}

#[test]
fn ordinary_directory_delete_ancestors_do_not_block_typed_pr_anchors() {
    let anchors = FrozenPrAnchors::new(
        &[input(12, "Kept", &["src/a/x"])],
        &["/", "src/", "/src/a/"],
    )
    .unwrap();
    assert_eq!(
        anchors.get(12).unwrap().path,
        "/src/a/.pull-requests/#12 Kept"
    );
}

#[test]
fn fallback_region_itself_uses_deterministic_unused_suffixes() {
    let code = [
        "pull requests",
        "pull requests-2/#13 Region/nested",
        "pull requests-3/",
    ];
    let anchors = FrozenPrAnchors::new(&[input(13, "Region", &["root-file"])], &code).unwrap();
    assert_eq!(anchors.get(13).unwrap().anchor, "/pull requests-4");
    assert_eq!(anchors.get(13).unwrap().path, "/pull requests-4/#13 Region");
}

#[test]
fn allocation_is_independent_of_pr_and_code_path_input_order() {
    let a = input(21, "A", &["src/x", "src/y"]);
    let b = input(20, "B", &["src/y", "src/x"]);
    let left = FrozenPrAnchors::new(&[a, b], &["src/.pull-requests/f", "src/y"]).unwrap();
    let right = FrozenPrAnchors::new(&[b, a], &["src/y", "src/.pull-requests/f"]).unwrap();
    assert_eq!(left, right);
    assert_eq!(left.len(), 2);
    assert!(!left.is_empty());
    assert!(left.get(99).is_none());
    assert!(FrozenPrAnchors::new(&[], &[]).unwrap().is_empty());
}

#[test]
fn title_and_anchor_are_owned_and_frozen_across_external_edits_and_reopen_lookups() {
    let mut title = String::from("Original");
    let mut changed = String::from("src/original/x");
    let anchors = FrozenPrAnchors::new(&[input(30, &title, &[&changed])], &[]).unwrap();
    let opened = anchors.get(30).unwrap().path.clone();
    title.clear();
    title.push_str("Edited");
    changed.clear();
    changed.push_str("tests/new/x");
    assert_eq!(anchors.get(30).unwrap().title, "Original");
    assert_eq!(anchors.get(30).unwrap().anchor, "/src/original");
    assert_eq!(anchors.get(30).unwrap().path, opened);
    let refreshed = FrozenPrAnchors::new(&[input(30, &title, &[&changed])], &[]).unwrap();
    assert_eq!(
        refreshed.get(30).unwrap().path,
        "/tests/new/.pull-requests/#30 Edited"
    );
}

#[test]
fn allocator_does_not_mutate_code_log_or_change_legacy_l0_defaults() {
    let bytes = b"1|code-owner|A|/src/a.rs\n2|code-owner|M|/src/a.rs\n";
    let mut log = EventLog::new();
    let mut parser = CustomLogParser::new();
    let mut sink = EventLogSink::new(&mut log);
    parser.push(bytes, &mut sink);
    parser.finish(&mut sink);
    let before = format!("{log:?}");
    let code = vec!["/src/a.rs"];
    let anchors = FrozenPrAnchors::new(&[input(31, "R|P", &code)], &code).unwrap();
    assert_eq!(format!("{log:?}"), before);
    assert_eq!(code, ["/src/a.rs"]);
    assert_eq!(log.commit(0).changes[0].event, ChangeEvent::Commit);
    assert_eq!(log.commit(0).changes[0].kind, ChangeKind::Add);
    assert_eq!(log.commit(1).changes[0].kind, ChangeKind::Modify);
    assert_eq!(pulls::node_path(31, "R|P"), "/pull requests/#31 R|P");
    assert_eq!(anchors.get(31).unwrap().path, "/src/.pull-requests/#31 R-P");
}

#[test]
fn invalid_numbers_duplicates_and_unsafe_paths_have_typed_safe_errors() {
    assert_eq!(
        FrozenPrAnchors::new(&[input(0, "", &[])], &[]),
        Err(PrAnchorError::InvalidNumber)
    );
    assert_eq!(
        FrozenPrAnchors::new(&[input(2, "One", &[]), input(2, "Edited", &[])], &[]),
        Err(PrAnchorError::DuplicateNumber { number: 2 })
    );
    for path in [
        "",
        "/",
        "src/",
        "src//x",
        "src/../x",
        "./x",
        "src\\x",
        "src/secret|token",
        "src/secret\nbody",
    ] {
        let error = FrozenPrAnchors::new(&[input(3, "", &[path])], &[]).unwrap_err();
        assert_eq!(
            error,
            PrAnchorError::InvalidChangedPath {
                number: 3,
                index: 0
            }
        );
        assert!(!format!("{error:?}: {error}").contains("secret"));
    }
    assert_eq!(
        FrozenPrAnchors::new(&[], &["code/../token"]),
        Err(PrAnchorError::InvalidCodePath { index: 0 })
    );
}

#[test]
fn documented_input_and_output_bounds_reject_overflow_before_allocation() {
    let path = "x".repeat(MAX_PATH_BYTES + 1);
    assert_eq!(
        FrozenPrAnchors::new(&[input(1, "", &[&path])], &[]),
        Err(PrAnchorError::InvalidChangedPath {
            number: 1,
            index: 0
        })
    );
    let title = "t".repeat(MAX_TITLE_INPUT_BYTES + 1);
    assert_eq!(
        FrozenPrAnchors::new(&[input(1, &title, &[])], &[]),
        Err(PrAnchorError::TitleTooLong { number: 1 })
    );
    let code = vec!["x"; MAX_CODE_PATHS + 1];
    assert_eq!(
        FrozenPrAnchors::new(&[], &code),
        Err(PrAnchorError::TooManyCodePaths)
    );
    let changed = vec!["x"; MAX_CHANGED_PATHS + 1];
    assert_eq!(
        FrozenPrAnchors::new(&[input(1, "", &changed)], &[]),
        Err(PrAnchorError::TooManyChangedPaths { number: 1 })
    );
    let prs = vec![input(1, "", &[]); MAX_PULL_REQUESTS + 1];
    assert_eq!(
        FrozenPrAnchors::new(&prs, &[]),
        Err(PrAnchorError::TooManyPullRequests)
    );
    let directory = "d".repeat(MAX_NODE_PATH_BYTES);
    let long_anchor_file = format!("{directory}/x");
    assert_eq!(
        FrozenPrAnchors::new(&[input(1, "", &[&long_anchor_file])], &[]),
        Err(PrAnchorError::NodePathTooLong { number: 1 })
    );
}

#[test]
fn aggregate_changed_path_budget_is_checked_before_deduplication() {
    let changed = vec!["src/x"; MAX_CHANGED_PATHS];
    let count = MAX_TOTAL_CHANGED_PATHS / MAX_CHANGED_PATHS + 1;
    let prs: Vec<_> = (1..=count)
        .map(|number| input(number as u64, "", &changed))
        .collect();
    assert_eq!(
        FrozenPrAnchors::new(&prs, &[]),
        Err(PrAnchorError::TooManyTotalChangedPaths)
    );
}

#[test]
fn maximum_frozen_node_path_is_accepted_by_typed_pr_labels() {
    let tail = "/.pull-requests/#1";
    let directory = "d".repeat(MAX_NODE_PATH_BYTES - 1 - tail.len());
    let file = format!("{directory}/x");
    let anchors = FrozenPrAnchors::new(&[input(1, "", &[&file])], &[]).unwrap();
    let path = &anchors.get(1).unwrap().path;
    assert_eq!(path.len(), MAX_NODE_PATH_BYTES);
    assert!(gource_core::events::is_pr_label(path));
}

#[test]
fn bounded_suffix_search_skips_many_adversarially_occupied_directories() {
    let code: Vec<String> = (1..=150)
        .map(|i| {
            if i == 1 {
                "src/.pull-requests/f".to_string()
            } else {
                format!("src/.pull-requests-{i}/f")
            }
        })
        .collect();
    let paths: Vec<&str> = code.iter().map(String::as_str).collect();
    let anchors = FrozenPrAnchors::new(&[input(32, "Many", &["src/x"])], &paths).unwrap();
    assert_eq!(
        anchors.get(32).unwrap().path,
        "/src/.pull-requests-151/#32 Many"
    );
}
