// SPDX-License-Identifier: GPL-3.0-or-later
//! Real Simulation extraction checks. No copied lifecycle or rendering model.
use gource_core::events::{
    AttrSetId, ChangeEvent, EventLog, FileChange, PrChangeRecord, PrCompleteness, PrLifecycle,
    PrLogMetadata, PrLogUpdate, PrPayload, PrState,
};
use gource_core::settings::Settings;
use gource_core::sim::Simulation;
use gource_core::view::SceneView;

fn log(lifecycle: PrCompleteness, review: bool) -> EventLog {
    let mut log = EventLog::new();
    let path = log.intern("/src/.pull-requests/#8 Frozen proposal");
    let owner = log.intern("Owner");
    let reviewer = log.intern("Reviewer");
    for (index, event) in [ChangeEvent::PrOpened, ChangeEvent::ReviewApproved]
        .into_iter()
        .enumerate()
    {
        if index == 1 && !review {
            break;
        }
        let actor = if index == 0 { owner } else { reviewer };
        let change = FileChange {
            path,
            kind: event.pr_kind().unwrap(),
            colour: None,
            event,
        };
        let metadata = PrChangeRecord {
            change_index: 0,
            payload: PrPayload::Lifecycle(PrLifecycle {
                number: 8,
                owner,
                target: None,
                created: -86400,
                state: PrState::Open,
                initial: index == 0,
            }),
        };
        log.push_commit_attr_pr(
            1,
            actor,
            &[change],
            index as u64 * 100,
            AttrSetId::NONE,
            &[metadata],
        )
        .unwrap();
    }
    log.update_pr_log(PrLogUpdate::Metadata(PrLogMetadata {
        version: 1,
        from: -172800,
        to: 172800,
        as_of: 1,
        lifecycle,
        reviews: PrCompleteness::Partial,
        files: PrCompleteness::Unknown,
    }));
    log.set_complete(true);
    log
}

fn view(log: &EventLog, hud: bool) -> SceneView {
    let mut sim = Simulation::new(Settings {
        auto_skip_seconds: -1.0,
        days_per_second: 0.0,
        pr_flow_hud: hud,
        ..Settings::default()
    });
    sim.set_viewport(390, 384, 1.0);
    for _ in 0..120 {
        sim.update(log, 1.0 / 60.0);
    }
    let mut view = SceneView::default();
    sim.extract_view(log, &mut view);
    view
}

#[test]
fn seeded_proposal_has_distinct_dimmed_marker_short_age_label_and_recordable_key() {
    let view = view(&log(PrCompleteness::Complete, false), false);
    assert_eq!(view.users.len(), 0, "seeds have no actor actions");
    assert_eq!(
        view.files.len(),
        3,
        "PR marker uses linked discs in the existing file batch"
    );
    assert!(view.files.iter().all(|f| f.alpha > 0.0 && f.alpha < 1.0));
    assert!(
        view.labels
            .iter()
            .any(|label| view.str(label.text) == "P #8 1d")
    );
    assert!(!view.hud.pr_key.is_empty());
    assert!(view.hud.pr_flow.is_none(), "repository flow defaults off");
    assert_eq!(view.hud.pr_coverage.len(), 3);
    let coverage: Vec<_> = view.hud.pr_coverage.iter().map(|&r| view.str(r)).collect();
    assert_eq!(
        coverage,
        ["Lifecycle: complete", "Reviews: partial", "Files: unknown"]
    );
    let label = view
        .labels
        .iter()
        .find(|label| view.str(label.text) == "P #8 1d")
        .unwrap();
    let gource_core::view::LabelAnchor::WorldTopLeft { pos } = label.anchor else {
        panic!("proposal label remains in the existing file-name pass");
    };
    let viewport = gource_core::glam::Vec2::new(390.0, 384.0);
    let marker_y = view
        .camera
        .project(viewport, view.files[0].pos.extend(0.0))
        .y;
    let label_y = view.camera.project(viewport, pos.extend(0.0)).y;
    assert!(
        label_y >= marker_y + 16.0,
        "PR label must clear the marker/directory label band: marker {marker_y}, label {label_y}"
    );
}

#[test]
fn review_verdict_is_not_colour_only_and_repository_flow_requires_complete_lifecycle() {
    let complete = view(&log(PrCompleteness::Complete, true), true);
    assert!(
        complete
            .labels
            .iter()
            .any(|label| complete.str(label.text) == "A #8 1d")
    );
    let flow = complete
        .hud
        .pr_flow
        .expect("complete lifecycle permits honest repository summary");
    assert_eq!(complete.str(flow), "PRs (repository): 1 open, oldest 1d");
    let partial = view(&log(PrCompleteness::Partial, true), true);
    assert!(
        partial.hud.pr_flow.is_none(),
        "partial coverage never invents repository counts"
    );
}

#[test]
fn proposal_label_clearance_uses_file_directory_fonts_and_device_scale_not_hud_font() {
    use gource_core::glam::Vec2;
    use gource_core::input::{InputEvent, PointerButton};
    for (font_size, filename_font_size, dirname_font_size, scale) in
        [(1, 14, 14, 1.0), (14, 48, 40, 1.0), (1, 14, 14, 2.0)]
    {
        let log = log(PrCompleteness::Complete, false);
        let mut sim = Simulation::new(Settings {
            auto_skip_seconds: -1.0,
            days_per_second: 0.0,
            font_size,
            filename_font_size,
            dirname_font_size,
            ..Settings::default()
        });
        let viewport = Vec2::new(390.0, 384.0) * scale;
        sim.set_viewport(viewport.x as u32, viewport.y as u32, scale);
        for _ in 0..120 {
            sim.update(&log, 1.0 / 60.0);
        }
        let mut view = SceneView::default();
        sim.extract_view(&log, &mut view);
        let label = view
            .labels
            .iter()
            .find(|label| view.str(label.text) == "P #8 1d")
            .unwrap();
        let gource_core::view::LabelAnchor::WorldTopLeft { pos } = label.anchor else {
            unreachable!()
        };
        let marker_y = view
            .camera
            .project(viewport, view.files[0].pos.extend(0.0))
            .y;
        let label_y = view.camera.project(viewport, pos.extend(0.0)).y;
        let minimum = sim
            .settings()
            .scaled_filename_font_size()
            .max(sim.settings().scaled_dirname_font_size())
            .max(18) as f32
            * scale;
        assert!(
            label_y >= marker_y + minimum,
            "independent fonts/DPR must preserve row clearance: {font_size}/{filename_font_size}/{dirname_font_size}/{scale}: {marker_y} -> {label_y}"
        );
        let point = view.camera.project(viewport, view.files[0].pos.extend(0.0));
        sim.handle_input(InputEvent::PointerDown {
            x: point.x,
            y: point.y,
            button: PointerButton::Left,
        });
        sim.extract_view(&log, &mut view);
        assert!(
            view.labels
                .iter()
                .any(|label| label.font == gource_core::view::FontRole::FileSelected),
            "actual pointer selects the proposal"
        );
        let old_z = view.camera.pos.z;
        for _ in 0..6 {
            sim.handle_input(InputEvent::Wheel { delta: -1.0 });
        }
        for _ in 0..120 {
            sim.update(&log, 1.0 / 60.0);
        }
        sim.extract_view(&log, &mut view);
        assert!(
            (view.camera.pos.z - old_z).abs() > 1.0,
            "actual manual zoom changed projection"
        );
        let selected = view
            .labels
            .iter()
            .find(|label| label.font == gource_core::view::FontRole::FileSelected)
            .unwrap();
        let gource_core::view::LabelAnchor::WorldTopLeft { pos } = selected.anchor else {
            unreachable!()
        };
        let marker_y = view
            .camera
            .project(viewport, view.files[0].pos.extend(0.0))
            .y;
        let label_y = view.camera.project(viewport, pos.extend(0.0)).y;
        assert!(
            label_y >= marker_y + minimum - 0.01,
            "selection and zoom preserve actual row clearance: {marker_y} -> {label_y}"
        );
    }
}

#[test]
fn pr_hud_is_fingerprinted_and_clearing_it_restores_legacy_hash() {
    use gource_core::view_hash::hash_view;
    let mut view = SceneView::default();
    let plain = hash_view(&view);
    let line = view.push_text("Lifecycle: partial");
    view.hud.pr_coverage.push(line);
    assert_ne!(
        hash_view(&view),
        plain,
        "coverage is visible recording state"
    );
    view.hud.pr_coverage.clear();
    assert_eq!(
        hash_view(&view),
        plain,
        "unused text and absent PR HUD preserve L0 hashes"
    );
    view.hud.pr_key.push(line);
    assert_ne!(
        hash_view(&view),
        plain,
        "key meanings participate in replay fingerprints"
    );
    view.hud.pr_key.clear();
    view.hud.pr_flow = Some(line);
    assert_ne!(
        hash_view(&view),
        plain,
        "repository summary participates in replay fingerprints"
    );
}

#[test]
fn later_request_ready_and_reopen_do_not_attribute_old_review_to_latest_actor() {
    use glam::Vec2;
    use gource_core::input::InputEvent;
    for event in [
        ChangeEvent::PrReviewRequested,
        ChangeEvent::PrReady,
        ChangeEvent::PrReopened,
    ] {
        let mut log = log(PrCompleteness::Complete, true);
        let path = log
            .lookup("/src/.pull-requests/#8 Frozen proposal")
            .unwrap();
        let owner = log.lookup("Owner").unwrap();
        let actor = log.intern("Requester");
        let target = log.intern("Target reviewer");
        log.push_commit_attr_pr(
            1,
            actor,
            &[FileChange {
                path,
                kind: event.pr_kind().unwrap(),
                colour: None,
                event,
            }],
            200,
            AttrSetId::NONE,
            &[PrChangeRecord {
                change_index: 0,
                payload: PrPayload::Lifecycle(PrLifecycle {
                    number: 8,
                    owner,
                    target: (event == ChangeEvent::PrReviewRequested).then_some(target),
                    created: -86400,
                    state: PrState::Open,
                    initial: false,
                }),
            }],
        )
        .unwrap();
        let mut sim = Simulation::new(Settings {
            auto_skip_seconds: -1.0,
            days_per_second: 0.0,
            hide_users: true,
            ..Settings::default()
        });
        sim.set_viewport(390, 384, 1.0);
        for _ in 0..120 {
            sim.update(&log, 1.0 / 60.0);
        }
        let mut view = SceneView::default();
        sim.extract_view(&log, &mut view);
        let file = view
            .files
            .first()
            .expect("open proposal remains materialized");
        let pointer = view
            .camera
            .project(Vec2::new(390.0, 384.0), file.pos.extend(0.0));
        sim.handle_input(InputEvent::PointerMove {
            x: pointer.x,
            y: pointer.y,
            buttons: 0,
        });
        sim.extract_view(&log, &mut view);
        let textbox = view
            .hud
            .textbox
            .as_ref()
            .expect("real pointer picks proposal");
        let lines: Vec<_> = textbox.lines.iter().map(|&line| view.str(line)).collect();
        assert!(
            lines.contains(&"Last actor: Requester"),
            "{event:?}: {lines:?}"
        );
        assert!(
            lines.contains(&"Review state: Review approved"),
            "{event:?}: {lines:?}"
        );
        assert!(
            !lines
                .iter()
                .any(|line| line.contains("Approved by Requester"))
        );
        assert!(
            view.labels
                .iter()
                .any(|label| view.str(label.text) == "A #8 1d")
        );
        assert!(
            view.hud
                .pr_key
                .iter()
                .all(|&line| !view.str(line).contains("R ready / Q requested")),
            "key cannot advertise glyphs absent from actual immutable state"
        );
        if event == ChangeEvent::PrReviewRequested {
            assert!(lines.contains(&"Requested: Target reviewer"));
        }
    }
}
