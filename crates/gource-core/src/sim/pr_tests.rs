// SPDX-License-Identifier: GPL-3.0-or-later
//! Real simulation state and action boundaries, without copied lifecycle logic.
use super::*;
use crate::events::{
    PrAssociation, PrChangeRecord, PrCompleteness, PrLifecycle, PrLogMetadata, PrLogUpdate,
    PrPayload, PrState,
};
use crate::pull_request::PrPhase;

const NODE: &str = "/src/.pull-requests/#8";
fn settings() -> Settings {
    Settings {
        auto_skip_seconds: -1.0,
        days_per_second: 0.0,
        file_idle_time: 0.0,
        ..Settings::default()
    }
}
fn lifecycle(
    log: &mut EventLog,
    path: &str,
    number: u64,
    time: i64,
    actor: &str,
    event: ChangeEvent,
    initial: bool,
) -> StrId {
    let path = log.intern(path);
    let actor = log.intern(actor);
    let owner = log.intern("author");
    let target =
        (event == ChangeEvent::PrReviewRequested).then(|| log.intern("requested-reviewer"));
    let kind = match event {
        ChangeEvent::PrMerged | ChangeEvent::PrClosed => ChangeKind::Delete,
        ChangeEvent::PrOpened | ChangeEvent::PrReopened => ChangeKind::Add,
        _ => ChangeKind::Modify,
    };
    let change = FileChange {
        path,
        kind,
        colour: None,
        event,
    };
    let record = PrChangeRecord {
        change_index: 0,
        payload: PrPayload::Lifecycle(PrLifecycle {
            number,
            owner,
            target,
            created: -86400,
            state: PrState::Draft,
            initial,
        }),
    };
    log.push_commit_attr_pr(
        time,
        actor,
        &[change],
        log.commit_count() as u64 * 100,
        AttrSetId::NONE,
        &[record],
    )
    .unwrap();
    path
}
fn code(log: &mut EventLog, path: &str, time: i64, kind: ChangeKind) {
    let path = log.intern(path);
    let actor = log.intern("coder");
    log.push_commit(
        time,
        actor,
        [FileChange {
            path,
            kind,
            colour: None,
            event: ChangeEvent::Commit,
        }],
        log.commit_count() as u64 * 100,
    );
}
#[allow(clippy::too_many_arguments)]
fn association(
    log: &mut EventLog,
    code_path: &str,
    node: &str,
    number: u64,
    time: i64,
    repo: &str,
    sha: &str,
    kind: ChangeKind,
) {
    let path = log.intern(code_path);
    let actor = log.intern("coder");
    let repository = log.intern(repo);
    let source_sha = log.intern(sha);
    let pr_node = log.intern(node);
    let record = PrChangeRecord {
        change_index: 0,
        payload: PrPayload::Association(PrAssociation {
            repository,
            number,
            source_sha,
            pr_node,
        }),
    };
    log.push_commit_attr_pr(
        time,
        actor,
        &[FileChange {
            path,
            kind,
            colour: None,
            event: ChangeEvent::Commit,
        }],
        log.commit_count() as u64 * 100,
        AttrSetId::NONE,
        &[record],
    )
    .unwrap();
}
fn run(sim: &mut Simulation, log: &EventLog, frames: usize) {
    for _ in 0..frames {
        sim.update(log, 1.0 / 60.0);
    }
}
fn present_code(sim: &mut Simulation, log: &EventLog, path: &str) -> FileId {
    for _ in 0..600 {
        if let Some(file) = sim.tree.find_file(path)
            && !sim.tree.files[file].is_hidden()
        {
            return file;
        }
        sim.update(log, 1.0 / 60.0);
    }
    panic!("genuine code create beam did not make its file present");
}
fn coverage(log: &mut EventLog) {
    log.update_pr_log(PrLogUpdate::Metadata(PrLogMetadata {
        version: 1,
        from: -100000,
        to: 100000,
        as_of: 100000,
        lifecycle: PrCompleteness::Complete,
        reviews: PrCompleteness::Complete,
        files: PrCompleteness::Complete,
    }));
}

#[test]
fn coverage_envelope_without_lifecycle_has_no_pr_hud() {
    let mut log = EventLog::new();
    coverage(&mut log);
    code(&mut log, "/src/main.rs", 1, ChangeKind::Add);
    log.set_complete(true);
    let mut sim = Simulation::new(Settings {
        pr_flow_hud: true,
        ..settings()
    });
    run(&mut sim, &log, 2);
    let mut scene = SceneView::default();
    sim.extract_view(&log, &mut scene);
    assert!(scene.hud.pr_flow.is_none());
    assert!(scene.hud.pr_key.is_empty());
    assert!(scene.hud.pr_coverage.is_empty());
    assert!(sim.pr_flow_stats(&log).is_none());
}

#[test]
fn queued_future_lifecycle_is_not_truth_until_consumed() {
    let mut log = EventLog::new();
    let path = lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, false);
    lifecycle(
        &mut log,
        NODE,
        8,
        100,
        "closer",
        ChangeEvent::PrClosed,
        false,
    );
    let mut sim = Simulation::new(settings());
    run(&mut sim, &log, 4);
    assert_eq!(sim.commit_queue.front(), Some(&1));
    assert_eq!(sim.pr_node_by_path(path).unwrap().phase, PrPhase::Draft);
    assert_eq!(sim.pr_node_by_path(path).unwrap().generation, 1);
    assert_eq!(sim.pr_files.len(), 1);
}

#[test]
fn same_second_close_reopen_and_old_review_beams_cannot_mutate_new_generation() {
    let mut log = EventLog::new();
    let path = lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, false);
    let mut sim = Simulation::new(settings());
    run(&mut sim, &log, 2);
    lifecycle(&mut log, NODE, 8, 1, "closer", ChangeEvent::PrClosed, false);
    lifecycle(
        &mut log,
        NODE,
        8,
        1,
        "reopener",
        ChangeEvent::PrReopened,
        false,
    );
    lifecycle(
        &mut log,
        NODE,
        8,
        1,
        "reviewer-one",
        ChangeEvent::ReviewChangesRequested,
        false,
    );
    lifecycle(
        &mut log,
        NODE,
        8,
        1,
        "reviewer-two",
        ChangeEvent::ReviewApproved,
        false,
    );
    run(&mut sim, &log, 2);
    let file = *sim.pr_files.get(&(path, 8)).unwrap();
    let snapshot = *sim.pr_node_by_path(path).unwrap();
    assert_eq!(snapshot.phase, PrPhase::Draft);
    assert_eq!(snapshot.review, Some(ChangeEvent::ReviewApproved));
    assert_eq!(log.str(snapshot.owner), "author");
    assert_eq!(log.str(snapshot.actor), "reviewer-two");
    let before_colour = sim.tree.files[file].file_colour();
    let before_touch = sim.tree.files[file].touch_colour;
    let mut stale = RAction::remove(file, 1, 0.0).with_pr((path, 8), 2);
    stale.logic(3.0, 1, &mut sim.tree);
    assert!(stale.is_finished());
    assert!(!sim.tree.files[file].removing);
    assert_eq!(sim.tree.files[file].touch_colour, before_touch);
    assert_eq!(sim.tree.files[file].file_colour(), before_colour);
    run(&mut sim, &log, 300);
    assert_eq!(*sim.pr_node_by_path(path).unwrap(), snapshot);
    assert!(sim.tree.files.contains(file));
    assert!(!sim.tree.files[file].is_hidden());
}

#[test]
fn seek_prefix_keeps_age_owner_and_review_without_actions_or_terminal_flash() {
    let mut log = EventLog::new();
    let path = lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, false);
    lifecycle(
        &mut log,
        NODE,
        8,
        2,
        "reviewer",
        ChangeEvent::ReviewApproved,
        false,
    );
    lifecycle(&mut log, NODE, 8, 3, "merger", ChangeEvent::PrMerged, false);
    code(&mut log, "/src/later.rs", 4, ChangeKind::Add);
    log.set_complete(true);
    let mut sim = Simulation::new(settings());
    sim.seek_to(&log, log.fraction_at_index(2));
    assert_eq!(sim.cursor, 2);
    let node = sim.pr_node_by_path(path).unwrap();
    assert_eq!(node.phase, PrPhase::Draft);
    assert_eq!(node.review, Some(ChangeEvent::ReviewApproved));
    assert_eq!(node.age_days(86400), 2);
    assert_eq!(node.terminal_started, None);
    assert_eq!(sim.users.len(), 0);
    assert_eq!(sim.pr.new_proposals, 0);
    assert_eq!(sim.pr_files.len(), 1);
    sim.seek_to(&log, log.fraction_at_index(3));
    assert_eq!(sim.pr_node_by_path(path).unwrap().phase, PrPhase::Merged);
    assert_eq!(sim.pr_node_by_path(path).unwrap().terminal_started, None);
    assert_eq!(sim.pr_files.len(), 0);
}

#[test]
fn straight_play_start_and_seek_match_lifecycle_age_and_hud_at_boundary() {
    let mut log = EventLog::new();
    let path = lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, false);
    lifecycle(
        &mut log,
        NODE,
        8,
        2,
        "reviewer",
        ChangeEvent::ReviewApproved,
        false,
    );
    lifecycle(
        &mut log,
        NODE,
        8,
        3,
        "ready-actor",
        ChangeEvent::PrReady,
        false,
    );
    code(&mut log, "/src/later.rs", 86400, ChangeKind::Add);
    coverage(&mut log);
    log.set_complete(true);
    let mut s = settings();
    s.pr_flow_hud = true;
    let mut played = Simulation::new(s.clone());
    for time in [1, 2, 3, 86400] {
        played.currtime = time;
        run(&mut played, &log, 1);
    }
    let mut seeded_settings = s.clone();
    seeded_settings.start_timestamp = 86400;
    let mut started = Simulation::new(seeded_settings);
    run(&mut started, &log, 1);
    let mut sought = Simulation::new(s);
    sought.seek(&log, log.fraction_at_index(3));
    run(&mut sought, &log, 1);
    let truth = *played.pr_node_by_path(path).unwrap();
    let flow = played.pr_flow_stats(&log);
    let mut expected_view = SceneView::default();
    played.extract_view(&log, &mut expected_view);
    let expected_hud = expected_view
        .str(expected_view.hud.pr_flow.unwrap())
        .to_owned();
    for sim in [&mut started, &mut sought] {
        let node = *sim.pr_node_by_path(path).unwrap();
        assert_eq!(
            node, truth,
            "same frozen lifecycle snapshot, not full physics"
        );
        assert_eq!(node.age_days(sim.currtime), truth.age_days(played.currtime));
        assert_eq!(sim.pr_flow_stats(&log), flow);
        assert_eq!(sim.pr.new_proposals, 0);
        assert!(
            sim.user_map.keys().all(|name| name == "coder"),
            "no seeded PR actors"
        );
        let mut view = SceneView::default();
        sim.extract_view(&log, &mut view);
        assert_eq!(view.str(view.hud.pr_flow.unwrap()), expected_hud);
    }
}

#[test]
fn actor_and_path_filters_do_not_change_complete_repository_flow() {
    let mut log = EventLog::new();
    let path = lifecycle(
        &mut log,
        NODE,
        8,
        1,
        "hidden-author",
        ChangeEvent::PrOpened,
        false,
    );
    lifecycle(
        &mut log,
        NODE,
        8,
        1,
        "hidden-reviewer",
        ChangeEvent::PrReady,
        false,
    );
    coverage(&mut log);
    let mut s = settings();
    s.user_filters.push("^hidden-".into());
    s.file_filters.push("pull-requests".into());
    let mut sim = Simulation::new(s);
    run(&mut sim, &log, 4);
    assert_eq!(sim.pr_node_by_path(path).unwrap().phase, PrPhase::Open);
    assert_eq!(sim.pr_flow_stats(&log).unwrap().open, 1);
    assert_eq!(sim.pr_files.len(), 0);
    assert_eq!(sim.users.len(), 0);
}

#[test]
fn missing_first_review_never_invents_a_proposal_or_complete_hud() {
    let mut log = EventLog::new();
    lifecycle(
        &mut log,
        NODE,
        8,
        1,
        "reviewer",
        ChangeEvent::ReviewApproved,
        false,
    );
    coverage(&mut log);
    let mut sim = Simulation::new(settings());
    run(&mut sim, &log, 4);
    assert_eq!(sim.pr_nodes().len(), 0);
    assert_eq!(sim.pr_files.len(), 0);
    assert_eq!(sim.pr.new_proposals, 0);
    assert_eq!(sim.pr_flow_stats(&log), None);
}

#[test]
fn real_code_exact_and_prefix_collisions_fail_closed_without_protecting_code() {
    for conflict in [
        NODE.to_owned(),
        "/src/.pull-requests".to_owned(),
        format!("{NODE}/real.rs"),
    ] {
        let mut log = EventLog::new();
        lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, false);
        code(&mut log, &conflict, 2, ChangeKind::Add);
        coverage(&mut log);
        let mut sim = Simulation::new(settings());
        run(&mut sim, &log, 4);
        assert_eq!(sim.pr_files.len(), 0, "future real path already conflicts");
        assert_eq!(sim.pr_flow_stats(&log), None);
        sim.currtime = 2;
        run(&mut sim, &log, 4);
        let file = sim.tree.find_file(&conflict).unwrap();
        assert_eq!(sim.tree.files[file].pr_key, None);
    }
}

#[test]
fn lexical_sibling_cannot_mask_real_code_descendant_collision() {
    let mut log = EventLog::new();
    lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, false);
    let sibling = format!("{NODE}-extra");
    let descendant = format!("{NODE}/real.rs");
    code(&mut log, &sibling, 2, ChangeKind::Add);
    code(&mut log, &descendant, 2, ChangeKind::Add);
    coverage(&mut log);
    let mut sim = Simulation::new(settings());
    run(&mut sim, &log, 4);
    assert_eq!(
        sim.pr_files.len(),
        0,
        "earlier lexical sibling must not hide the real directory-prefix conflict"
    );
    assert_eq!(sim.pr_flow_stats(&log), None);
    sim.currtime = 2;
    run(&mut sim, &log, 4);
    for path in [&sibling, &descendant] {
        let file = sim.tree.find_file(path).unwrap();
        assert_eq!(sim.tree.files[file].pr_key, None);
        assert!(!sim.tree.files[file].pr_pinned);
    }
}

#[test]
fn hide_prs_scene_matches_pr_stripped_input_including_clock_and_review_avatars() {
    let mut rich = EventLog::new();
    lifecycle(
        &mut rich,
        NODE,
        8,
        1,
        "author",
        ChangeEvent::PrOpened,
        false,
    );
    code(&mut rich, "/src/code.rs", 10, ChangeKind::Add);
    lifecycle(
        &mut rich,
        NODE,
        8,
        100,
        "reviewer",
        ChangeEvent::ReviewApproved,
        false,
    );
    rich.set_complete(true);
    let mut stripped = EventLog::new();
    code(&mut stripped, "/src/code.rs", 10, ChangeKind::Add);
    stripped.set_complete(true);
    let mut s = settings();
    s.hide_pull_requests = true;
    let mut a = Simulation::new(s.clone());
    let mut b = Simulation::new(s);
    let mut av = SceneView::default();
    let mut bv = SceneView::default();
    for _ in 0..180 {
        a.update(&rich, 1.0 / 60.0);
        b.update(&stripped, 1.0 / 60.0);
        a.extract_view(&rich, &mut av);
        b.extract_view(&stripped, &mut bv);
        assert_eq!(
            crate::view_hash::hash_view(&av),
            crate::view_hash::hash_view(&bv)
        );
        assert_eq!(a.currtime, b.currtime);
    }
}

#[test]
fn exact_association_flashes_only_existing_code_without_touching_it() {
    let mut log = EventLog::new();
    let path = lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, false);
    code(&mut log, "/src/code.rs", 1, ChangeKind::Add);
    let mut sim = Simulation::new(settings());
    run(&mut sim, &log, 5);
    let file = present_code(&mut sim, &log, "/src/code.rs");
    association(
        &mut log,
        "/src/code.rs",
        NODE,
        8,
        1,
        "owner/repo",
        &"a".repeat(40),
        ChangeKind::Modify,
    );
    lifecycle(&mut log, NODE, 8, 1, "merger", ChangeEvent::PrMerged, false);
    run(&mut sim, &log, 1);
    assert_eq!(sim.pr_node_by_path(path).unwrap().phase, PrPhase::Merged);
    assert!(
        sim.pr_flash_age_for_file(file).is_some(),
        "qualified existing code has a bounded style flash"
    );
    assert_eq!(sim.tree.files[file].pr_key, None);
    run(&mut sim, &log, 150);
    assert_eq!(sim.pr_flash_age_for_file(file), None);
}

#[test]
fn association_without_present_file_does_not_invent_flash_or_file() {
    let mut log = EventLog::new();
    lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, false);
    association(
        &mut log,
        "/src/missing.rs",
        NODE,
        8,
        1,
        "owner/repo",
        &"a".repeat(40),
        ChangeKind::Modify,
    );
    lifecycle(&mut log, NODE, 8, 1, "merger", ChangeEvent::PrMerged, false);
    let mut sim = Simulation::new(settings());
    sim.process_commit(&log, 0, 0.0);
    // Association annotates a genuine existing commit but its visual receipt must
    // never materialize the associated path by itself.
    let before = sim.tree.files.len();
    sim.register_pr_association(
        &log,
        log.commit(1).changes[0].path,
        match log
            .pr_change(log.records()[1].first_change as usize)
            .unwrap()
        {
            PrPayload::Association(value) => *value,
            _ => unreachable!(),
        },
    );
    assert_eq!(sim.tree.files.len(), before);
    assert!(sim.tree.find_file("/src/missing.rs").is_none());
}

#[test]
fn request_target_is_not_actor_or_owner_and_ready_clears_draft() {
    let mut log = EventLog::new();
    let path = lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, true);
    lifecycle(
        &mut log,
        NODE,
        8,
        1,
        "requester",
        ChangeEvent::PrReviewRequested,
        false,
    );
    let mut sim = Simulation::new(settings());
    run(&mut sim, &log, 2);
    let node = sim.pr_node_by_path(path).unwrap();
    assert_eq!(log.str(node.owner), "author");
    assert_eq!(log.str(node.actor), "requester");
    assert_eq!(log.str(node.target.unwrap()), "requested-reviewer");
    assert!(!sim.user_map.contains_key("author"));
    assert!(!sim.user_map.contains_key("requested-reviewer"));
    assert_eq!(sim.pr.new_proposals, 0);
    assert_eq!(sim.played_set_counts.iter().sum::<u32>(), 1);
    lifecycle(
        &mut log,
        NODE,
        8,
        1,
        "ready-actor",
        ChangeEvent::PrReady,
        false,
    );
    run(&mut sim, &log, 2);
    let node = sim.pr_node_by_path(path).unwrap();
    assert_eq!(node.phase, PrPhase::Open);
    assert_eq!(node.target, None);
    assert_eq!(log.str(node.actor), "ready-actor");
}

#[test]
fn terminal_envelope_is_two_animation_seconds_not_replay_days_or_beam_completion() {
    let mut log = EventLog::new();
    let path = lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, false);
    lifecycle(&mut log, NODE, 8, 1, "merger", ChangeEvent::PrMerged, false);
    lifecycle(
        &mut log,
        NODE,
        8,
        1,
        "transport-companion",
        ChangeEvent::PrClosed,
        false,
    );
    let mut sim = Simulation::new(settings());
    run(&mut sim, &log, 2);
    let node = *sim.pr_node_by_path(path).unwrap();
    assert_eq!(node.phase, PrPhase::Merged);
    assert_eq!(log.str(node.actor), "merger");
    assert_eq!(node.generation, 2);
    let file = *sim.pr_files.get(&(path, 8)).unwrap();
    for &user in sim.user_map.values() {
        for action in &mut sim.users[user].actions {
            action.rate = 0.001;
        }
        for action in &mut sim.users[user].active_actions {
            action.rate = 0.001;
        }
    }
    sim.currtime = 86_400 * 1000;
    run(&mut sim, &log, 60);
    assert!(sim.tree.files.contains(file));
    assert!(node.terminal_age(sim.runtime).unwrap() < 2.0);
    run(&mut sim, &log, 65);
    assert!(!sim.tree.files.contains(file));
    assert_eq!(sim.pr_node_by_path(path).unwrap().phase, PrPhase::Merged);
}

#[test]
fn start_date_and_start_position_fold_unfiltered_prefix_without_seed_actions() {
    let mut log = EventLog::new();
    let path = lifecycle(
        &mut log,
        NODE,
        8,
        1,
        "filtered-author",
        ChangeEvent::PrOpened,
        false,
    );
    lifecycle(
        &mut log,
        NODE,
        8,
        2,
        "reviewer",
        ChangeEvent::ReviewApproved,
        false,
    );
    code(&mut log, "/src/code.rs", 3, ChangeKind::Add);
    log.set_complete(true);
    for by_position in [false, true] {
        let mut s = settings();
        s.user_filters.push("^filtered-author$".into());
        if by_position {
            s.start_position = log.fraction_at_index(2);
        } else {
            s.start_timestamp = 3;
        }
        let mut sim = Simulation::new(s);
        run(&mut sim, &log, 2);
        let node = sim.pr_node_by_path(path).unwrap();
        assert_eq!(node.created, -86400);
        assert_eq!(node.review, Some(ChangeEvent::ReviewApproved));
        assert_eq!(node.generation, 2, "prefix folded exactly once");
        assert!(!sim.user_map.contains_key("filtered-author"));
        assert!(!sim.user_map.contains_key("reviewer"));
        assert_eq!(sim.pr.new_proposals, 0);
        assert_eq!(sim.pr_files.len(), 1);
    }
}

#[test]
fn start_date_after_all_activity_sets_seeded_context_replay_age_without_actions() {
    let mut log = EventLog::new();
    let path = lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, false);
    lifecycle(
        &mut log,
        NODE,
        8,
        2,
        "reviewer",
        ChangeEvent::ReviewApproved,
        false,
    );
    coverage(&mut log);
    log.set_complete(true);
    let mut s = settings();
    s.start_timestamp = 86400;
    s.pr_flow_hud = true;
    let mut sim = Simulation::new(s);
    run(&mut sim, &log, 2);
    assert_eq!(
        sim.currtime, 86400,
        "context uses the requested replay boundary"
    );
    let node = sim.pr_node_by_path(path).unwrap();
    assert_eq!(node.age_days(sim.currtime), 2);
    assert_eq!(node.review, Some(ChangeEvent::ReviewApproved));
    let flow = sim.pr.flow_stats(&log, sim.currtime).unwrap();
    assert_eq!(flow.open, 1);
    assert_eq!(flow.oldest_days, Some(2));
    assert_eq!(sim.pr.new_proposals, 0);
    assert!(
        sim.user_map.is_empty(),
        "prefix seeds make no actor or beam"
    );
    assert_eq!(sim.pr_files.len(), 1);
}

#[test]
fn actionless_start_stop_window_retains_seeded_boundary_clock() {
    let mut log = EventLog::new();
    let path = lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, false);
    code(&mut log, "/src/later.rs", 172800, ChangeKind::Add);
    coverage(&mut log);
    log.set_complete(true);
    let mut s = settings();
    s.start_timestamp = 86400;
    s.stop_timestamp = 129600;
    s.pr_flow_hud = true;
    let mut sim = Simulation::new(s);
    run(&mut sim, &log, 2);
    assert_eq!(sim.currtime, 86400);
    assert_eq!(sim.pr_node_by_path(path).unwrap().age_days(sim.currtime), 2);
    assert_eq!(sim.pr_flow_stats(&log).unwrap().open, 1);
    assert_eq!(sim.pr.new_proposals, 0);
    assert!(sim.user_map.is_empty());
    assert!(sim.tree.find_file("/src/later.rs").is_none());
}

#[test]
fn duplicate_numbers_across_repo_nodes_require_explicit_node_and_exact_qualifiers() {
    let mut log = EventLog::new();
    let one = lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, false);
    let other_path = "/other/.pull-requests/#8";
    let other = lifecycle(
        &mut log,
        other_path,
        8,
        1,
        "author",
        ChangeEvent::PrOpened,
        false,
    );
    code(&mut log, "/src/one.rs", 1, ChangeKind::Add);
    code(&mut log, "/other/two.rs", 1, ChangeKind::Add);
    let mut sim = Simulation::new(settings());
    run(&mut sim, &log, 5);
    let one_file = present_code(&mut sim, &log, "/src/one.rs");
    let two_file = present_code(&mut sim, &log, "/other/two.rs");
    association(
        &mut log,
        "/src/one.rs",
        NODE,
        8,
        1,
        "owner/one",
        &"a".repeat(40),
        ChangeKind::Modify,
    );
    association(
        &mut log,
        "/other/two.rs",
        other_path,
        8,
        1,
        "owner/two",
        &"b".repeat(40),
        ChangeKind::Modify,
    );
    lifecycle(&mut log, NODE, 8, 1, "merger", ChangeEvent::PrMerged, false);
    run(&mut sim, &log, 1);
    assert!(sim.pr_flash_age_for_file(one_file).is_some());
    assert_eq!(sim.pr_flash_age_for_file(two_file), None);
    assert_eq!(sim.pr_node_by_path(one).unwrap().phase, PrPhase::Merged);
    assert_eq!(sim.pr_node_by_path(other).unwrap().phase, PrPhase::Draft);
    // A competing SHA/repository receipt never last-wins or transfers number-only truth.
    association(
        &mut log,
        "/src/one.rs",
        NODE,
        8,
        1,
        "different/repo",
        &"c".repeat(40),
        ChangeKind::Modify,
    );
    run(&mut sim, &log, 1);
    assert_eq!(sim.pr_flash_age_for_file(one_file), None);
}

#[test]
fn receipt_never_attaches_to_a_deleted_recreated_or_hidden_file_generation() {
    let mut log = EventLog::new();
    lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, false);
    code(&mut log, "/src/code.rs", 1, ChangeKind::Add);
    let mut sim = Simulation::new(settings());
    run(&mut sim, &log, 5);
    let old = present_code(&mut sim, &log, "/src/code.rs");
    association(
        &mut log,
        "/src/code.rs",
        NODE,
        8,
        1,
        "owner/repo",
        &"a".repeat(40),
        ChangeKind::Modify,
    );
    run(&mut sim, &log, 1);
    sim.delete_file(old);
    let change = log.commit(1).changes[0];
    let new = sim.add_file(&log, &change).unwrap();
    sim.tree.set_file_hidden(new, false);
    assert_ne!(old, new);
    lifecycle(&mut log, NODE, 8, 1, "merger", ChangeEvent::PrMerged, false);
    run(&mut sim, &log, 1);
    assert_eq!(sim.pr_flash_age_for_file(old), None);
    assert_eq!(sim.pr_flash_age_for_file(new), None);
    let receipt = match *log
        .pr_change(log.records()[2].first_change as usize)
        .unwrap()
    {
        PrPayload::Association(value) => value,
        _ => unreachable!(),
    };
    sim.tree.set_file_hidden(new, true);
    sim.register_pr_association(&log, change.path, receipt);
    sim.tree.set_file_hidden(new, false);
    assert_eq!(sim.pr_flash_age_for_file(new), None);
}

#[test]
fn receipt_registration_and_extraction_never_touch_or_recolour_code() {
    let mut log = EventLog::new();
    lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, false);
    code(&mut log, "/src/code.rs", 1, ChangeKind::Add);
    let mut sim = Simulation::new(settings());
    run(&mut sim, &log, 5);
    let file = present_code(&mut sim, &log, "/src/code.rs");
    association(
        &mut log,
        "/src/code.rs",
        NODE,
        8,
        1,
        "owner/repo",
        &"a".repeat(40),
        ChangeKind::Modify,
    );
    let receipt = match *log
        .pr_change(log.records()[2].first_change as usize)
        .unwrap()
    {
        PrPayload::Association(value) => value,
        _ => unreachable!(),
    };
    let before = sim.tree.files[file].clone();
    sim.register_pr_association(&log, log.commit(1).changes[0].path, receipt);
    assert_eq!(sim.tree.files[file], before);
    lifecycle(&mut log, NODE, 8, 1, "merger", ChangeEvent::PrMerged, false);
    sim.process_commit(&log, 3, sim.runtime);
    let before = sim.tree.files[file].clone();
    for _ in 0..100 {
        assert!(sim.pr_flash_age_for_file(file).is_some());
    }
    assert_eq!(sim.tree.files[file], before);
}

#[test]
fn hovered_pr_is_read_only_typed_context_and_suppressed_when_hidden() {
    let mut log = EventLog::new();
    let path = lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, true);
    let mut sim = Simulation::new(settings());
    run(&mut sim, &log, 2);
    assert_eq!(sim.hovered_pr(), None);
    sim.hover_file = sim.pr_files.get(&(path, 8)).copied();
    assert_eq!(sim.hovered_pr(), sim.pr_node_by_path(path));
    let before = *sim.hovered_pr().unwrap();
    sim.settings.hide_pull_requests = true;
    assert_eq!(sim.hovered_pr(), None);
    assert_eq!(*sim.pr_node_by_path(path).unwrap(), before);
    sim.settings.hide_pull_requests = false;
    let stale = sim.hover_file.unwrap();
    sim.delete_file(stale);
    sim.hover_file = Some(stale);
    assert_eq!(sim.hovered_pr(), None);
}

#[test]
fn filtered_association_is_no_flash_without_restoring_code_but_merge_truth_survives() {
    let mut log = EventLog::new();
    let path = lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, false);
    code(&mut log, "/src/code.rs", 1, ChangeKind::Add);
    coverage(&mut log);
    let mut sim = Simulation::new(settings());
    let file = present_code(&mut sim, &log, "/src/code.rs");
    sim.settings.user_filters.push("^(coder|merger)$".into());
    sim.filters.sync(&sim.settings);
    association(
        &mut log,
        "/src/code.rs",
        NODE,
        8,
        1,
        "owner/repo",
        &"a".repeat(40),
        ChangeKind::Modify,
    );
    lifecycle(&mut log, NODE, 8, 1, "merger", ChangeEvent::PrMerged, false);
    let action_count = sim.commit_seq;
    run(&mut sim, &log, 2);
    assert_eq!(sim.pr_node_by_path(path).unwrap().phase, PrPhase::Merged);
    assert_eq!(sim.pr_flow_stats(&log).unwrap().open, 0);
    assert_eq!(sim.pr_flash_age_for_file(file), None);
    assert_eq!(
        sim.commit_seq, action_count,
        "filters restore no genuine code or PR actions"
    );
    assert!(!sim.user_map.contains_key("merger"));
    assert_eq!(sim.tree.files[file].pr_key, None);
}

#[test]
fn duplicate_wire_path_and_owner_ids_are_one_rich_semantic_node() {
    let mut log = EventLog::new();
    let canonical_path = lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, false);
    let canonical_owner = log.lookup("author").unwrap();
    let alias_path = log.push_string(NODE);
    let alias_owner = log.push_string("author");
    assert_ne!(canonical_path, alias_path);
    assert_ne!(canonical_owner, alias_owner);
    for event in [ChangeEvent::ReviewApproved, ChangeEvent::PrClosed] {
        let actor = log.intern(if event == ChangeEvent::PrClosed {
            "closer"
        } else {
            "reviewer"
        });
        let record = PrChangeRecord {
            change_index: 0,
            payload: PrPayload::Lifecycle(PrLifecycle {
                number: 8,
                owner: alias_owner,
                target: None,
                created: -86400,
                state: PrState::Draft,
                initial: false,
            }),
        };
        log.push_commit_attr_pr(
            1,
            actor,
            &[FileChange {
                path: alias_path,
                kind: if event == ChangeEvent::PrClosed {
                    ChangeKind::Delete
                } else {
                    ChangeKind::Modify
                },
                event,
                colour: None,
            }],
            log.commit_count() as u64 * 100,
            AttrSetId::NONE,
            &[record],
        )
        .unwrap();
    }
    coverage(&mut log);
    let mut sim = Simulation::new(settings());
    run(&mut sim, &log, 3);
    assert_eq!(
        sim.pr_nodes().len(),
        1,
        "wire aliases cannot split the semantic lifecycle table"
    );
    let node = sim.pr_node_by_path(canonical_path).unwrap();
    assert_eq!(node.phase, PrPhase::Closed);
    assert_eq!(node.review, Some(ChangeEvent::ReviewApproved));
    assert_eq!(node.owner, canonical_owner);
    assert_eq!(node.generation, 3);
    assert_eq!(sim.pr_node_by_path(alias_path), Some(node));
    assert_eq!(
        sim.pr_flow_stats(&log).unwrap().open,
        0,
        "equivalent wire IDs do not invalidate coverage"
    );
    assert_eq!(sim.pr_files.len(), 1);
    assert_eq!(sim.pr.new_proposals, 1);
    // The original frozen wire representation remains untouched for v3 round-trip.
    assert_eq!(log.commit(1).changes[0].path, alias_path);
    assert_eq!(
        match log
            .pr_change(log.records()[1].first_change as usize)
            .unwrap()
        {
            PrPayload::Lifecycle(value) => value.owner,
            _ => unreachable!(),
        },
        alias_owner
    );
    run(&mut sim, &log, 150);
    assert_eq!(sim.pr_files.len(), 0);
}

#[test]
fn duplicate_wire_node_repository_and_sha_ids_keep_exact_receipt_binding() {
    let mut log = EventLog::new();
    lifecycle(&mut log, NODE, 8, 1, "author", ChangeEvent::PrOpened, false);
    code(&mut log, "/src/code.rs", 1, ChangeKind::Add);
    let mut sim = Simulation::new(settings());
    let file = present_code(&mut sim, &log, "/src/code.rs");
    let sha_text = "a".repeat(40);
    association(
        &mut log,
        "/src/code.rs",
        NODE,
        8,
        1,
        "owner/repo",
        &sha_text,
        ChangeKind::Modify,
    );
    let alias_node = log.push_string(NODE);
    let alias_repo = log.push_string("owner/repo");
    let alias_sha = log.push_string(&sha_text);
    let path = log.lookup("/src/code.rs").unwrap();
    let actor = log.lookup("coder").unwrap();
    let aliased = PrAssociation {
        repository: alias_repo,
        number: 8,
        source_sha: alias_sha,
        pr_node: alias_node,
    };
    log.push_commit_attr_pr(
        1,
        actor,
        &[FileChange {
            path,
            kind: ChangeKind::Modify,
            colour: None,
            event: ChangeEvent::Commit,
        }],
        log.commit_count() as u64 * 100,
        AttrSetId::NONE,
        &[PrChangeRecord {
            change_index: 0,
            payload: PrPayload::Association(aliased),
        }],
    )
    .unwrap();
    lifecycle(&mut log, NODE, 8, 1, "merger", ChangeEvent::PrMerged, false);
    run(&mut sim, &log, 2);
    assert!(
        sim.pr_flash_age_for_file(file).is_some(),
        "exact same qualifier text is not ambiguous solely because wire IDs differ"
    );
    assert_eq!(sim.pr_nodes().len(), 1);
    assert_eq!(
        match log
            .pr_change(log.records()[3].first_change as usize)
            .unwrap()
        {
            PrPayload::Association(value) => *value,
            _ => unreachable!(),
        },
        aliased,
        "original imported receipt IDs remain immutable"
    );
}
