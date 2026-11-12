// SPDX-License-Identifier: GPL-3.0-or-later

//! Steady-state frames (`update` + `extract_view`) allocate nothing.
//!
//! A counting global allocator records allocations made on the test thread
//! while counting is switched on. The scenario keeps a fixed set of users
//! busy modifying a fixed set of files, so after warm-up no entity, string
//! or buffer has to grow.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use gource_core::events::{
    AttrKind, AttrSetId, Attribution, ChangeEvent, ChangeKind, EventLog, FileChange,
};
use gource_core::input::InputEvent;
use gource_core::settings::Settings;
use gource_core::sim::Simulation;
use gource_core::view::SceneView;

struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
}

fn note() {
    let _ = COUNTING.try_with(|c| {
        if c.get() {
            let _ = ALLOCS.try_with(|a| a.set(a.get() + 1));
        }
    });
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

fn busy_log(files: usize, users: usize, commits: usize) -> EventLog {
    busy_log_with(files, users, commits, |_, _| AttrSetId::NONE)
}

/// [`busy_log`] where `attrs(log, i)` gives the attribution set of the
/// `i`-th modification.
fn busy_log_with(
    files: usize,
    users: usize,
    commits: usize,
    mut attrs: impl FnMut(&mut EventLog, usize) -> AttrSetId,
) -> EventLog {
    let mut log = EventLog::new();
    let paths: Vec<String> = (0..files)
        .map(|i| {
            format!(
                "/dir{}/sub{}/file{i}.{}",
                i % 7,
                i % 3,
                ["c", "h", "rs"][i % 3]
            )
        })
        .collect();
    let names: Vec<String> = (0..users).map(|u| format!("user{u}")).collect();
    let mut offset = 0;
    // initial import
    let u = log.intern(&names[0]);
    let adds: Vec<FileChange> = paths
        .iter()
        .map(|p| FileChange {
            path: log.intern(p),
            kind: ChangeKind::Add,
            colour: None,
            event: ChangeEvent::Commit,
        })
        .collect();
    log.push_commit(1_000_000, u, adds, offset);
    // then a steady stream of modifications, one every 10 minutes
    for i in 0..commits {
        offset += 10;
        let u = log.intern(&names[i % users]);
        let p = log.intern(&paths[(i * 7) % files]);
        let set = attrs(&mut log, i);
        log.push_commit_attr(
            1_000_600 + i as i64 * 600,
            u,
            [FileChange {
                path: p,
                kind: ChangeKind::Modify,
                colour: None,
                event: ChangeEvent::Commit,
            }],
            offset,
            set,
        );
    }
    log.set_total_bytes(offset + 10);
    log.set_complete(true);
    log
}

#[test]
fn steady_state_frames_do_not_allocate() {
    let log = busy_log(200, 5, 4000);
    let mut sim = Simulation::new(Settings {
        // labels for everything, the file key and a title in the HUD
        highlight_all_users: true,
        highlight_dirs: true,
        show_key: true,
        title: "steady".into(),
        ..Settings::default()
    });
    sim.set_viewport(1280, 720, 1.0);
    let mut view = SceneView::default();

    let dt = 1.0 / 60.0;
    let hover = InputEvent::PointerMove {
        x: 640.0,
        y: 360.0,
        buttons: 0,
    };
    for _ in 0..1800 {
        // keep the cursor from idling out (3 s) so the trace runs every frame
        sim.handle_input(hover);
        sim.update(&log, dt);
        sim.extract_view(&log, &mut view);
    }
    let before = sim.status(&log);
    assert!(before.users > 0 && before.files == 200, "{before:?}");

    COUNTING.with(|c| c.set(true));
    for _ in 0..600 {
        sim.handle_input(hover);
        sim.update(&log, dt);
        sim.extract_view(&log, &mut view);
    }
    COUNTING.with(|c| c.set(false));
    let allocs = ALLOCS.with(|a| a.get());

    let after = sim.status(&log);
    assert!(after.commits_processed > before.commits_processed + 100);
    assert!(!view.labels.is_empty() && !view.users.is_empty());
    assert_eq!(allocs, 0, "allocations in 600 steady-state frames");
}

#[test]
fn rich_pr_markers_age_key_and_flow_do_not_allocate_after_warmup() {
    use gource_core::events::{
        PrChangeRecord, PrCompleteness, PrLifecycle, PrLogMetadata, PrLogUpdate, PrPayload, PrState,
    };
    let mut log = EventLog::new();
    let path = log.intern("/src/.pull-requests/#8 Frozen title");
    let owner = log.intern("Owner");
    let change = FileChange {
        path,
        kind: ChangeKind::Add,
        colour: None,
        event: ChangeEvent::PrOpened,
    };
    let record = PrChangeRecord {
        change_index: 0,
        payload: PrPayload::Lifecycle(PrLifecycle {
            number: 8,
            owner,
            target: None,
            created: -86400,
            state: PrState::Open,
            initial: true,
        }),
    };
    log.push_commit_attr_pr(1, owner, &[change], 0, AttrSetId::NONE, &[record])
        .unwrap();
    log.update_pr_log(PrLogUpdate::Metadata(PrLogMetadata {
        version: 1,
        from: -172800,
        to: 172800,
        as_of: 1,
        lifecycle: PrCompleteness::Complete,
        reviews: PrCompleteness::Partial,
        files: PrCompleteness::Unknown,
    }));
    log.set_complete(true);
    let mut sim = Simulation::new(Settings {
        days_per_second: 0.0,
        auto_skip_seconds: -1.0,
        pr_flow_hud: true,
        ..Settings::default()
    });
    sim.set_viewport(390, 384, 1.0);
    let mut view = SceneView::default();
    for _ in 0..120 {
        sim.update(&log, 1.0 / 60.0);
        sim.extract_view(&log, &mut view);
    }
    assert_eq!(view.files.len(), 3);
    assert_eq!(view.hud.pr_key.len(), 3);
    assert!(view.hud.pr_flow.is_some());
    ALLOCS.with(|a| a.set(0));
    COUNTING.with(|c| c.set(true));
    for _ in 0..600 {
        sim.update(&log, 1.0 / 60.0);
        sim.extract_view(&log, &mut view);
    }
    COUNTING.with(|c| c.set(false));
    assert_eq!(
        ALLOCS.with(|a| a.get()),
        0,
        "rich extraction must reuse warmed buffers"
    );
}

/// The same with AI attributions: assisted and agent-authored commits,
/// decorations from the built-in and viewer rules, the agent key and the
/// hover line.
#[test]
fn steady_state_frames_with_decorations_do_not_allocate() {
    let log = busy_log_with(200, 5, 4000, |log, i| {
        let entry = |log: &mut EventLog, kind, id: &str, model: Option<&str>| Attribution {
            kind,
            id: log.intern(id),
            model: model.map(|m| log.intern(m)),
        };
        let set = match i % 4 {
            0 => vec![entry(
                log,
                AttrKind::Assist,
                "claude",
                Some("Claude Opus 5.5"),
            )],
            1 => vec![
                entry(log, AttrKind::Assist, "claude", None),
                entry(log, AttrKind::Assist, "codex", None),
            ],
            2 => vec![entry(log, AttrKind::Agent, "cursor", None)],
            _ => Vec::new(),
        };
        log.intern_attr_set(&set)
    });
    let mut settings = Settings {
        highlight_all_users: true,
        agent_key: true,
        title: "steady".into(),
        ..Settings::default()
    };
    settings
        .apply_option(
            "user-style",
            "ai:codex => hat=agent; glow=agent\nhuman => badge=on",
        )
        .unwrap();
    let mut sim = Simulation::new(settings);
    sim.set_viewport(1280, 720, 1.0);
    let mut view = SceneView::default();

    let dt = 1.0 / 60.0;
    let hover = |sim: &mut Simulation, frame: usize| {
        // sweep the pointer so it crosses users (hover text with the AI line)
        let x = 340.0 + (frame % 600) as f32;
        sim.handle_input(InputEvent::PointerMove {
            x,
            y: 360.0,
            buttons: 0,
        });
    };
    let mut decorated = 0;
    let mut hovered_ai = 0;
    let mut step = |sim: &mut Simulation, view: &mut SceneView, frame: usize| {
        hover(sim, frame);
        sim.update(&log, dt);
        sim.extract_view(&log, view);
        decorated += view.users.iter().filter(|u| u.decor.mask != 0).count();
        if let Some(tb) = &view.hud.textbox
            && tb.lines.len() == 2
        {
            hovered_ai += 1;
        }
    };
    for frame in 0..1800 {
        step(&mut sim, &mut view, frame);
    }
    COUNTING.with(|c| c.set(true));
    for frame in 1800..2400 {
        step(&mut sim, &mut view, frame);
    }
    COUNTING.with(|c| c.set(false));
    let allocs = ALLOCS.with(|a| a.get());

    assert!(decorated > 0, "no decorations were drawn");
    assert!(hovered_ai > 0, "the hover line never showed");
    assert!(!view.hud.agents.is_empty());
    assert_eq!(
        allocs, 0,
        "allocations in 600 steady-state frames with decorations"
    );
}
