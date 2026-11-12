// SPDX-License-Identifier: GPL-3.0-or-later

//! The recorder against the fixture logs (`recording.md` section 8): stop
//! conditions (T-4), output size and camera (T-6), estimates against the
//! exact length (T-9a, REC-69), the dry run (T-9b), the HUD (T-21), the
//! glyph eviction counter (T-22), the settings snapshot (T-26), preview and
//! restart (T-27), determinism (REC-36) and the recipe round trip.

mod common;

use common::*;
use gource_core::settings::Settings;
use gource_core::sim::Simulation;
use gource_core::view::SceneView;
use gource_core::view_hash::hash_view;
use gource_web::engine::Engine;
use gource_web::record::codes;
use gource_web::record::estimate::estimate;
use gource_web::record::json::{self, Value};
use gource_web::record::planner::RecordPlanner;
use gource_web::record::recipe::{RecipeKind, recipe};
use gource_web::record::setup::RecordRequest;
use gource_web::record::stop::{IN_TAIL, KEY, LAST, LIMIT};

const CAP: usize = 200_000;

#[test]
fn rich_initial_seeds_are_context_not_recording_activity() {
    let seed = b"1704067200|Owner|A|/src/.pull-requests/#7 Context||ev=pr-opened;pr=7;pr-author=Owner;pr-created=1700000000;pr-state=open;pr-initial=1\n";
    let log = parse_log(seed);
    assert_eq!(log.commit_count(), 1);
    assert_eq!(log.pr_changes().len(), 1);
    let seed_estimate = estimate(&log, &Settings::default(), 60, 5.0, 0.0);
    assert!(
        seed_estimate.empty(),
        "initial state must not invent activity"
    );
    assert_eq!(seed_estimate.upper_s, 0.0);
    assert_eq!(seed_estimate.about_s, 0.0);
    assert_eq!(driver(&log, &[]).err().unwrap().code, codes::EMPTY_RUN);
    assert_eq!(
        RecordPlanner::create(
            &log,
            Settings::default(),
            LIVE,
            &RecordRequest::default(),
            TestClock::default(),
        )
        .err()
        .unwrap()
        .code,
        codes::EMPTY_RUN
    );

    let mut active = seed.to_vec();
    active.extend_from_slice(b"1704067210|Author|A|/src/real.rs\n");
    let active = parse_log(&active);
    assert!(!estimate(&active, &Settings::default(), 60, 5.0, 0.0).empty());
    assert!(driver(&active, &[]).is_ok());
}

#[test]
fn multiple_context_seeds_do_not_invent_span_tail_or_loop_duration() {
    let log = parse_log(b"1704067200|Owner|A|/src/.pull-requests/#7 Context||ev=pr-opened;pr=7;pr-author=Owner;pr-created=1700000000;pr-state=open;pr-initial=1\n1704153600|Owner|A|/src/.pull-requests/#8 Context||ev=pr-opened;pr=8;pr-author=Owner;pr-created=1700000000;pr-state=draft;pr-initial=1\n");
    let settings = Settings {
        loop_playback: true,
        ..Settings::default()
    };
    let estimate = estimate(&log, &settings, 60, 5.0, 30.0);
    assert!(estimate.empty());
    assert_eq!(
        estimate.span_days, 0.0,
        "seed context is not recording activity span"
    );
    assert_eq!(
        estimate.tail_s, 0.0,
        "there are no actions to animate or fade"
    );
    assert_eq!(estimate.body_s(), 0.0);
    assert!(!estimate.clipped);
}

#[test]
fn real_recording_fork_preserves_typed_pr_prefix_age_and_repository_hud() {
    let log = parse_log(b"1704067200|Owner|A|/src/.pull-requests/#7 Context||ev=pr-opened;pr=7;pr-author=Owner;pr-created=1700000000;pr-state=open;pr-initial=1;pr-log=1,1700000000,1800000000,1704067200,c,p,u\n1704153600|Author|A|/src/real.rs\n");
    assert_eq!(log.pr_changes().len(), 1);
    let mut d = driver(
        &log,
        &[
            ("start-date", "2024-01-02"),
            ("pr-flow-hud", "true"),
            ("record-max-seconds", "2"),
        ],
    )
    .unwrap();
    assert_eq!(d.engine().log().pr_changes(), log.pr_changes());
    assert_eq!(d.engine().log().pr_coverage(), log.pr_coverage());
    record(&mut d, CAP);
    let scene = d.engine().scene_view();
    assert!(
        scene
            .labels
            .iter()
            .any(|label| scene.str(label.text).starts_with("P #7 "))
    );
    let flow = scene
        .hud
        .pr_flow
        .expect("frozen complete lifecycle survives recording fork");
    assert!(
        scene
            .str(flow)
            .starts_with("PRs (repository): 1 open, oldest ")
    );
    assert_eq!(scene.hud.pr_coverage.len(), 3);
    assert_eq!(d.engine().simulation().pr_nodes().len(), 1);
}

#[test]
fn rich_boundary_recording_recipe_reproduces_every_frame_without_replaying_seeds() {
    let log = parse_log(include_bytes!(
        "../../../apps/player/tests/samples/pull-request-lifecycle.log"
    ));
    let pairs = [
        ("start-date", "2024-01-02"),
        ("seconds-per-day", "1"),
        ("pull-request-mode", "full"),
        ("pull-request-style", "dim-highlight"),
        ("pr-flow-hud", "true"),
        ("record-framerate", "30"),
        ("record-viewport", "1280x720"),
        ("record-max-seconds", "2"),
    ];
    let mut first = driver(&log, &pairs).unwrap();
    let frames = record(&mut first, CAP);
    assert!(!frames.is_empty());
    let original_flow = first
        .engine()
        .simulation()
        .pr_flow_stats(first.engine().log());
    assert!(
        original_flow.is_some(),
        "frozen lifecycle coverage remains usable"
    );
    let sidecar = recipe(
        first.setup(),
        r#"{"source":{"kind":"local","private":false}}"#,
        RecipeKind::Sidecar,
        Some(first.result_value()),
        Some(log.commit(0).timestamp),
    )
    .unwrap();
    let options = sidecar.get("options").unwrap();
    assert_eq!(
        options.get("pull-request-mode").unwrap().as_str(),
        Some("full")
    );
    assert_eq!(options.get("pr-flow-hud").unwrap().as_str(), Some("true"));
    let mut request = RecordRequest::default();
    for section in ["options", "record"] {
        for (key, value) in sidecar.get(section).unwrap().members().unwrap() {
            request.set(key, value.as_str().unwrap()).unwrap();
        }
    }
    let mut reproduced = gource_web::record::driver::RecordDriver::create(
        &log,
        Settings::default(),
        LIVE,
        &request,
        FakeBackend::default(),
        TestClock::default(),
    )
    .unwrap();
    assert_eq!(
        record(&mut reproduced, CAP),
        frames,
        "all scene hashes and frame timing reproduce"
    );
    assert_eq!(reproduced.engine().log().pr_changes(), log.pr_changes());
    assert_eq!(reproduced.engine().log().pr_coverage(), log.pr_coverage());
    assert_eq!(
        reproduced
            .engine()
            .simulation()
            .pr_flow_stats(reproduced.engine().log()),
        original_flow
    );
    assert_eq!(
        reproduced
            .engine()
            .simulation()
            .settings()
            .pull_request_style,
        first.engine().simulation().settings().pull_request_style
    );
}

fn last_frame_flags(frames: &[FrameInfo]) -> u32 {
    frames.last().unwrap().flags
}

fn tail_frames(frames: &[FrameInfo]) -> usize {
    frames.iter().filter(|f| f.flags & IN_TAIL != 0).count()
}

/// T-4: the tail after the last commit, `-1`, a zero tail, the length cap
/// and a loop with a cap.
#[test]
fn stop_conditions() {
    let log = fixture("file-removal.log");

    // default 5 s tail at 60 fps: the frame that played the last commit plus 300
    let mut d = driver(&log, &[]).unwrap();
    let frames = record(&mut d, CAP);
    assert_eq!(tail_frames(&frames), 301);
    assert_eq!(last_frame_flags(&frames) & (LAST | LIMIT), LAST);
    assert!(d.ended().unwrap().limit_name().is_none());
    // the last frame is exactly the tail after the frame of the last commit
    let first_tail = frames.iter().position(|f| f.flags & IN_TAIL != 0).unwrap();
    let span_us = frames.last().unwrap().pts_us - frames[first_tail].pts_us;
    assert_eq!(span_us, 5_000_000);
    // frame 0 is a keyframe, then every 2 s
    for (i, f) in frames.iter().enumerate() {
        assert_eq!(f.flags & KEY != 0, i % 120 == 0, "frame {i}");
    }

    // at 30 fps the same run ends after 150 tail frames
    let mut d = driver(&log, &[("record-framerate", "30")]).unwrap();
    assert_eq!(tail_frames(&record(&mut d, CAP)), 151);

    // a zero tail ends on the frame that played the last commit
    let mut d = driver(&log, &[("record-tail-seconds", "0")]).unwrap();
    let zero = record(&mut d, CAP);
    assert_eq!(tail_frames(&zero), 1);
    assert_eq!(zero.len(), first_tail + 1);

    // -1 waits until every user has left (Gource's exporter would quit)
    let mut d = driver(&log, &[("record-tail-seconds", "-1")]).unwrap();
    let until_quit = record(&mut d, CAP);
    assert!(d.engine().simulation().app_finished());
    assert!(
        until_quit.len() > zero.len() + 60 * 10,
        "{}",
        until_quit.len()
    );
    assert_eq!(until_quit[..first_tail], zero[..first_tail]);

    // the cap
    let mut d = driver(&log, &[("record-max-seconds", "2")]).unwrap();
    let capped = record(&mut d, CAP);
    assert_eq!(capped.len(), 120);
    assert_eq!(last_frame_flags(&capped) & (LAST | LIMIT), LAST | LIMIT);
    assert_eq!(d.ended().unwrap().limit_name(), Some("max_seconds"));

    // a loop needs a cap, and then runs exactly to it
    let err = driver(&log, &[("loop", "true")]).err().unwrap();
    assert_eq!(err.code, codes::UNBOUNDED);
    let mut d = driver(&log, &[("loop", "true"), ("record-max-seconds", "30")]).unwrap();
    let looped = record(&mut d, CAP);
    assert_eq!(looped.len(), 1800);
    assert_eq!(tail_frames(&looped), 0);
    assert!(!d.setup().sim_settings.stop_at_end);

    // timestamps and durations follow the plan; the last frame has the nominal period
    for (i, f) in frames.iter().enumerate() {
        assert_eq!(f.pts_us, (i as u64 * 1_000_000 + 30) / 60);
    }
    assert_eq!(frames.last().unwrap().duration_us, 16_667);
    assert_eq!(frames[0].duration_us, 16_667);
    assert_eq!(frames[1].duration_us, 16_666);
}

/// REC-122 and REC-41: empty ranges, a single commit, incomplete logs.
#[test]
fn empty_single_and_incomplete_runs() {
    for name in ["empty.log", "svn-dir-delete.log"] {
        let err = driver(&fixture(name), &[]).err().unwrap();
        assert_eq!(err.code, codes::EMPTY_RUN, "{name}");
        let err = RecordPlanner::create(
            &fixture(name),
            Settings::default(),
            LIVE,
            &RecordRequest::default(),
            TestClock::default(),
        )
        .err()
        .unwrap();
        assert_eq!(err.code, codes::EMPTY_RUN);
    }
    // a start date after the last commit
    let log = fixture("file-removal.log");
    let err = driver(&log, &[("start-date", "2030-01-01")]).err().unwrap();
    assert_eq!(err.code, codes::EMPTY_RUN);
    assert_eq!(
        err.to_string(),
        "record_empty_run: The selected range has no commits."
    );

    let single = fixture("single-event.log");
    let mut d = driver(&single, &[]).unwrap();
    let frames = record(&mut d, CAP);
    assert_eq!(d.setup().estimate.range.len(), 1);
    assert_eq!(tail_frames(&frames), 301);
    assert!(frames.len() <= 302, "{}", frames.len());

    let mut incomplete = log.clone();
    incomplete.set_complete(false);
    assert_eq!(
        driver(&incomplete, &[]).err().unwrap().code,
        codes::LOG_INCOMPLETE
    );
}

/// REC-44: invalid sizes and a drawable the platform shrinks.
#[test]
fn size_checks() {
    let log = fixture("file-removal.log");
    let err = driver(&log, &[("record-viewport", "4096x2304")])
        .err()
        .unwrap();
    assert_eq!(err.code, codes::SIZE_UNSUPPORTED);
    assert!(err.message.contains("30 fps"));
    assert!(
        driver(
            &log,
            &[("record-viewport", "4096x2304"), ("record-framerate", "30")]
        )
        .is_ok()
    );
    let err = RecordRequest::default()
        .set("record-viewport", "1921x1080")
        .unwrap_err();
    assert_eq!(err.code, codes::INVALID_OPTION);
    assert!(err.message.contains("even"), "{}", err.message);
    let small = FakeBackend {
        max_size: Some([2048, 2048]),
        ..FakeBackend::default()
    };
    let err = driver_with(
        &log,
        Settings::default(),
        &[("record-viewport", "3840x2160")],
        small,
    )
    .err()
    .unwrap();
    assert_eq!(err.code, codes::SIZE_UNSUPPORTED);
    assert!(err.message.contains("2048x2048"), "{}", err.message);
    assert_eq!(
        driver(&log, &[("seconds-per-day", "fast")])
            .err()
            .unwrap()
            .code,
        codes::INVALID_OPTION
    );
}

/// Run `ticks` output frames at 60 fps (one tick each) and return the view.
fn view_after(
    log: &gource_core::events::EventLog,
    viewport: &str,
    frames: usize,
) -> (Driver, SceneView) {
    let mut d = driver(log, &[("record-viewport", viewport)]).unwrap();
    d.begin_recording();
    for _ in 0..frames {
        d.step_frame().unwrap();
    }
    let v = d.engine().scene_view().clone();
    (d, v)
}

/// T-6 (REC-37 to REC-39): the camera depends on the aspect only; labels
/// keep their size relative to the short side; `font_scale` is pinned.
#[test]
fn output_size_camera_and_labels() {
    let log = generated_log(60);
    let frames = 900;
    let (d720, v720) = view_after(&log, "1280x720", frames);
    let (_, v1440) = view_after(&log, "2560x1440", frames);
    let (d1080, v1080) = view_after(&log, "1920x1080", frames);
    let (dv, vv) = view_after(&log, "1080x1920", frames);
    let (d4k, _) = view_after(&log, "3840x2160", frames);
    assert_eq!(v720.camera, v1440.camera);
    assert_eq!(v720.camera, v1080.camera);
    assert_ne!(v1080.camera, vv.camera);
    assert_eq!(v1080.viewport.to_array(), [1920.0, 1080.0]);
    assert_eq!(d720.engine().backend().size, [1280, 720]);

    let ui = |d: &Driver| d.setup().ui_scale;
    assert_eq!((ui(&d1080), ui(&dv), ui(&d4k)), (1.0, 1.0, 2.0));
    assert!((ui(&d720) - 2.0 / 3.0).abs() < 1e-6);
    for d in [&d720, &d1080, &dv, &d4k] {
        let s = d.engine().simulation().settings();
        assert_eq!(s.font_scale, 1.0);
        assert!(!s.default_font_scale);
    }
    // the same font sizes at 1920x1080 and 1080x1920, doubled at 4K
    let sizes = |d: &Driver| d.engine().backend().glyphs.sizes.clone();
    assert!(!sizes(&d1080).is_empty());
    assert_eq!(sizes(&d1080), sizes(&dv));
    let doubled: Vec<u16> = sizes(&d1080).iter().map(|s| s * 2).collect();
    assert_eq!(sizes(&d4k).into_iter().collect::<Vec<_>>(), doubled);

    // a font scale the viewer set is kept
    let mut base = Settings::default();
    base.apply_option("font-scale", "1.5").unwrap();
    let d = driver_with(
        &log,
        base,
        &[("record-viewport", "1080x1920")],
        FakeBackend::default(),
    )
    .unwrap();
    assert_eq!(d.engine().simulation().settings().font_scale, 1.5);

    // match window: the live size and device pixel ratio, scaled to fit
    let live = gource_web::record::setup::LiveView {
        device: [5120, 2880],
        dpr: 2.0,
    };
    let d = Driver::create(
        &log,
        Settings::default(),
        live,
        &request(&[("record-viewport", ""), ("record-framerate", "30")]),
        FakeBackend::default(),
        TestClock::default(),
    )
    .unwrap();
    assert_eq!((d.setup().width, d.setup().height), (4096, 2304));
    assert!((d.setup().ui_scale - 1.6).abs() < 1e-6);
    assert!(d.setup().match_window);
    assert!(d.engine().simulation().settings().default_font_scale);
}

/// T-21 (REC-40): no input reaches the recorder, so no text box, splash or
/// message is ever drawn.
#[test]
fn hud_stays_clean() {
    let log = generated_log(200);
    let mut d = driver(&log, &[("record-max-seconds", "17")]).unwrap();
    d.begin_recording();
    for _ in 0..1000 {
        d.step_frame().unwrap();
        let hud = &d.engine().scene_view().hud;
        assert!(hud.textbox.is_none());
        assert!(hud.splash <= 0.0);
        assert!(hud.message.is_none());
    }
}

/// T-22 (REC-42, REC-81): the eviction count reaches the stats and the
/// sidecar result.
#[test]
fn atlas_evictions_are_reported() {
    let log = fixture("file-removal.log");
    let backend = FakeBackend {
        evictions: 7,
        ..FakeBackend::default()
    };
    let mut d = driver_with(&log, Settings::default(), &[], backend).unwrap();
    record(&mut d, CAP);
    assert_eq!(
        d.stats_value().get("atlas_evictions").unwrap().as_f64(),
        Some(7.0)
    );
    assert_eq!(
        d.result_value().get("atlas_evictions").unwrap().as_f64(),
        Some(7.0)
    );
    // REC-116: the recorder adds no draw call
    assert_eq!(
        d.engine().backend().draw_calls,
        d.engine().status().draw_calls
    );
    assert!(d.engine().status().draw_calls <= 12);
}

/// REC-36: no wall time reaches the run; two recorders give equal frames.
#[test]
fn recordings_are_deterministic() {
    let log = generated_log(120);
    let pairs = [
        ("record-framerate", "25"),
        ("record-max-seconds", "8"),
        ("seconds-per-day", "1"),
    ];
    let a = record(&mut driver(&log, &pairs).unwrap(), CAP);
    let b = record(&mut driver(&log, &pairs).unwrap(), CAP);
    assert_eq!(a, b);
    assert_eq!(a.len(), 200);
}

/// T-27 (REC-89, REC-114, REC-120): `step_frame` needs `begin_recording`;
/// a preview, sliced within its budget, then a recording equals a fresh
/// recording.
#[test]
fn preview_then_record_equals_fresh() {
    let log = generated_log(2000);
    let pairs = [("record-max-seconds", "1")];
    let fresh = record(&mut driver(&log, &pairs).unwrap(), CAP);

    let mut d = driver(&log, &pairs).unwrap();
    let err = d.step_frame().err().unwrap();
    assert_eq!(err.code, codes::NOT_STARTED);
    // the test clock advances 1 ms per reading, one reading per tick: an
    // 8 ms slice runs 8 ticks
    let mut slices = 0;
    let before = d.engine().status().ticks;
    while !d.preview_for(5.0, 8.0).unwrap() {
        slices += 1;
        assert_eq!(d.engine().status().ticks - before, slices * 8);
    }
    assert_eq!(slices, 300 / 8);
    assert_eq!(d.engine().status().ticks, 300);
    assert!(
        d.preview_for(5.0, 8.0).unwrap(),
        "a finished preview is kept"
    );
    assert_eq!(
        d.stats_value().get("preview_empty").unwrap(),
        &Value::Bool(false)
    );
    let after_preview = record(&mut d, CAP);
    assert_eq!(after_preview, fresh);
    // no preview while recording
    d.begin_recording();
    assert_eq!(d.preview_for(1.0, 8.0).unwrap_err().code, codes::BUSY);
}

/// T-9a (REC-17, REC-122) and REC-69: on every fixture the upper bound is
/// at least the exact length (also at time-scale 0.5 and with tail -1), the
/// "about" estimate is within 0.5x to 2x of it, and progress is monotone
/// and ends at 1. T-9b (REC-68): the dry run counts the frames the
/// recorder renders.
#[test]
fn estimates_and_dry_runs_match_the_recorder() {
    let cases: &[&[(&str, &str)]] = &[
        &[],
        &[("record-framerate", "30")],
        &[("record-framerate", "24"), ("time-scale", "0.5")],
        &[("record-tail-seconds", "-1")],
        &[("record-tail-seconds", "-1"), ("time-scale", "0.5")],
        &[("seconds-per-day", "0.1"), ("record-framerate", "30")],
        &[("seconds-per-day", "1"), ("record-framerate", "30")],
        &[("seconds-per-day", "10"), ("record-framerate", "30")],
    ];
    let mut checked = 0;
    let dense: &[&[(&str, &str)]] = &[
        &[("seconds-per-day", "1"), ("record-framerate", "30")],
        &[
            ("seconds-per-day", "0.1"),
            ("record-framerate", "30"),
            ("record-tail-seconds", "-1"),
        ],
    ];
    let mut logs: Vec<_> = fixtures().into_iter().map(|(n, l)| (n, l, cases)).collect();
    logs.push(("generated-60".into(), generated_log(60), dense));
    for (name, log, cases) in logs {
        for pairs in cases.iter() {
            let mut d = match driver(&log, pairs) {
                Ok(d) => d,
                Err(e) => {
                    assert_eq!(e.code, codes::EMPTY_RUN, "{name} {pairs:?}");
                    continue;
                }
            };
            let frames = record(&mut d, CAP);
            let exact_s = frames.len() as f64 / f64::from(d.setup().plan.fps);
            let e = &d.setup().estimate;
            assert!(
                e.upper_s >= exact_s,
                "{name} {pairs:?}: upper {} < exact {exact_s}",
                e.upper_s
            );
            assert!(
                e.about_s >= 0.5 * exact_s && e.about_s <= 2.0 * exact_s,
                "{name} {pairs:?}: about {} vs exact {exact_s}",
                e.about_s
            );
            assert!(
                frames.windows(2).all(|w| w[0].progress <= w[1].progress),
                "{name}"
            );
            assert_eq!(frames.last().unwrap().progress, 1.0);
            assert!(frames[..frames.len() - 1].iter().all(|f| f.progress < 1.0));

            let mut planner = RecordPlanner::create(
                &log,
                Settings::default(),
                LIVE,
                &request(pairs),
                TestClock::default(),
            )
            .unwrap();
            while planner.run_for(8.0) > 0 {}
            assert!(planner.done());
            assert_eq!(planner.frames(), frames.len() as u64, "{name} {pairs:?}");
            let result = planner.result_value();
            assert_eq!(result.get("exact").unwrap(), &Value::Bool(true));
            assert_eq!(result.get("empty").unwrap(), &Value::Bool(false));

            // with the exact count, progress is frames over frames
            let mut exact = driver(&log, pairs).unwrap();
            exact.set_exact_frames(planner.frames());
            let p = record(&mut exact, CAP);
            let mid = p.len() / 2;
            assert!((p[mid].progress - (mid + 1) as f64 / p.len() as f64).abs() < 1e-9);
            checked += 1;
        }
    }
    assert!(checked >= 18 * cases.len() + dense.len(), "{checked}");
}

/// A filter that removes every commit empties the run without the cheap
/// estimate seeing it; the dry run and the preview report it (REC-122).
#[test]
fn filters_that_remove_everything_are_detected() {
    let log = fixture("file-removal.log");
    let pairs = [("user-filter", ".*")];
    let mut planner = RecordPlanner::create(
        &log,
        Settings::default(),
        LIVE,
        &request(&pairs),
        TestClock::default(),
    )
    .unwrap();
    while planner.run_for(50.0) > 0 {}
    assert_eq!(
        planner.result_value().get("empty").unwrap(),
        &Value::Bool(true)
    );
    let mut d = driver(&log, &pairs).unwrap();
    while !d.preview_for(5.0, 50.0).unwrap() {}
    assert_eq!(
        d.stats_value().get("preview_empty").unwrap(),
        &Value::Bool(true)
    );
}

/// T-26 (REC-31): a simulation built from the snapshot of a live engine
/// equals one built from the same options applied by name, including state
/// changed with the keyboard; the recipe carries exactly that state.
#[test]
fn settings_snapshot_is_exact() {
    for name in [
        "file-removal.log",
        "dense-contributors.log",
        "unordered-timestamps.log",
    ] {
        let log = fixture(name);
        let options = [
            ("seconds-per-day", "0.5"),
            ("hide", "filenames"),
            ("title", "My run"),
            ("file-idle-time-at-end", "2"),
            ("user-filter", "^nobody$"),
        ];
        let mut live = Engine::new(
            FakeBackend::default(),
            TestClock::default(),
            Settings::default(),
        );
        live.resize(1280.0, 720.0, 1.0).unwrap();
        // the log arrives through batches as on the page
        let mut parser = gource_sources::CustomLogParser::new();
        let mut enc = gource_sources::BatchEncoder::new();
        let text = log_text(&log);
        parser.push(text.as_bytes(), &mut enc);
        parser.finish(&mut enc);
        live.begin_log();
        live.append_batch(&enc.take_batch()).unwrap();
        live.finish_log(text.len() as f64);
        for (k, v) in options {
            live.set_option(k, v).unwrap();
        }
        live.restart();
        live.set_fixed_dt(1.0 / 60.0);
        for _ in 0..10 {
            live.step(1).unwrap();
        }
        // keyboard: faster (`+`), colour reseed (`s`), tree hidden (`t`)
        for key in ["+", "s", "t"] {
            assert!(live.key(key, true, false, false));
        }
        // play to the end so `file-idle-time-at-end` replaces the idle time
        for _ in 0..3000 {
            live.step(1).unwrap();
        }
        assert_eq!(live.simulation().settings().file_idle_time, 2.0);
        let snapshot = live.simulation().settings_snapshot();
        assert_eq!(snapshot.file_idle_time, 0.0);

        let by_name = {
            let mut s = Settings::default();
            for (k, v) in gource_web::record::recipe::non_default_options(&snapshot) {
                s.apply_option(k, &v).unwrap();
            }
            s
        };
        let hashes = |settings: Settings| {
            let mut sim = Simulation::new(settings);
            sim.set_viewport(1280, 720, 1.0);
            let mut view = SceneView::default();
            (0..600)
                .map(|_| {
                    sim.update(&log, 1.0 / 60.0);
                    sim.extract_view(&log, &mut view);
                    hash_view(&view)
                })
                .collect::<Vec<u64>>()
        };
        assert_eq!(hashes(snapshot.clone()), hashes(by_name), "{name}");
        assert_ne!(
            snapshot.hash_seed, 31,
            "the reseed key is part of the snapshot"
        );
    }
}

/// The custom-log text of a parsed log (the fixtures have no colours or
/// meta fields).
fn log_text(log: &gource_core::events::EventLog) -> String {
    let mut out = String::new();
    for i in 0..log.commit_count() {
        let c = log.commit(i);
        for f in c.changes {
            out.push_str(&format!(
                "{}|{}|{}|{}\n",
                c.timestamp,
                log.str(c.user),
                f.kind.as_action(),
                log.str(f.path)
            ));
        }
    }
    out
}

/// REC-70 to REC-73: the recipe reproduces the frames; the embedded copy
/// redacts name-bearing values and, for private sources, the location.
#[test]
fn recipe_round_trip_and_redaction() {
    let log = generated_log(100);
    let mut base = Settings::default();
    base.apply_option("hash-seed", "7").unwrap();
    base.apply_option("bloom-intensity", "0.5").unwrap();
    let pairs = [
        ("seconds-per-day", "0.3"),
        ("title", "Secret project"),
        ("user-filter", "^user3$"),
        ("record-framerate", "30"),
        ("record-viewport", "1280x720"),
        ("record-quality", "max"),
        ("record-max-seconds", "3"),
        ("record-name", "acme-secret"),
    ];
    let mut d = driver_with(&log, base, &pairs, FakeBackend::default()).unwrap();
    let frames = record(&mut d, CAP);
    let first_ts = Some(log.commit(0).timestamp);

    let ctx = r#"{"build":"abc123","source":{"kind":"repo","host":"github.com","path":"acme/secret","branch":"main","head":"f00","private":true},"time_zone":"Europe/Berlin","created":"2026-10-02T12:00:00Z","result":{"codec":"avc1.64001f","frames":1}}"#;
    let sidecar = recipe(
        d.setup(),
        ctx,
        RecipeKind::Sidecar,
        Some(d.result_value()),
        first_ts,
    )
    .unwrap();
    let embedded = recipe(d.setup(), ctx, RecipeKind::Embedded, None, first_ts).unwrap();
    // the JSON text parses back to the same value
    assert_eq!(json::parse(&sidecar.to_json()).unwrap(), sidecar);

    let opts = sidecar.get("options").unwrap();
    assert_eq!(opts.get("hash-seed").unwrap().as_str(), Some("7"));
    assert_eq!(opts.get("title").unwrap().as_str(), Some("Secret project"));
    assert_eq!(opts.get("seconds-per-day").unwrap().as_str(), Some("0.3"));
    assert!(
        opts.get("stop-at-end").is_none(),
        "forced overrides are not recipe options"
    );
    let rec = sidecar.get("record").unwrap();
    assert_eq!(
        rec.get("record-viewport").unwrap().as_str(),
        Some("1280x720")
    );
    assert_eq!(
        rec.get("record-ui-scale").unwrap().as_str(),
        Some("0.6666667")
    );
    assert_eq!(rec.get("record-framerate").unwrap().as_str(), Some("30"));
    assert!(
        rec.get("record-hardware").is_none(),
        "defaults are left out"
    );
    let result = sidecar.get("result").unwrap();
    assert_eq!(
        result.get("frames").unwrap().as_f64(),
        Some(frames.len() as f64)
    );
    assert_eq!(result.get("codec").unwrap().as_str(), Some("avc1.64001f"));
    assert_eq!(result.get("limit").unwrap().as_str(), Some("max_seconds"));
    assert_eq!(
        sidecar.get("time_zone").unwrap().as_str(),
        Some("Europe/Berlin")
    );
    let src = sidecar.get("source").unwrap();
    assert!(src.get("path").is_none() && src.get("head").is_none());
    assert_eq!(src.get("branch").unwrap().as_str(), Some("main"));
    assert_eq!(
        sidecar.get("plan").unwrap().get("fixed_dt").unwrap(),
        &Value::Number("0.016666668".into())
    );

    // the embedded copy: no result, time zone, creation time, names or location
    for key in ["result", "time_zone", "created"] {
        assert!(embedded.get(key).is_none(), "{key}");
    }
    let eopts = embedded.get("options").unwrap();
    assert!(eopts.get("title").is_none() && eopts.get("user-filter").is_none());
    assert!(embedded.get("record").unwrap().get("record-name").is_none());
    let redacted: Vec<&str> = match embedded.get("redacted").unwrap() {
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        redacted,
        [
            "source.path",
            "source.branch",
            "source.head",
            "title",
            "user-filter",
            "record-name"
        ]
    );
    assert!(!embedded.to_json().contains("acme"));
    // confirmed by the viewer: the location stays
    let confirmed = ctx.replacen('{', r#"{"confirmed":true,"#, 1);
    let e2 = recipe(d.setup(), &confirmed, RecipeKind::Embedded, None, first_ts).unwrap();
    assert_eq!(
        e2.get("source").unwrap().get("path").unwrap().as_str(),
        Some("acme/secret")
    );

    // reproduce: the sidecar's options and record options on default settings
    let mut again = RecordRequest::default();
    for section in ["options", "record"] {
        for (k, v) in sidecar.get(section).unwrap().members().unwrap() {
            again.set(k, v.as_str().unwrap()).unwrap();
        }
    }
    let mut d2 = gource_web::record::driver::RecordDriver::create(
        &log,
        Settings::default(),
        LIVE,
        &again,
        FakeBackend::default(),
        TestClock::default(),
    )
    .unwrap();
    assert_eq!(record(&mut d2, CAP), frames);
}

/// The plan JSON the dialog reads.
#[test]
fn plan_json_shape() {
    let log = fixture("file-removal.log");
    let d = driver(&log, &[]).unwrap();
    let plan = json::parse(&d.plan_value().to_json()).unwrap();
    let num = |k: &str| plan.get(k).unwrap().as_f64().unwrap();
    assert_eq!(
        (num("fps"), num("ticks_per_frame"), num("tick_hz")),
        (60.0, 1.0, 60.0)
    );
    assert_eq!(
        (num("width"), num("height"), num("ui_scale")),
        (1920.0, 1080.0, 1.0)
    );
    assert_eq!(plan.get("codec").unwrap().as_str(), Some("avc1.64002a"));
    assert_eq!(num("bitrate_bps"), 18e6);
    assert_eq!(num("keyframe_every"), 120.0);
    let est = plan.get("estimate").unwrap();
    assert_eq!(est.get("exact").unwrap(), &Value::Bool(false));
    assert_eq!(est.get("frames").unwrap(), &Value::Null);
    assert!(est.get("upper_s").unwrap().as_f64().unwrap() >= 5.0);
    let size = est.get("size_upper_bytes").unwrap().as_f64().unwrap();
    let upper = est.get("upper_s").unwrap().as_f64().unwrap();
    assert!((size - 2.5 * 18e6 * upper / 8.0).abs() < 1e4);
    let layout = plan.get("layout").unwrap();
    assert_eq!(layout.get("seekable").unwrap().as_str(), Some("fragmented"));
    assert_eq!(
        layout.get("memory").unwrap().as_str(),
        Some("faststart-deferred")
    );
    let ladder = match plan.get("ladder").unwrap() {
        Value::Array(items) => items.clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        ladder[0].get("codec").unwrap().as_str(),
        Some("avc1.64002a")
    );
    assert_eq!(ladder[0].get("note").unwrap(), &Value::Null);

    // with the exact count (or a cap) the seekable layout is faststart
    let mut d = driver(&log, &[]).unwrap();
    d.set_exact_frames(400);
    let plan = d.plan_value();
    let layout = plan.get("layout").unwrap();
    assert_eq!(layout.get("seekable").unwrap().as_str(), Some("faststart"));
    assert_eq!(layout.get("max_samples").unwrap().as_f64(), Some(400.0));
    assert_eq!(layout.get("max_sync_samples").unwrap().as_f64(), Some(5.0));
    assert_eq!(
        plan.get("estimate")
            .unwrap()
            .get("duration_s")
            .unwrap()
            .as_f64(),
        Some(6.667)
    );

    // the cheap estimate on its own
    let e = estimate(&log, &d.setup().sim_settings, 60, 5.0, 0.0);
    assert_eq!(e.range.len(), log.commit_count());
}
