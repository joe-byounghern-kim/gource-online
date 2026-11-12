// SPDX-License-Identifier: GPL-3.0-or-later

//! Play every scenario log for a few thousand frames at 1/60 s and check the
//! entity counts against what Gource does with the same input.
//!
//! How the expectations follow from the C++ (default settings: 10 s per
//! day, so a frame advances the clock by 144 s and every fixture is read in
//! its first couple of frames; files never idle out; users are deleted 10 s
//! after their last action):
//! - Deleted files are gone at the end and their emptied dirs are reaped,
//!   but the root node never is.
//! - The first file renames the empty root (`/` becomes its dir); a file
//!   outside the root forks a new root at the common prefix.
//! - `file-to-dir.log`: each `Not` file turns into a directory while still
//!   hidden (its create action has not been applied yet), so its forced
//!   removal starts with `fade_start == 0` and never completes. Gource keeps
//!   all four files (only the last one visible) and four dirs.
//! - `deletion-recreation.log`: actions touch a file when they start and
//!   remove it when they finish. Bob's create and modify start before
//!   Alice's remove finishes, so the remove wins and no file is left.
//! - `malformed-records.log`: `custom.cpp`'s regex accepts 6 lines (an
//!   empty user becomes `Unknown`, an empty action `A`, `src/has|delimiter`
//!   is cut at the `|`, a 5-digit colour is ignored).
//! - `utf8-caption.log` is a caption file, not a log: no commits.

mod common;

use std::path::Path;

use gource_core::settings::{CameraMode, OptionEffect, Settings};
use gource_core::view::{FontRole, SceneView};

const DT: f32 = 1.0 / 60.0;
const FRAMES: usize = 3000;

#[derive(Debug, PartialEq)]
struct Expect {
    commits: usize,
    final_files: usize,
    final_dirs: usize,
    max_files: usize,
    max_users: usize,
    max_dirs: usize,
}

fn expect(name: &str) -> Expect {
    let e = |commits, final_files, final_dirs, max_files, max_users, max_dirs| Expect {
        commits,
        final_files,
        final_dirs,
        max_files,
        max_users,
        max_dirs,
    };
    match name {
        "custom-dir-delete.log" => e(3, 0, 1, 4, 1, 2),
        "file-removal.log" => e(3, 0, 1, 5, 1, 1),
        "file-to-dir.log" => e(3, 4, 4, 4, 1, 4),
        "svn-dir-delete.log" => e(2, 0, 1, 5, 1, 1),
        "utf8-caption.log" => e(0, 0, 1, 0, 0, 1),
        "utf8.log" => e(2, 0, 1, 1, 1, 1),
        "cancellation.log" => e(3, 3, 1, 3, 3, 1),
        "deep-unicode-unusual.log" => e(5, 2, 2, 3, 2, 3),
        "deletion-recreation.log" => e(4, 0, 1, 1, 2, 1),
        "dense-contributors.log" => e(12, 4, 1, 6, 6, 1),
        "duplicates.log" => e(2, 1, 1, 1, 2, 1),
        "empty.log" => e(0, 0, 1, 0, 0, 1),
        "encoder-failure.log" => e(3, 2, 1, 2, 2, 1),
        "equal-timestamps-source-order.log" => e(5, 2, 3, 3, 2, 3),
        "long-idle.log" => e(4, 1, 1, 2, 2, 1),
        "malformed-records.log" => e(6, 5, 3, 5, 3, 3),
        "single-event.log" => e(1, 1, 1, 1, 1, 1),
        "unordered-timestamps.log" => e(5, 2, 2, 3, 3, 2),
        "visual-hierarchy-activity.log" => e(8, 4, 5, 5, 3, 6),
        "zero-negative-timestamps.log" => e(5, 2, 2, 3, 3, 2),
        "ai-coauthors.log" | "ai-coauthors-stripped.log" => e(12, 8, 4, 9, 5, 4),
        other => panic!("no expectation for {other}"),
    }
}

struct Run {
    expect: Expect,
    hashes: Vec<u64>,
    labels_seen: usize,
    finished: bool,
    final_users: usize,
}

fn play(path: &Path, settings: Settings, frames: usize) -> Run {
    let log = common::load_log(path);
    let mut sim = common::new_sim(settings);
    let mut view = SceneView::default();
    let (mut max_files, mut max_users, mut max_dirs) = (0, 0, 0);
    let mut hashes = Vec::new();
    let mut labels_seen = 0;
    for frame in 0..frames {
        sim.update(&log, DT);
        sim.extract_view(&log, &mut view);
        let s = sim.status(&log);
        max_files = max_files.max(s.files);
        max_users = max_users.max(s.users);
        max_dirs = max_dirs.max(s.dirs);
        labels_seen = labels_seen.max(view.labels.len());
        if frame % 50 == 0 {
            hashes.push(common::view_hash(&view));
        }
        // view invariants
        assert!(view.files.len() <= s.files);
        assert!(view.users.len() <= s.users);
        for (i, d) in view.dirs.iter().enumerate() {
            if let Some(p) = d.parent {
                assert!((p as usize) < i, "parents come first");
            }
        }
        for l in &view.labels {
            assert!((l.text.end as usize) <= view.text.len());
        }
    }
    let s = sim.status(&log);
    Run {
        expect: Expect {
            commits: log.commit_count(),
            final_files: s.files,
            final_dirs: s.dirs,
            max_files,
            max_users,
            max_dirs,
        },
        hashes,
        labels_seen,
        finished: s.finished,
        final_users: s.users,
    }
}

fn fixture(name: &str) -> std::path::PathBuf {
    common::repo_root()
        .join("tests/fixtures/custom-log")
        .join(name)
}

#[test]
fn every_scenario_log_plays_with_expected_counts() {
    for path in common::scenario_logs() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let run = play(&path, Settings::default(), FRAMES);
        assert_eq!(run.expect, expect(&name), "{name}");
        assert!(run.finished, "{name} should have finished");
        assert_eq!(run.final_users, 0, "{name}: idle users are deleted");
        if run.expect.commits > 0 {
            assert!(run.labels_seen > 0, "{name}: names are shown at some point");
        }
    }
}

#[test]
fn same_log_dt_and_seed_give_identical_frames() {
    for path in common::scenario_logs() {
        let a = play(&path, Settings::default(), 600);
        let b = play(&path, Settings::default(), 600);
        assert_eq!(a.hashes, b.hashes, "{}", path.display());
    }
    // A different hash seed changes colours (and therefore the hashes).
    let path = fixture("dense-contributors.log");
    let a = play(&path, Settings::default(), 300);
    let seeded = Settings {
        hash_seed: 1234,
        ..Settings::default()
    };
    let b = play(&path, seeded, 300);
    assert_ne!(a.hashes, b.hashes);
}

/// Settings variants that reach other code paths, all without panics.
#[test]
fn settings_variants_play_without_panics() {
    let variants: Vec<(&str, Settings)> = vec![
        (
            "hide everything",
            Settings {
                hide_date: true,
                hide_users: true,
                hide_tree: true,
                hide_files: true,
                hide_usernames: true,
                hide_filenames: true,
                hide_dirnames: true,
                hide_bloom: true,
                hide_root: true,
                hide_mouse: true,
                hide_progress: true,
                ..Settings::default()
            },
        ),
        (
            "track camera, highlights, extensions",
            Settings {
                camera_mode: CameraMode::Track,
                highlight_all_users: true,
                highlight_dirs: true,
                file_extensions: true,
                fixed_user_size: true,
                show_key: true,
                title: "title".into(),
                ..Settings::default()
            },
        ),
        (
            "elastic, idle files, slow clock",
            Settings {
                elasticity: 0.5,
                file_idle_time: 2.0,
                days_per_second: (600.0f64 / 86400.0) as f32,
                user_scale: 2.0,
                max_file_lag: -1.0,
                auto_skip_seconds: -1.0,
                ..Settings::default()
            },
        ),
        (
            "no time travel, crop, follow, max files",
            Settings {
                no_time_travel: true,
                crop_vertical: true,
                follow_users: vec!["alice".into(), "bob".into()],
                highlight_users: vec!["carol".into()],
                max_files: 2,
                loop_playback: true,
                ..Settings::default()
            },
        ),
    ];
    for path in common::scenario_logs() {
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        for (label, settings) in &variants {
            let run = play(&path, settings.clone(), 1200);
            assert!(
                run.expect.max_files <= expect(&name).max_files,
                "{label}: {name}"
            );
            if settings.max_files > 0 {
                assert!(run.expect.max_files <= 2, "{label}: {name}");
            }
        }
    }
}

#[test]
fn hidden_elements_are_not_in_the_view() {
    let log = common::load_log(&fixture("visual-hierarchy-activity.log"));
    let settings = Settings {
        hide_users: true,
        hide_files: true,
        hide_dirnames: true,
        hide_date: true,
        hide_tree: true,
        hide_bloom: true,
        ..Settings::default()
    };
    let mut sim = common::new_sim(settings);
    let mut view = SceneView::default();
    common::run(&mut sim, &log, &mut view, 120, DT);
    assert!(!view.dirs.is_empty());
    assert!(view.files.is_empty());
    assert!(view.users.is_empty());
    assert!(view.actions.is_empty());
    assert!(view.labels.is_empty());
    assert!(view.hud.date.is_none());
    assert!(view.dirs.iter().all(|d| !d.draw_edge && !d.draw_bloom));

    let mut sim = common::new_sim(Settings::default());
    common::run(&mut sim, &log, &mut view, 120, DT);
    assert!(!view.files.is_empty());
    assert!(!view.users.is_empty());
    assert!(view.dirs.iter().any(|d| d.draw_edge));
    assert!(view.dirs.iter().any(|d| d.draw_bloom));
    assert!(view.hud.date.is_some());
    assert!(view.labels.iter().any(|l| l.font == FontRole::UserName));
    assert!(view.labels.iter().any(|l| l.font == FontRole::FileName));
    assert!(view.labels.iter().any(|l| l.font == FontRole::DirName));
    // The root has no edge and no name; every other dir follows its parent.
    assert_eq!(view.dirs[0].parent, None);
    assert!(!view.dirs[0].draw_edge);
    // FileKey: "rs" and "md"
    let exts: Vec<&str> = view
        .hud
        .extensions
        .iter()
        .map(|e| view.str(e.ext))
        .collect();
    assert_eq!(exts, vec!["md", "rs"]);
}

#[test]
fn restart_reproduces_a_fresh_simulation() {
    let log = common::load_log(&fixture("dense-contributors.log"));
    let mut view = SceneView::default();

    let mut fresh = common::new_sim(Settings::default());
    common::run(&mut fresh, &log, &mut view, 200, DT);
    let expected = common::view_hash(&view);

    let mut sim = common::new_sim(Settings::default());
    common::run(&mut sim, &log, &mut view, 77, DT);
    sim.restart(&log);
    common::run(&mut sim, &log, &mut view, 200, DT);
    assert_eq!(common::view_hash(&view), expected);
}

#[test]
fn filters_and_date_range_rebuild_the_scene() {
    let log = common::load_log(&fixture("visual-hierarchy-activity.log"));
    let mut view = SceneView::default();
    let mut sim = common::new_sim(Settings::default());
    common::run(&mut sim, &log, &mut view, 600, DT);
    assert_eq!(sim.status(&log).files, 4);

    assert_eq!(
        sim.set_option("file-filter", r"\.md$").unwrap(),
        OptionEffect::Restart
    );
    sim.restart(&log);
    common::run(&mut sim, &log, &mut view, 600, DT);
    assert_eq!(sim.status(&log).files, 3);

    sim.set_option("file-filter", "").unwrap();
    sim.set_option("user-show-filter", "Ada").unwrap();
    sim.restart(&log);
    common::run(&mut sim, &log, &mut view, 600, DT);
    // Ada: src/main.rs and docs/README.md
    assert_eq!(sim.status(&log).files, 2);

    sim.set_option("user-show-filter", "").unwrap();
    // 1700001063 is 2023-11-14 22:31:03 UTC: only the last three commits.
    sim.set_option("start-date", "2023-11-14 22:31:00Z")
        .unwrap();
    sim.restart(&log);
    common::run(&mut sim, &log, &mut view, 600, DT);
    // docs/README.md, src/main.rs (modified, so created), tools/bench/driver.rs
    assert_eq!(sim.status(&log).files, 3);

    sim.set_option("start-date", "").unwrap();
    sim.set_option("stop-date", "2023-11-14 22:30:00Z").unwrap();
    sim.restart(&log);
    common::run(&mut sim, &log, &mut view, 600, DT);
    // src/main.rs and tests/unit/parser_test.rs (parser.rs was deleted)
    assert_eq!(sim.status(&log).files, 2);
    assert!(sim.status(&log).finished);
}
