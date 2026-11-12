// SPDX-License-Identifier: GPL-3.0-or-later
//! Real Simulation regressions for typed PR lifecycle, not a copied model.
use gource_core::events::{
    AttrSetId, ChangeEvent, ChangeKind, EventLog, FileChange, PrChangeRecord, PrLifecycle,
    PrPayload, PrState,
};
use gource_core::settings::Settings;
use gource_core::sim::Simulation;

const NODE: &str = "/src/.pull-requests/#8";

fn settings() -> Settings {
    Settings {
        auto_skip_seconds: -1.0,
        days_per_second: 0.0,
        file_idle_time: 0.0,
        ..Settings::default()
    }
}

fn rich(log: &mut EventLog, actor: &str, event: ChangeEvent, initial: bool) {
    let path = log.intern(NODE);
    let user = log.intern(actor);
    let owner = log.intern("proposal-owner");
    let change = FileChange {
        path,
        kind: match event {
            ChangeEvent::PrMerged | ChangeEvent::PrClosed => ChangeKind::Delete,
            ChangeEvent::PrOpened | ChangeEvent::PrReopened => ChangeKind::Add,
            _ => ChangeKind::Modify,
        },
        colour: None,
        event,
    };
    let record = PrChangeRecord {
        change_index: 0,
        payload: PrPayload::Lifecycle(PrLifecycle {
            number: 8,
            owner,
            target: None,
            created: -86400,
            state: PrState::Open,
            initial,
        }),
    };
    let offset = log.commit_count() as u64 * 100;
    log.push_commit_attr_pr(1, user, &[change], offset, AttrSetId::NONE, &[record])
        .unwrap();
}

fn ordinary(log: &mut EventLog, path: &str, kind: ChangeKind) {
    let path = log.intern(path);
    let user = log.intern("code-actor");
    let offset = log.commit_count() as u64 * 100;
    log.push_commit(
        1,
        user,
        [FileChange {
            path,
            kind,
            colour: None,
            event: ChangeEvent::Commit,
        }],
        offset,
    );
}

fn run(sim: &mut Simulation, log: &EventLog, frames: usize) {
    for _ in 0..frames {
        sim.update(log, 1.0 / 60.0);
    }
}

#[test]
fn initial_rich_seed_is_context_without_actor_avatar() {
    let mut log = EventLog::new();
    rich(&mut log, "proposal-owner", ChangeEvent::PrOpened, true);
    log.set_complete(true);
    let mut sim = Simulation::new(settings());
    run(&mut sim, &log, 2);
    let status = sim.status(&log);
    assert_eq!(
        status.files, 1,
        "seed materializes its existing proposal context"
    );
    assert_eq!(
        status.users, 0,
        "a seed is not a new proposal or actor action"
    );
    assert_eq!(sim.played_set_counts().iter().sum::<u32>(), 0);
}

#[test]
fn open_rich_node_survives_positive_idle_without_keepalive_events() {
    let mut log = EventLog::new();
    rich(&mut log, "proposal-owner", ChangeEvent::PrOpened, false);
    log.set_complete(true);
    let mut settings = settings();
    settings.file_idle_time = 0.05;
    let mut sim = Simulation::new(settings);
    run(&mut sim, &log, 1200);
    assert_eq!(log.commit_count(), 1, "retention adds no keepalive records");
    assert_eq!(
        sim.status(&log).files,
        1,
        "open typed nodes bypass ordinary idle expiry"
    );
}

#[test]
fn actor_filtered_close_still_closes_and_retires_typed_node() {
    let mut log = EventLog::new();
    rich(&mut log, "proposal-owner", ChangeEvent::PrOpened, false);
    rich(&mut log, "filtered-closer", ChangeEvent::PrClosed, false);
    log.set_complete(true);
    let mut settings = settings();
    settings.user_filters.push("^filtered-closer$".into());
    let mut sim = Simulation::new(settings);
    run(&mut sim, &log, 1200);
    assert_eq!(
        sim.status(&log).files,
        0,
        "filtered actors do not filter lifecycle truth"
    );
}

#[test]
fn ancestor_code_delete_keeps_open_typed_proposal() {
    let mut log = EventLog::new();
    rich(&mut log, "proposal-owner", ChangeEvent::PrOpened, false);
    ordinary(&mut log, "/src/code.rs", ChangeKind::Add);
    ordinary(&mut log, "/src/", ChangeKind::Delete);
    log.set_complete(true);
    let mut sim = Simulation::new(settings());
    run(&mut sim, &log, 1200);
    assert_eq!(
        sim.status(&log).files,
        1,
        "only the open typed proposal survives"
    );
}

#[test]
fn legacy_untyped_pr_node_keeps_ordinary_idle_semantics() {
    let mut log = EventLog::new();
    let user = log.intern("legacy-owner");
    let path = log.intern(NODE);
    log.push_commit(
        1,
        user,
        [FileChange {
            path,
            kind: ChangeKind::Add,
            colour: None,
            event: ChangeEvent::PrOpened,
        }],
        0,
    );
    log.set_complete(true);
    let mut settings = settings();
    settings.file_idle_time = 0.05;
    let mut sim = Simulation::new(settings);
    run(&mut sim, &log, 1200);
    assert_eq!(
        sim.status(&log).files,
        0,
        "L0 is not silently promoted to rich mode"
    );
}
