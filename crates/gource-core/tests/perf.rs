// SPDX-License-Identifier: GPL-3.0-or-later

//! Performance smoke test: a synthetic history of ~20k files in ~1000
//! directories by 200 users, 600 frames of `update` + `extract_view`.
//!
//! Prints the time per frame. Release builds (`cargo test --release -p
//! gource-core --test perf -- --nocapture`) must stay under a generous
//! budget; debug builds run the same frames without a time limit.

use std::time::Instant;

use gource_core::events::{ChangeEvent, ChangeKind, EventLog, FileChange};
use gource_core::settings::Settings;
use gource_core::sim::Simulation;
use gource_core::view::SceneView;

const USERS: usize = 200;
const FILES: usize = 20_000;
const FILES_PER_COMMIT: usize = 20;

fn synthetic_log() -> EventLog {
    let mut log = EventLog::new();
    let users: Vec<String> = (0..USERS).map(|u| format!("dev{u:03}")).collect();
    let exts = ["c", "h", "rs", "md", "png", "toml", "js", "ts"];
    let paths: Vec<String> = (0..FILES)
        .map(|i| {
            let d = i % 1000;
            format!(
                "/m{}/p{}/q{}/f{i}.{}",
                d % 10,
                (d / 10) % 10,
                d / 100,
                exts[i % exts.len()]
            )
        })
        .collect();
    let mut ts = 1_500_000_000i64;
    let mut offset = 0u64;
    // creation: 20 files per commit, one commit a minute
    for (c, chunk) in paths.chunks(FILES_PER_COMMIT).enumerate() {
        let u = log.intern(&users[c % USERS]);
        let changes: Vec<FileChange> = chunk
            .iter()
            .map(|p| FileChange {
                path: log.intern(p),
                kind: ChangeKind::Add,
                colour: None,
                event: ChangeEvent::Commit,
            })
            .collect();
        log.push_commit(ts, u, changes, offset);
        ts += 60;
        offset += 100;
    }
    // then modifications and some deletions
    for c in 0..2000usize {
        let u = log.intern(&users[(c * 7) % USERS]);
        let changes: Vec<FileChange> = (0..5)
            .map(|k| FileChange {
                path: log.intern(&paths[(c * 131 + k * 17) % FILES]),
                kind: if k == 4 && c % 3 == 0 {
                    ChangeKind::Delete
                } else {
                    ChangeKind::Modify
                },
                colour: None,
                event: ChangeEvent::Commit,
            })
            .collect();
        log.push_commit(ts, u, changes, offset);
        ts += 60;
        offset += 100;
    }
    log.set_total_bytes(offset);
    log.set_complete(true);
    log
}

#[test]
fn twenty_thousand_files_and_two_hundred_users() {
    let log = synthetic_log();
    let mut sim = Simulation::new(Settings::default());
    sim.set_viewport(1920, 1080, 1.0);
    let mut view = SceneView::default();

    let dt = 1.0 / 60.0;
    let frames = 600;
    let mut max_files = 0;
    let mut max_users = 0;
    let start = Instant::now();
    let mut worst = 0.0f64;
    for _ in 0..frames {
        let t = Instant::now();
        sim.update(&log, dt);
        sim.extract_view(&log, &mut view);
        worst = worst.max(t.elapsed().as_secs_f64() * 1000.0);
        let s = sim.status(&log);
        max_files = max_files.max(s.files);
        max_users = max_users.max(s.users);
    }
    let total = start.elapsed().as_secs_f64() * 1000.0;
    let s = sim.status(&log);
    let c = sim.counters();
    println!(
        "perf: {frames} frames, {:.3} ms/frame (worst {:.2} ms); files {} (max {}), users {} (max {}), dirs {}; \
         view: {} dirs, {} files, {} users, {} actions, {} labels; inner loops: dirs {}, users {}",
        total / frames as f64,
        worst,
        s.files,
        max_files,
        s.users,
        max_users,
        s.dirs,
        view.dirs.len(),
        view.files.len(),
        view.users.len(),
        view.actions.len(),
        view.labels.len(),
        c.dir_inner_loops,
        c.user_inner_loops,
    );

    assert!(
        max_files >= FILES * 9 / 10,
        "most files were created: {max_files}"
    );
    assert!(max_users >= USERS / 2, "users were active: {max_users}");
    if !cfg!(debug_assertions) {
        // Gource's own budget is a 60 fps frame; allow plenty of headroom
        // for slow CI machines.
        assert!(total / (frames as f64) < 50.0, "too slow: {total} ms");
    }
}
