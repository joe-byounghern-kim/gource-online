// SPDX-License-Identifier: GPL-3.0-or-later

//! Simulation internals: the C++ golden comparison and unit tests of the
//! `Gource::logic` rules.

use glam::Vec2;

use super::*;
use crate::events::{ChangeEvent, ChangeKind, EventLog, FileChange};
use crate::input::{InputEvent, Key, PointerButton};
use crate::view::{FontRole, SceneView};

const GOLDEN: &str = include_str!("../../tests/golden/sim_golden.tsv");
const GOLDEN_GLIBC: &str = include_str!("../../tests/golden/sim_golden_glibc.tsv");
const SCENARIO: &str = include_str!("../../tests/golden/sim/scenario.log");

/// The scenario as the C++ harness reads it: `ts|user|action|path[|RRGGBB]`,
/// consecutive lines with the same timestamp and user form a commit.
fn scenario_log(text: &str) -> EventLog {
    let mut log = EventLog::new();
    let mut offset = 0u64;
    let mut current: Option<(i64, String, Vec<FileChange>, u64)> = None;
    for line in text.lines() {
        let line_offset = offset;
        offset += line.len() as u64 + 1;
        let f: Vec<&str> = line.split('|').collect();
        let ts: i64 = f[0].parse().unwrap();
        if current
            .as_ref()
            .is_some_and(|(t, u, _, _)| *t != ts || u != f[1])
        {
            let (t, u, changes, off) = current.take().unwrap();
            let uid = log.intern(&u);
            log.push_commit(t, uid, changes, off);
        }
        if current.is_none() {
            current = Some((ts, f[1].to_string(), Vec::new(), line_offset));
        }
        let path = format!("/{}", f[3]);
        let colour = f.get(4).filter(|c| c.len() == 6).map(|c| {
            let v = |i: usize| u8::from_str_radix(&c[i..i + 2], 16).unwrap();
            [v(0), v(2), v(4)]
        });
        let change = FileChange {
            path: log.intern(&path),
            kind: ChangeKind::from_action(f[2]),
            colour,
            event: ChangeEvent::Commit,
        };
        current.as_mut().unwrap().2.push(change);
    }
    if let Some((t, u, changes, off)) = current {
        let uid = log.intern(&u);
        log.push_commit(t, uid, changes, off);
    }
    log.set_total_bytes(offset);
    log.set_complete(true);
    log
}

fn hex(f: f32) -> String {
    format!("{:08x}", f.to_bits())
}

/// The rows `sim_golden.cpp`'s `snapshot` prints.
fn snapshot(sim: &Simulation, frame: usize, out: &mut Vec<String>) {
    let cp = sim.camera.pos;
    out.push(format!(
        "{frame}\tclock\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        sim.currtime,
        sim.lasttime,
        hex(sim.subseconds),
        hex(sim.idle_time),
        hex(cp.x),
        hex(cp.y),
        hex(cp.z),
        hex(sim.rotation_remaining_angle),
        sim.commit_queue.len(),
        sim.cursor
    ));
    for (name, &u) in &sim.user_map {
        let user = &sim.users[u];
        out.push(format!(
            "{frame}\tuser\t{name}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            hex(user.pawn.pos.x),
            hex(user.pawn.pos.y),
            hex(user.pawn.accel.x),
            hex(user.pawn.accel.y),
            hex(user.alpha(&sim.settings)),
            hex(user.colour().x),
            hex(user.action_interval),
            user.actions.len(),
            user.active_actions.len(),
            user.action_count,
            user.active_count
        ));
    }
    if !frame.is_multiple_of(100) {
        return;
    }
    for (path, &d) in &sim.tree.dir_map {
        let n = &sim.tree.dirs[d];
        out.push(format!(
            "{frame}\tdir\t{path}\t{}\t{}\t{}\t{}\t{}",
            hex(n.pos.x),
            hex(n.pos.y),
            hex(n.spos.x),
            hex(n.dir_radius),
            hex(n.col.x)
        ));
    }
    for (path, &f) in &sim.tree.file_map {
        let file = &sim.tree.files[f];
        let p = sim.tree.file_abs_pos(f);
        let c = file.colour();
        out.push(format!(
            "{frame}\tfile\t{path}\t{}\t{}\t{}\t{}\t{}\t{}",
            hex(p.x),
            hex(p.y),
            hex(file.alpha()),
            hex(c.x),
            hex(c.z),
            file.is_hidden() as i32
        ));
    }
}

fn new_sim() -> Simulation {
    let mut sim = Simulation::new(Settings::default());
    sim.set_viewport(1280, 720, 1.0);
    sim
}

/// Replays the golden scenario for 2400 frames and compares every
/// snapshot row with `golden`.
fn assert_matches_golden(mut sim: Simulation, golden: &str) {
    let log = scenario_log(SCENARIO);
    let mut rows = Vec::new();
    for frame in 1..=2400 {
        sim.update(&log, 1.0 / 60.0);
        if frame % 20 == 0 {
            snapshot(&sim, frame, &mut rows);
        }
    }

    let golden: Vec<&str> = golden.lines().filter(|l| !l.starts_with('#')).collect();
    assert!(golden.len() > 2000, "golden table too small");
    for (i, (got, want)) in rows.iter().zip(golden.iter()).enumerate() {
        assert_eq!(got, want, "row {i} differs");
    }
    assert_eq!(rows.len(), golden.len());
    // The scenario reaches an auto-rotation.
    assert!(golden.iter().any(|r| {
        let c: Vec<&str> = r.split('\t').collect();
        c[1] == "clock" && c[9] != "00000000"
    }));
}

/// Users, actions, the camera (incl. auto-rotation), the clock, auto-skip,
/// time travel, directory deletes and the tree, frame by frame against
/// Gource's own user.cpp / action.cpp / zoomcamera.cpp / dirnode.cpp driven
/// by a transcription of gource.cpp's logic (tests/golden/sim).
#[test]
fn simulation_matches_cpp_golden() {
    assert_matches_golden(new_sim(), GOLDEN);
}

/// The same scenario with `rand-mode glibc` against the harness built with
/// glibc's own `rand()`: every former `rand()` site draws from one global
/// generator in the C++ call order.
#[test]
fn simulation_matches_cpp_golden_with_glibc_rand() {
    let mut settings = Settings::default();
    settings.apply_option("rand-mode", "glibc").unwrap();
    let mut sim = Simulation::new(settings);
    sim.set_viewport(1280, 720, 1.0);
    assert_matches_golden(sim, GOLDEN_GLIBC);
}

/// `rand-mode` takes effect on restart, and switching back gives the keyed
/// run again.
#[test]
fn rand_mode_applies_on_restart() {
    let log = scenario_log(SCENARIO);
    let run = |sim: &mut Simulation| {
        for _ in 0..600 {
            sim.update(&log, 1.0 / 60.0);
        }
        sim.users
            .iter()
            .map(|(_, u)| (u.pawn.pos.x.to_bits(), u.pawn.pos.y.to_bits()))
            .collect::<Vec<_>>()
    };
    let mut sim = new_sim();
    let keyed = run(&mut sim);
    assert_eq!(
        sim.set_option("rand-mode", "glibc").unwrap(),
        OptionEffect::Restart
    );
    sim.restart(&log);
    let glibc = run(&mut sim);
    assert_ne!(keyed, glibc);
    sim.set_option("rand-mode", "keyed").unwrap();
    sim.restart(&log);
    assert_eq!(run(&mut sim), keyed);
}

fn change(log: &mut EventLog, path: &str, kind: ChangeKind) -> FileChange {
    FileChange {
        path: log.intern(path),
        kind,
        colour: None,
        event: ChangeEvent::Commit,
    }
}

type Spec<'a> = (i64, &'a str, &'a [(&'a str, ChangeKind)]);

/// One commit per entry: (timestamp, user, [(path, kind)]).
fn make_log(commits: &[Spec]) -> EventLog {
    let mut log = EventLog::new();
    for (i, (ts, user, changes)) in commits.iter().enumerate() {
        let u = log.intern(user);
        let cs: Vec<FileChange> = changes
            .iter()
            .map(|(p, k)| change(&mut log, p, *k))
            .collect();
        log.push_commit(*ts, u, cs, i as u64 * 10);
    }
    log.set_total_bytes(commits.len() as u64 * 10);
    log.set_complete(true);
    log
}

const A: ChangeKind = ChangeKind::Add;
const M: ChangeKind = ChangeKind::Modify;
const D: ChangeKind = ChangeKind::Delete;

fn key(c: char) -> InputEvent {
    InputEvent::Key {
        key: Key::Char(c),
        pressed: true,
        shift: false,
        ctrl: false,
    }
}

fn special(k: Key) -> InputEvent {
    InputEvent::Key {
        key: k,
        pressed: true,
        shift: false,
        ctrl: false,
    }
}

#[test]
fn commit_queue_holds_at_most_100_commits_ahead() {
    let mut log = EventLog::new();
    let u = log.intern("u");
    for i in 0..300 {
        let c = change(&mut log, &format!("/f{i}.c"), A);
        // all at the same second: everything is "due"
        log.push_commit(1000, u, [c], i as u64);
    }
    log.set_total_bytes(300);
    log.set_complete(true);

    let mut sim = new_sim();
    // The first readLog runs before currtime is set (0), so it stops after
    // one commit.
    sim.update(&log, 1.0 / 60.0);
    assert_eq!(sim.cursor, 1);
    assert_eq!(sim.tree.file_count(), 1);
    // Then 100 are queued and all processed in the same frame (all due).
    sim.update(&log, 1.0 / 60.0);
    assert_eq!(sim.cursor, 101);
    assert!(sim.commit_queue.is_empty());
    assert_eq!(sim.tree.file_count(), 101);
    sim.update(&log, 1.0 / 60.0);
    assert_eq!(sim.cursor, 201);
}

#[test]
fn auto_skip_jumps_to_the_next_commit_after_three_idle_seconds() {
    let log = make_log(&[
        (1_000_000, "a", &[("/x.c", A)]),
        // 100 days later: at 10 s per day, 1000 s away without auto-skip
        (1_000_000 + 86400 * 100, "a", &[("/y.c", A)]),
    ]);
    let mut sim = new_sim();
    let mut frames = 0;
    while sim.tree.file_count() < 2 {
        sim.update(&log, 1.0 / 60.0);
        frames += 1;
        assert!(frames < 60 * 30, "auto-skip did not happen");
    }
    assert_eq!(sim.lasttime, 1_000_000 + 86400 * 100);

    // With auto-skip disabled the second commit is still far away.
    let mut sim = Simulation::new(Settings {
        auto_skip_seconds: -1.0,
        ..Settings::default()
    });
    for _ in 0..60 * 30 {
        sim.update(&log, 1.0 / 60.0);
    }
    assert_eq!(sim.tree.file_count(), 1);
}

#[test]
fn time_travel_rewinds_unless_disabled() {
    let log = make_log(&[
        (2_000_000, "a", &[("/x.c", A)]),
        (1_000_000, "a", &[("/y.c", A)]),
    ]);
    let mut sim = new_sim();
    sim.update(&log, 1.0 / 60.0);
    sim.update(&log, 1.0 / 60.0);
    // the older commit is due immediately and rewinds the clock
    assert_eq!(sim.tree.file_count(), 2);
    assert!(sim.currtime < 2_000_000);
    assert_eq!(sim.lasttime, 1_000_000);

    let mut sim = Simulation::new(Settings {
        no_time_travel: true,
        ..Settings::default()
    });
    sim.update(&log, 1.0 / 60.0);
    sim.update(&log, 1.0 / 60.0);
    assert_eq!(sim.tree.file_count(), 2);
    assert!(sim.currtime >= 2_000_000);
    assert_eq!(sim.lasttime, 2_000_000);
}

#[test]
fn users_are_deleted_ten_seconds_after_their_last_action() {
    let log = make_log(&[(1000, "a", &[("/x.c", A)])]);
    let mut sim = new_sim();
    let mut gone_at = None;
    for frame in 0..60 * 20 {
        sim.update(&log, 1.0 / 60.0);
        if sim.user_map.is_empty() && gone_at.is_none() && frame > 0 {
            gone_at = Some(frame);
        }
    }
    let gone_at = gone_at.expect("user deleted") as f32 / 60.0;
    // ~2 s of work, then 10 s idle
    assert!((11.0..14.0).contains(&gone_at), "{gone_at}");
    assert_eq!(sim.tree.file_count(), 1);
}

#[test]
fn file_key_counts_extensions_and_follows_deletes() {
    let log = make_log(&[
        (
            1000,
            "a",
            &[("/x.c", A), ("/y.c", A), ("/z.h", A), ("/README", A)],
        ),
        (1001, "a", &[("/y.c", D)]),
    ]);
    let mut sim = new_sim();
    for _ in 0..60 * 10 {
        sim.update(&log, 1.0 / 60.0);
    }
    let counts: Vec<(&str, i32)> = sim
        .file_key
        .iter()
        .map(|(e, k)| (e.as_str(), k.count))
        .collect();
    assert_eq!(counts, vec![("", 1), ("c", 1), ("h", 1)]);
    assert_eq!(sim.file_key["c"].colour, colour_hash(b"c", 31));

    let mut view = SceneView::default();
    sim.extract_view(&log, &mut view);
    let exts: Vec<(&str, u32)> = view
        .hud
        .extensions
        .iter()
        .map(|e| (view.str(e.ext), e.count))
        .collect();
    assert_eq!(exts, vec![("", 1), ("c", 1), ("h", 1)]);
}

#[test]
fn faded_key_entries_are_dropped_and_recreated_with_the_new_colour() {
    let mut log = EventLog::new();
    let u = log.intern("a");
    let path = log.intern("/x.c");
    let red = FileChange {
        path,
        kind: A,
        colour: Some([255, 0, 0]),
        event: ChangeEvent::Commit,
    };
    let blue = FileChange {
        path,
        kind: A,
        colour: Some([0, 0, 255]),
        event: ChangeEvent::Commit,
    };
    let del = FileChange {
        path,
        kind: D,
        colour: None,
        event: ChangeEvent::Commit,
    };
    log.push_commit(1000, u, [red], 0);
    log.push_commit(1000 + 86400, u, [del], 10);
    log.push_commit(1000 + 86400 * 3, u, [blue], 20);
    log.set_total_bytes(30);
    log.set_complete(true);

    let mut sim = Simulation::new(Settings {
        show_key: true,
        // no auto-skip: the entry has time to fade out before "blue"
        auto_skip_seconds: 100.0,
        ..Settings::default()
    });
    sim.set_viewport(1280, 720, 1.0);
    let mut view = SceneView::default();
    let mut saw_red = false;
    let mut dropped = false;
    for _ in 0..60 * 60 {
        sim.update(&log, 1.0 / 60.0);
        match sim.file_key.get("c") {
            Some(e) if e.colour == Vec3::X => saw_red = true,
            Some(_) => break,
            None => dropped = true,
        }
        // a zero count is not listed
        sim.extract_view(&log, &mut view);
        assert!(view.hud.extensions.iter().all(|e| e.count > 0));
        view.clear();
    }
    assert!(saw_red && dropped);
    assert_eq!(sim.file_key["c"].colour, Vec3::Z);
    assert_eq!(sim.file_key["c"].count, 1);
}

#[test]
fn recolour_key_reseeds_files_users_and_key() {
    let log = make_log(&[(1000, "alice", &[("/x.c", A)])]);
    let mut sim = new_sim();
    for _ in 0..30 {
        sim.update(&log, 1.0 / 60.0);
    }
    let f = sim.tree.find_file("/x.c").unwrap();
    assert_eq!(sim.tree.files[f].file_colour(), colour_hash(b"c", 31));
    sim.handle_input(key('s'));
    sim.update(&log, 1.0 / 60.0);
    let seed = sim.settings.hash_seed as i32;
    assert_ne!(seed, 31);
    assert!((1..=10000).contains(&seed));
    assert_eq!(sim.tree.files[f].file_colour(), colour_hash(b"c", seed));
    let u = sim.user_map["alice"];
    assert_eq!(sim.users[u].usercol, colour_hash(b"alice", seed));
    assert_eq!(sim.file_key["c"].colour, colour_hash(b"c", seed));
    assert_eq!(sim.option_value("hash-seed").unwrap(), seed.to_string());

    // set_option("hash-seed") recolours the same way
    sim.set_option("hash-seed", "77").unwrap();
    assert_eq!(sim.tree.files[f].file_colour(), colour_hash(b"c", 77));
}

#[test]
fn follow_user_and_highlight_options() {
    let log = make_log(&[
        (1000, "alice", &[("/x.c", A)]),
        (1000, "bob", &[("/y.c", A)]),
    ]);
    let mut sim = Simulation::new(Settings {
        follow_users: vec!["bob".into()],
        highlight_users: vec!["alice".into()],
        ..Settings::default()
    });
    sim.update(&log, 1.0 / 60.0);
    sim.update(&log, 1.0 / 60.0);
    let bob = sim.user_map["bob"];
    let alice = sim.user_map["alice"];
    assert_eq!(sim.selected_user, Some(bob));
    assert!(sim.users[alice].highlighted);
    assert!(!sim.users[bob].highlighted);

    sim.set_option("highlight-users", "true").unwrap();
    assert!(sim.users[bob].highlighted);
    sim.set_option("highlight-users", "false").unwrap();
    sim.set_option("highlight-user", "").unwrap();
    assert!(!sim.users[alice].highlighted);

    let mut view = SceneView::default();
    sim.extract_view(&log, &mut view);
    // the selected user's name is the last label, in the selected font
    let last = view.labels.last().unwrap();
    assert_eq!(last.font, FontRole::UserSelected);
    assert_eq!(view.str(last.text), "bob");
}

#[test]
fn camera_focus_frames_one_top_level_dir_and_survives_restart() {
    let log = make_log(&[
        (
            1000,
            "o/a",
            &[("/o:a/x.c", A), ("/o:a/y.c", A), ("/o:a/z.c", A)],
        ),
        (
            1000,
            "o/b",
            &[("/o:b/x.c", A), ("/o:b/y.c", A), ("/o:b/z.c", A)],
        ),
    ]);
    let run = |focus: &str| {
        let mut sim = new_sim();
        sim.set_camera_focus(focus);
        for _ in 0..600 {
            sim.update(&log, 1.0 / 60.0);
        }
        sim
    };
    let (whole, a, b) = (run(""), run("/o:a/"), run("/o:b/"));
    let centre = |sim: &Simulation, dir: &str| sim.tree.dir_bounds_under(dir).centre();
    let xy = |sim: &Simulation| Vec2::new(sim.camera.pos.x, sim.camera.pos.y);
    // Each focus moves the camera towards its own dir, not the whole tree.
    assert!(xy(&a).distance(centre(&a, "/o:a/")) < xy(&whole).distance(centre(&whole, "/o:a/")));
    assert!(xy(&b).distance(centre(&b, "/o:b/")) < xy(&whole).distance(centre(&whole, "/o:b/")));
    // A dir with nothing visible frames the whole tree.
    let missing = run("/o:missing/");
    assert_eq!(missing.camera.pos, whole.camera.pos);
    // Several dirs, one per line (the legacy /owner/repo/ layout is listed too).
    assert_eq!(run("/o/b/\n/o:b/").camera.pos, b.camera.pos);

    let mut sim = run("/o:a/");
    sim.restart(&log);
    assert_eq!(sim.camera_focus(), "/o:a/");
    sim.set_camera_focus("");
    assert_eq!(sim.camera_focus(), "");
}

#[test]
fn camera_mode_option_and_key() {
    let mut sim = new_sim();
    assert!(!sim.track_users);
    sim.set_option("camera-mode", "track").unwrap();
    assert!(sim.track_users);
    let log = make_log(&[(1000, "a", &[("/x.c", A)])]);
    sim.update(&log, 1.0 / 60.0);
    sim.handle_input(key('v'));
    assert!(!sim.track_users);
    assert_eq!(sim.option_value("camera-mode").unwrap(), "overview");
    sim.handle_input(InputEvent::PointerDown {
        x: 1.0,
        y: 1.0,
        button: PointerButton::Middle,
    });
    assert!(sim.track_users);
}

#[test]
fn keys_change_speed_time_scale_and_gravity() {
    let log = make_log(&[(1000, "a", &[("/x.c", A)])]);
    let mut sim = new_sim();
    sim.update(&log, 1.0 / 60.0);
    sim.handle_input(key('+'));
    assert_eq!(sim.settings.days_per_second, 0.2);
    // doubling up to 1, then whole days: 0.4, 0.8, 1.0
    for _ in 0..3 {
        sim.handle_input(key('='));
    }
    assert_eq!(sim.settings.days_per_second, 1.0);
    sim.handle_input(key('='));
    assert_eq!(sim.settings.days_per_second, 2.0);
    sim.handle_input(key('-'));
    assert_eq!(sim.settings.days_per_second, 1.0);
    sim.handle_input(key('-'));
    assert_eq!(sim.settings.days_per_second, 0.5);

    sim.handle_input(key('.'));
    assert_eq!(sim.settings.time_scale, 2.0);
    sim.handle_input(key(','));
    assert_eq!(sim.settings.time_scale, 1.0);
    sim.handle_input(key(','));
    assert_eq!(sim.settings.time_scale, 0.5);
    sim.handle_input(key('/'));
    assert_eq!(sim.settings.time_scale, 1.0);

    let g = sim.tree.config.force_gravity;
    sim.handle_input(key(']'));
    assert_eq!(sim.tree.config.force_gravity, (g as f64 * 1.1) as f32);
    sim.handle_input(key('T'));
    assert!(sim.settings.hide_tree);

    // u cycles: names -> hidden -> highlighted -> names
    sim.handle_input(key('u'));
    assert!(sim.settings.hide_usernames && !sim.settings.highlight_all_users);
    sim.handle_input(key('u'));
    assert!(!sim.settings.hide_usernames && sim.settings.highlight_all_users);
    sim.handle_input(key('u'));
    assert!(!sim.settings.hide_usernames && !sim.settings.highlight_all_users);

    // disable-input ignores keys
    sim.set_option("disable-input", "true").unwrap();
    sim.handle_input(key('t'));
    assert!(sim.settings.hide_tree);
}

#[test]
fn space_pauses_and_paused_frames_only_move_the_camera() {
    let log = make_log(&[(1000, "a", &[("/x.c", A)]), (5000, "a", &[("/y.c", A)])]);
    let mut sim = new_sim();
    sim.update(&log, 1.0 / 60.0);
    sim.handle_input(special(Key::Space));
    assert!(sim.paused());
    let (t, cursor, runtime) = (sim.currtime, sim.cursor, sim.runtime);
    for _ in 0..120 {
        sim.update(&log, 1.0 / 60.0);
    }
    assert_eq!(
        (sim.currtime, sim.cursor, sim.runtime),
        (t, cursor, runtime)
    );
    sim.handle_input(special(Key::Space));
    assert!(!sim.paused());
    sim.update(&log, 1.0 / 60.0);
    assert!(sim.currtime > t);
}

#[test]
fn wheel_zooms_within_limits() {
    let log = make_log(&[(1000, "a", &[("/x.c", A)])]);
    let mut sim = new_sim();
    sim.update(&log, 1.0 / 60.0);
    let before = -sim.camera.dest.z;
    sim.handle_input(InputEvent::Wheel { delta: 1.0 });
    assert!(sim.manual_zoom);
    assert_eq!(-sim.camera.dest.z, before * 1.1);
    for _ in 0..200 {
        sim.handle_input(InputEvent::Wheel { delta: -1.0 });
    }
    assert_eq!(-sim.camera.dest.z, sim.settings.camera_zoom_min);
    for _ in 0..200 {
        sim.handle_input(InputEvent::Wheel { delta: 1.0 });
    }
    assert_eq!(-sim.camera.dest.z, sim.settings.camera_zoom_max);
}

#[test]
fn default_font_scale_follows_the_viewport_until_set_explicitly() {
    let mut sim = Simulation::new(Settings::default());
    sim.set_viewport(1280, 720, 1.0);
    assert_eq!(sim.settings.font_scale, 1.0);
    sim.set_viewport(2560, 1599, 1.0);
    assert_eq!(sim.settings.font_scale, 1.0);
    sim.set_viewport(2560, 1600, 1.0);
    assert_eq!(sim.settings.font_scale, 2.0);
    sim.set_viewport(3840, 3200, 1.0);
    assert_eq!(sim.settings.font_scale, 3.0);
    // HiDPI: the DPI ratio is the scale, applied through ui_scale
    sim.set_viewport(3840, 2160, 2.0);
    assert_eq!(sim.settings.font_scale, 1.0);

    sim.set_option("font-scale", "1.5").unwrap();
    assert!(!sim.settings.default_font_scale);
    sim.set_viewport(3840, 3200, 1.0);
    assert_eq!(sim.settings.font_scale, 1.5);
    // an empty value restores the automatic scale at once
    sim.set_option("font-scale", "").unwrap();
    assert!(sim.settings.default_font_scale);
    assert_eq!(sim.settings.font_scale, 3.0);
    assert_eq!(sim.settings.scaled_font_size(), 48);

    // an explicit scale survives restart
    sim.set_option("font-scale", "2.5").unwrap();
    let log = make_log(&[(1000, "a", &[("/x.c", A)])]);
    sim.restart(&log);
    sim.set_viewport(1280, 720, 1.0);
    assert_eq!(sim.settings.font_scale, 2.5);
}

#[test]
fn dates_use_the_offset_in_force_at_each_timestamp() {
    // 1262304000 = 2010-01-01 00:00 UTC; 1277787455 = 2010-06-29 04:57:35 UTC
    let log = make_log(&[
        (1262304000, "a", &[("/x.c", A)]),
        (1277787455, "a", &[("/y.c", A)]),
    ]);
    let mut sim = Simulation::new(Settings {
        // CET in winter, CEST from 2010-03-28 01:00 UTC
        utc_offset_minutes: 60,
        utc_offset_changes: vec![(1269738000, 120)],
        ..Settings::default()
    });
    sim.set_viewport(1280, 720, 1.0);
    assert_eq!(sim.date_at(&log, 0.0).unwrap(), "Friday, 01 January, 2010");
    sim.update(&log, 1.0 / 60.0);
    assert_eq!(
        sim.status(&log).display_date,
        "Friday, 01 January, 2010 01:00:00"
    );
    // a summer date given without a zone is read with the summer offset
    sim.set_option("start-date", "2010-06-29 06:57:35").unwrap();
    assert_eq!(sim.settings.start_timestamp, 1277787455);
    sim.restart(&log);
    sim.update(&log, 1.0 / 60.0);
    assert_eq!(
        sim.status(&log).display_date,
        "Tuesday, 29 June, 2010 06:57:35"
    );
}

/// The host can supply the zone's offset table after the options were set:
/// dates and the stop date follow it, without a restart.
#[test]
fn utc_offsets_can_be_replaced_later() {
    let log = make_log(&[(1_230_000_720, "a", &[("/x.c", A)])]);
    let mut sim = new_sim();
    sim.set_option("stop-date", "2009-06-01 12:00:00").unwrap();
    // UTC-4 (summer time) everywhere: what a constant offset gives
    sim.set_utc_offsets(-240, Vec::new());
    assert_eq!(sim.settings().stop_timestamp, 1_243_872_000);
    for _ in 0..10 {
        sim.update(&log, 1.0 / 60.0);
    }
    assert!(sim.status(&log).display_date.contains("22:52:00"));
    // UTC-5 until the change to summer time on 2009-03-08 07:00 UTC
    sim.set_utc_offsets(-300, vec![(1_236_495_600, -240)]);
    assert_eq!(sim.settings().stop_timestamp, 1_243_872_000);
    sim.update(&log, 1.0 / 60.0);
    assert!(sim.status(&log).display_date.contains("21:52:00"));
    assert_eq!(sim.date_at(&log, 0.0).unwrap(), "Monday, 22 December, 2008");
    // a stop date in winter moves with the table
    sim.set_option("stop-date", "2009-01-01 00:00:00").unwrap();
    assert_eq!(sim.settings().stop_timestamp, 1_230_786_000);
    sim.set_utc_offsets(-240, Vec::new());
    assert_eq!(sim.settings().stop_timestamp, 1_230_782_400);
}

#[test]
fn keypad_keys_zoom_and_shift_does_not_change_the_command() {
    let log = make_log(&[(1000, "a", &[("/x.c", A)])]);
    let mut sim = new_sim();
    sim.update(&log, 1.0 / 60.0);
    let dps = sim.settings.days_per_second;
    let before = -sim.camera.dest.z;
    sim.handle_input(special(Key::KeypadPlus));
    assert!(sim.manual_zoom);
    assert_eq!(-sim.camera.dest.z, before * 1.1);
    sim.handle_input(special(Key::KeypadMinus));
    assert_eq!(-sim.camera.dest.z, before * 1.1 / 1.1);
    assert_eq!(sim.settings.days_per_second, dps, "keypad keys only zoom");

    // Shift+'.', Shift+',', Shift+'/' run the time-scale commands
    sim.handle_input(key('>'));
    assert_eq!(sim.settings.time_scale, 2.0);
    sim.handle_input(key('<'));
    assert_eq!(sim.settings.time_scale, 1.0);
    sim.handle_input(key('<'));
    assert_eq!(sim.settings.time_scale, 0.5);
    sim.handle_input(key('?'));
    assert_eq!(sim.settings.time_scale, 1.0);
    // Shift+'[' / Shift+']' change gravity, Shift+'-' slows down
    let g = sim.tree.config.force_gravity;
    sim.handle_input(key('}'));
    assert_eq!(sim.tree.config.force_gravity, (g as f64 * 1.1) as f32);
    sim.handle_input(key('{'));
    assert_eq!(
        sim.tree.config.force_gravity,
        ((g as f64 * 1.1) as f32 as f64 / 1.1) as f32
    );
    sim.handle_input(key('_'));
    assert_eq!(sim.settings.days_per_second, dps * 0.5);
}

/// Screen position of world point `p` with Gource's mousetrace formula
/// inverted.
fn screen_of(sim: &Simulation, p: Vec2) -> Vec2 {
    let cam = sim.camera.pos;
    let (w, h) = (sim.viewport.x, sim.viewport.y);
    let q = (p - cam.truncate()) / cam.z;
    Vec2::new((w - q.x * h) / 2.0, (1.0 - q.y) * h / 2.0)
}

#[test]
fn clicking_a_file_selects_it_and_clicking_space_deselects() {
    let log = make_log(&[(1000, "a", &[("/src/x.c", A)])]);
    let mut sim = new_sim();
    let mut view = SceneView::default();
    // let the user arrive and finish, the file appear
    for _ in 0..300 {
        sim.update(&log, 1.0 / 60.0);
        sim.extract_view(&log, &mut view);
    }
    let f = sim.tree.find_file("/src/x.c").unwrap();
    let target = screen_of(&sim, sim.tree.file_abs_pos(f));
    sim.handle_input(InputEvent::PointerMove {
        x: target.x,
        y: target.y,
        buttons: 0,
    });
    sim.update(&log, 1.0 / 60.0);
    sim.extract_view(&log, &mut view);
    assert_eq!(sim.hover_file, Some(f));
    // hover text box: name, then the path without its leading '/'
    let tb = view.hud.textbox.as_ref().expect("text box");
    let lines: Vec<&str> = tb.lines.iter().map(|&l| view.str(l)).collect();
    assert_eq!(lines, vec!["x.c", "src/"]);

    sim.handle_input(InputEvent::PointerDown {
        x: target.x,
        y: target.y,
        button: PointerButton::Left,
    });
    sim.update(&log, 1.0 / 60.0);
    sim.extract_view(&log, &mut view);
    sim.handle_input(InputEvent::PointerUp {
        x: target.x,
        y: target.y,
        button: PointerButton::Left,
    });
    assert_eq!(sim.selected_file, Some(f));
    assert!(sim.tree.files[f].pawn.selected);
    let last = view.labels.last().unwrap();
    assert_eq!(last.font, FontRole::FileSelected);
    assert_eq!(view.str(last.text), "x.c");
    // a selected file shows no text box
    assert!(view.hud.textbox.is_none());

    // click on empty space: deselect and start a background drag
    sim.handle_input(InputEvent::PointerMove {
        x: 5.0,
        y: 5.0,
        buttons: 0,
    });
    sim.handle_input(InputEvent::PointerDown {
        x: 5.0,
        y: 5.0,
        button: PointerButton::Left,
    });
    sim.update(&log, 1.0 / 60.0);
    sim.extract_view(&log, &mut view);
    assert_eq!(sim.selected_file, None);
    assert!(sim.mousedragged && sim.manual_camera && sim.grab_mouse);
    // dragging pans the camera by the relative motion
    let before = sim.camera.pos;
    sim.handle_input(InputEvent::PointerMove {
        x: 25.0,
        y: 5.0,
        buttons: buttons::LEFT,
    });
    sim.update(&log, 1.0 / 60.0);
    assert!(sim.camera.pos.x > before.x);
    sim.handle_input(InputEvent::PointerUp {
        x: 25.0,
        y: 5.0,
        button: PointerButton::Left,
    });
    assert!(!sim.mousedragged && !sim.grab_mouse);
}

#[test]
fn right_drag_rotates_about_the_camera() {
    let log = make_log(&[(1000, "a", &[("/src/x.c", A), ("/doc/y.md", A)])]);
    let mut sim = new_sim();
    for _ in 0..120 {
        sim.update(&log, 1.0 / 60.0);
    }
    sim.handle_input(InputEvent::PointerMove {
        x: 100.0,
        y: 100.0,
        buttons: 0,
    });
    sim.handle_input(InputEvent::PointerDown {
        x: 100.0,
        y: 100.0,
        button: PointerButton::Right,
    });
    assert!(sim.grab_mouse);
    sim.handle_input(InputEvent::PointerMove {
        x: 120.0,
        y: 101.0,
        buttons: buttons::RIGHT,
    });
    assert!(sim.manual_rotate);
    let expected = (5.0f32 as f64 * crate::frustum::DEGREES_TO_RADIANS) as f32;
    assert_eq!(sim.rotate_angle, expected);
    let d = sim.tree.dir_map["/src/"];
    let before = sim.tree.dirs[d].pos;
    sim.update(&log, 1.0 / 60.0);
    assert_eq!(sim.rotate_angle, 0.0);
    assert_ne!(sim.tree.dirs[d].pos, before);
}

#[test]
fn tab_selects_users_in_tag_order() {
    let log = make_log(&[(1000, "b", &[("/x.c", A)]), (1000, "a", &[("/y.c", A)])]);
    let mut sim = new_sim();
    for _ in 0..90 {
        sim.update(&log, 1.0 / 60.0);
    }
    // tags: x.c 1, b 2, y.c 3, a 4
    sim.handle_input(special(Key::Tab));
    assert_eq!(sim.selected_user, Some(sim.user_map["b"]));
    sim.handle_input(special(Key::Tab));
    assert_eq!(sim.selected_user, Some(sim.user_map["a"]));
    sim.handle_input(special(Key::Tab));
    assert_eq!(sim.selected_user, Some(sim.user_map["b"]));
}

#[test]
fn seek_resets_and_continues_from_the_fraction() {
    let mut log = EventLog::new();
    let u = log.intern("u");
    for i in 0..100 {
        let c = change(&mut log, &format!("/f{i}.c"), A);
        log.push_commit(1000 + i * 100_000, u, [c], i as u64 * 10);
    }
    log.set_total_bytes(1000);
    log.set_complete(true);

    let mut sim = new_sim();
    for _ in 0..10 {
        sim.update(&log, 1.0 / 60.0);
    }
    sim.set_paused(true);
    sim.seek(&log, 0.5);
    assert!(!sim.paused(), "seeking unpauses");
    assert_eq!(sim.cursor, 50);
    assert_eq!(sim.tree.file_count(), 0);
    sim.update(&log, 1.0 / 60.0);
    assert_eq!(sim.tree.file_count(), 1);
    assert!(sim.tree.find_file("/f50.c").is_some());
    let s = sim.status(&log);
    assert!((s.fraction - 0.51).abs() < 1e-6, "{}", s.fraction);
    assert!(s.seekable);

    // an incomplete log cannot be seeked
    log.set_complete(false);
    let cursor = sim.cursor;
    sim.seek(&log, 0.1);
    assert_eq!(sim.cursor, cursor);
    assert!(!sim.status(&log).seekable);

    // date_at peeks at the commit under a slider position
    log.set_complete(true);
    let date = sim.date_at(&log, 0.0).unwrap();
    assert_eq!(date, "Thursday, 01 January, 1970");
    assert_eq!(sim.date_at(&log, 1.0), None);
}

/// 100 commits, one new file each, 100000 s apart (10 bytes per commit).
fn hundred_commit_log(complete: bool) -> EventLog {
    let mut log = EventLog::new();
    let u = log.intern("u");
    for i in 0..100 {
        let c = change(&mut log, &format!("/f{i}.c"), A);
        log.push_commit(1000 + i * 100_000, u, [c], i as u64 * 10);
    }
    log.set_total_bytes(1000);
    log.set_complete(complete);
    log
}

#[test]
fn restart_ends_a_pause_so_the_scene_refills() {
    let log = hundred_commit_log(true);
    let mut sim = new_sim();
    for _ in 0..120 {
        sim.update(&log, 1.0 / 60.0);
    }
    sim.set_paused(true);
    sim.restart(&log);
    assert!(!sim.paused(), "a restart is a reset: the pause ends");
    for _ in 0..120 {
        sim.update(&log, 1.0 / 60.0);
    }
    let s = sim.status(&log);
    assert!(s.files > 0 && s.commits_processed > 0, "{s:?}");
}

#[test]
fn start_position_waits_for_a_streamed_log_to_complete() {
    let settings = Settings {
        start_position: 0.5,
        ..Settings::default()
    };

    // complete from the start: seek once at init
    let complete = hundred_commit_log(true);
    let mut sim = Simulation::new(settings.clone());
    sim.set_viewport(1280, 720, 1.0);
    sim.update(&complete, 1.0 / 60.0);
    assert_eq!(sim.status(&complete).commits_processed, 51);
    assert!(sim.tree.find_file("/f50.c").is_some());

    // streamed: plays from the start until the log completes, then seeks
    let mut log = hundred_commit_log(false);
    let mut sim = Simulation::new(settings);
    sim.set_viewport(1280, 720, 1.0);
    for _ in 0..30 {
        sim.update(&log, 1.0 / 60.0);
    }
    assert!(sim.tree.find_file("/f0.c").is_some());
    assert!(sim.pending_start_position);
    log.set_complete(true);
    sim.update(&log, 1.0 / 60.0);
    assert!(!sim.pending_start_position);
    let s = sim.status(&log);
    assert_eq!(s.commits_processed, 51);
    assert!((s.fraction - 0.51).abs() < 1e-6, "{}", s.fraction);
    assert!(sim.tree.find_file("/f0.c").is_none());
    assert!(sim.tree.find_file("/f50.c").is_some());

    // applied once only
    for _ in 0..30 {
        sim.update(&log, 1.0 / 60.0);
    }
    assert!(sim.status(&log).commits_processed >= 51);
}

#[test]
fn streaming_log_waits_then_continues() {
    let mut log = EventLog::new();
    let u = log.intern("u");
    let push = |log: &mut EventLog, i: u64| {
        let p = log.intern(&format!("/d{}/f{i}.c", i % 3));
        log.push_commit(
            1000 + i as i64 * 3600,
            u,
            [FileChange {
                path: p,
                kind: A,
                colour: None,
                event: ChangeEvent::Commit,
            }],
            i * 20,
        );
    };

    let mut sim = new_sim();
    // empty, incomplete: loading
    for _ in 0..10 {
        sim.update(&log, 1.0 / 60.0);
    }
    assert!(!sim.initialised);
    assert!(!sim.status(&log).finished);

    for i in 0..5 {
        push(&mut log, i);
    }
    for _ in 0..600 {
        sim.update(&log, 1.0 / 60.0);
    }
    assert_eq!(sim.tree.file_count(), 5);
    let s = sim.status(&log);
    assert!(!s.finished, "an incomplete log waits");
    assert!(!s.seekable);
    // no position until the log is complete (the byte total still grows)
    assert_eq!(s.fraction, 0.0);

    for i in 5..10 {
        push(&mut log, i);
    }
    log.set_complete(true);
    log.set_total_bytes(200);
    for _ in 0..600 {
        sim.update(&log, 1.0 / 60.0);
    }
    assert_eq!(sim.tree.file_count(), 10);
    let s = sim.status(&log);
    assert!(s.finished);
    assert_eq!(s.fraction, 1.0);
}

/// A stop condition ends one pass, not the session: seeking (or looping)
/// starts a new pass that plays until the condition is reached again.
#[test]
fn seeking_after_a_stop_condition_plays_again() {
    let log = hundred_commit_log(true);
    let fast = Settings {
        days_per_second: 10.0,
        ..Settings::default()
    };
    let stop_date = |extra: Settings| {
        Simulation::new(Settings {
            stop_timestamp: 1000 + 30 * 100_000,
            ..extra
        })
    };
    let run = |sim: &mut Simulation, frames: usize| {
        for _ in 0..frames {
            sim.update(&log, 1.0 / 60.0);
        }
    };
    let cases = [
        stop_date(fast.clone()),
        Simulation::new(Settings {
            stop_position: 0.3,
            ..fast.clone()
        }),
        Simulation::new(Settings {
            stop_at_end: true,
            ..fast.clone()
        }),
    ];
    for mut sim in cases {
        sim.set_viewport(1280, 720, 1.0);
        run(&mut sim, 60 * 60);
        assert!(sim.stop_position_reached, "{:?}", sim.status(&log));
        assert!(sim.status(&log).finished);
        sim.seek(&log, 0.1);
        assert!(!sim.status(&log).finished);
        run(&mut sim, 60);
        let s = sim.status(&log);
        assert!(s.files > 0 && s.commits_processed > 10, "{s:?}");
    }

    // loop + stop date: every pass replays the start
    let mut sim = stop_date(Settings {
        loop_playback: true,
        ..fast.clone()
    });
    sim.set_viewport(1280, 720, 1.0);
    let (mut passes, mut last_cursor) = (0, 0);
    for _ in 0..60 * 120 {
        sim.update(&log, 1.0 / 60.0);
        if sim.cursor < last_cursor {
            passes += 1;
            assert!(sim.tree.find_file("/f0.c").is_some());
        }
        last_cursor = sim.cursor;
        assert!(sim.cursor <= 32, "never plays past the stop date");
    }
    assert!(passes >= 2, "{passes} loops");
}

/// Timestamps at the ends of i64 (a saturated `atoll` in the log reader)
/// neither overflow the clock nor break date formatting.
#[test]
fn extreme_timestamps_do_not_overflow_the_clock() {
    let log = make_log(&[
        (i64::MAX - 5, "a", &[("/x.c", A)]),
        (i64::MAX, "a", &[("/y.c", A)]),
        (i64::MIN, "b", &[("/z.c", A)]),
        (1_000_000, "b", &[("/w.c", A)]),
    ]);
    let mut sim = Simulation::new(Settings {
        utc_offset_minutes: 14 * 60,
        ..Settings::default()
    });
    sim.set_viewport(1280, 720, 1.0);
    let mut view = SceneView::default();
    for _ in 0..60 * 20 {
        sim.update(&log, 1.0 / 60.0);
        sim.extract_view(&log, &mut view);
        let _ = sim.date_at(&log, 0.3);
        assert!(sim.currtime >= i64::MAX - 5 || sim.currtime <= 1_000_000 + 86400 * 1000);
    }
    assert_eq!(sim.status(&log).commits_processed, 4);
}

#[test]
fn stop_conditions_and_loop() {
    let log = make_log(&[(1000, "a", &[("/x.c", A)]), (2000, "a", &[("/y.c", A)])]);
    let mut sim = Simulation::new(Settings {
        stop_at_time: 0.5,
        ..Settings::default()
    });
    for _ in 0..60 {
        sim.update(&log, 1.0 / 60.0);
    }
    assert!(sim.stop_position_reached);

    // loop: after the log ends and everything is idle, start again
    let mut sim = Simulation::new(Settings {
        loop_playback: true,
        ..Settings::default()
    });
    let mut restarted = false;
    let mut seen_two = false;
    for _ in 0..60 * 40 {
        sim.update(&log, 1.0 / 60.0);
        if sim.tree.file_count() == 2 {
            seen_two = true;
        }
        if seen_two && sim.tree.file_count() < 2 {
            restarted = true;
            break;
        }
    }
    assert!(restarted, "loop playback restarts from the beginning");
}

#[test]
fn deleting_a_selected_hovered_file_clears_both() {
    let log = make_log(&[(1000, "a", &[("/x.c", A)]), (1001, "a", &[("/x.c", D)])]);
    let mut sim = new_sim();
    sim.update(&log, 1.0 / 60.0);
    let f = sim.tree.find_file("/x.c").unwrap();
    sim.select_file(Some(f));
    sim.hover_file = Some(f);
    for _ in 0..600 {
        sim.update(&log, 1.0 / 60.0);
    }
    assert!(sim.tree.find_file("/x.c").is_none());
    assert_eq!(sim.selected_file, None);
    assert_eq!(sim.hover_file, None);
}

#[test]
fn directory_delete_removes_every_file_below() {
    let log = make_log(&[
        (
            1000,
            "a",
            &[("/a/b/x.c", A), ("/a/b/c/y.c", A), ("/a/z.c", A)],
        ),
        (1001, "a", &[("/a/b/", D)]),
        // adding or modifying a directory path is ignored
        (1002, "a", &[("/a/", M)]),
    ]);
    let mut sim = new_sim();
    for _ in 0..60 * 8 {
        sim.update(&log, 1.0 / 60.0);
    }
    let files: Vec<&str> = sim.tree.file_map.keys().map(String::as_str).collect();
    assert_eq!(files, vec!["/a/z.c"]);
}

#[test]
fn extract_view_before_the_first_commit_is_the_loading_screen() {
    let log = EventLog::new();
    let mut sim = new_sim();
    let mut view = SceneView::default();
    sim.update(&log, 1.0 / 60.0);
    sim.extract_view(&log, &mut view);
    assert!(view.dirs.is_empty());
    assert_eq!(view.background, sim.settings.background_colour);
    assert_eq!(view.viewport, Vec2::new(1280.0, 720.0));
}

#[test]
fn message_and_splash_timers() {
    let log = make_log(&[(1000, "a", &[("/x.c", A)])]);
    let mut sim = new_sim();
    sim.set_message("hello");
    sim.show_splash();
    let mut view = SceneView::default();
    sim.update(&log, 1.0 / 60.0);
    sim.extract_view(&log, &mut view);
    let (text, remaining) = view.hud.message.unwrap();
    assert_eq!(view.str(text), "hello");
    assert!(remaining < 5.0 && remaining > 4.9);
    assert!(view.hud.splash > 14.9);
    for _ in 0..60 * 6 {
        sim.update(&log, 1.0 / 60.0);
    }
    sim.extract_view(&log, &mut view);
    assert!(view.hud.message.is_none());
}

#[test]
fn file_idle_time_at_end_replaces_the_idle_time_and_restart_restores_it() {
    let log = make_log(&[(1000, "a", &[("/x.c", A)])]);
    let mut sim = Simulation::new(Settings {
        file_idle_time_at_end: 2.0,
        ..Settings::default()
    });
    sim.update(&log, 1.0 / 60.0);
    sim.update(&log, 1.0 / 60.0);
    assert_eq!(sim.settings.file_idle_time, 2.0);
    for _ in 0..60 * 8 {
        sim.update(&log, 1.0 / 60.0);
    }
    assert_eq!(sim.tree.file_count(), 0, "the file idled out");
    sim.restart(&log);
    assert_eq!(sim.settings.file_idle_time, 0.0);
}

#[test]
fn pull_request_merges_and_closes_have_their_own_beams() {
    use crate::action::{CREATE_COLOUR, PR_CLOSED_COLOUR, PR_MERGED_COLOUR, REMOVE_COLOUR};

    let mut log = EventLog::new();
    let pr = |log: &mut EventLog, path: &str, kind: ChangeKind, event: ChangeEvent| FileChange {
        path: log.intern(path),
        kind,
        colour: None,
        event,
    };
    let alice = log.intern("alice");
    let bob = log.intern("bob");
    let commits = [
        (
            1000,
            alice,
            pr(&mut log, "/src/a.c", A, ChangeEvent::Commit),
        ),
        (
            1001,
            alice,
            pr(&mut log, "/pull requests/#1 One", A, ChangeEvent::PrOpened),
        ),
        (
            1001,
            bob,
            pr(&mut log, "/pull requests/#2 Two", A, ChangeEvent::PrOpened),
        ),
        (
            1002,
            alice,
            pr(&mut log, "/pull requests/#1 One", D, ChangeEvent::PrMerged),
        ),
        (
            1003,
            bob,
            pr(&mut log, "/pull requests/#2 Two", D, ChangeEvent::PrClosed),
        ),
        (1004, bob, pr(&mut log, "/src/a.c", D, ChangeEvent::Commit)),
    ];
    for (i, (ts, user, change)) in commits.into_iter().enumerate() {
        log.push_commit(ts, user, [change], i as u64 * 10);
    }
    log.set_total_bytes(60);
    log.set_complete(true);

    let mut sim = new_sim();
    let mut colours = Vec::new();
    for _ in 0..30 {
        sim.update(&log, 1.0 / 60.0);
        for (_, user) in sim.users.iter() {
            for a in user.actions.iter().chain(&user.active_actions) {
                if !colours.contains(&a.colour) {
                    colours.push(a.colour);
                }
            }
        }
    }
    for c in [
        CREATE_COLOUR,
        PR_MERGED_COLOUR,
        PR_CLOSED_COLOUR,
        REMOVE_COLOUR,
    ] {
        assert!(colours.contains(&c), "{c:?} in {colours:?}");
    }
    assert_eq!(colours.len(), 4, "{colours:?}");
}
