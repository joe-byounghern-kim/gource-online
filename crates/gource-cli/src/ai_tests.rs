// SPDX-License-Identifier: GPL-3.0-or-later

//! AI decorations end to end over the custom-log fixtures (AI-62, AI-66,
//! AI-72): the `DECO` golden table, stripped versus hidden parity, and
//! determinism of fixed-dt runs.
//!
//! The golden table is `crates/gource-core/tests/golden/deco_golden.tsv`;
//! regenerate it with `GOURCE_BLESS=1 cargo test -p gource-cli deco_golden`
//! after an intended change of the decorations or the simulation.

use std::ffi::OsStr;
use std::path::PathBuf;

use gource_core::events::EventLog;
use gource_core::settings::Settings;
use gource_core::sim::Simulation;
use gource_core::view::SceneView;
use gource_render::{Frame, FrameBuilder, MonospaceGlyphs};

use crate::hash::hash_view;
use crate::load::load_custom_log;

const DT: f32 = 1.0 / 60.0;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixture(name: &str) -> EventLog {
    let path = repo_root().join("tests/fixtures/custom-log").join(name);
    load_custom_log(OsStr::new(&path)).unwrap().log
}

fn sim(options: &[(&str, &str)]) -> Simulation {
    let mut settings = Settings::default();
    for (name, value) in options {
        settings.apply_option(name, value).unwrap();
    }
    let mut sim = Simulation::new(settings);
    sim.set_viewport(1280, 720, 1.0);
    sim
}

/// `(frame, hash)` every `every` frames.
fn hashes(log: &EventLog, options: &[(&str, &str)], frames: u32, every: u32) -> Vec<(u32, u64)> {
    let mut sim = sim(options);
    let mut view = SceneView::default();
    let mut out = Vec::new();
    for frame in 1..=frames {
        sim.update(log, DT);
        sim.extract_view(log, &mut view);
        if frame % every == 0 {
            out.push((frame, hash_view(&view)));
        }
    }
    out
}

#[test]
fn deco_golden() {
    let log = fixture("ai-coauthors.log");
    let runs: [(&str, &[(&str, &str)]); 3] = [
        ("default", &[]),
        (
            "rules",
            &[
                (
                    "user-style",
                    "ai:claude => hat=agent; glow=agent\nagent => badge=FF00FF",
                ),
                ("agent-key", "true"),
            ],
        ),
        ("hat-still", &[("ai-style", "hat"), ("decor-motion", "off")]),
    ];
    let mut table = String::from("# run\tframe\thash (hash_view with the AI sections)\n");
    for (name, options) in runs {
        for (frame, hash) in hashes(&log, options, 900, 30) {
            table.push_str(&format!("{name}\t{frame}\t{hash:016x}\n"));
        }
    }
    let path = repo_root().join("crates/gource-core/tests/golden/deco_golden.tsv");
    if std::env::var_os("GOURCE_BLESS").is_some() {
        std::fs::write(&path, &table).unwrap();
    }
    let golden = std::fs::read_to_string(&path).unwrap();
    assert_eq!(table, golden, "regenerate with GOURCE_BLESS=1 if intended");
}

#[test]
fn decorated_fixture_has_decorations() {
    let log = fixture("ai-coauthors.log");
    let mut sim = sim(&[]);
    let mut view = SceneView::default();
    let mut decorated_frames = 0;
    for _ in 0..900 {
        sim.update(&log, DT);
        sim.extract_view(&log, &mut view);
        if view.users.iter().any(|u| u.decor.mask != 0) {
            decorated_frames += 1;
        }
    }
    assert!(decorated_frames > 60, "{decorated_frames}");
}

/// AI-66: the log with `ai=` fields drawn with `hide=decorations` equals
/// the same log without the fields, view for view and frame for frame.
#[test]
fn stripped_log_equals_hidden_decorations() {
    let decorated = fixture("ai-coauthors.log");
    let stripped = fixture("ai-coauthors-stripped.log");
    assert!(decorated.attr_set_count() > 1);
    assert_eq!(stripped.attr_set_count(), 1);

    let mut a = sim(&[("hide", "decorations")]);
    let mut b = sim(&[]);
    let (mut va, mut vb) = (SceneView::default(), SceneView::default());
    let (mut fa, mut fb) = (Frame::default(), Frame::default());
    let (mut ba, mut bb) = (FrameBuilder::new(), FrameBuilder::new());
    let mut glyphs = MonospaceGlyphs;
    let mut plain_differs = false;
    let mut c = sim(&[]);
    let mut vc = SceneView::default();
    for frame in 0..1200 {
        a.update(&decorated, DT);
        b.update(&stripped, DT);
        c.update(&decorated, DT);
        a.extract_view(&decorated, &mut va);
        b.extract_view(&stripped, &mut vb);
        c.extract_view(&decorated, &mut vc);
        assert_eq!(hash_view(&va), hash_view(&vb), "frame {frame}");
        assert!(va == vb, "view differs at frame {frame}");
        ba.build(&va, a.settings(), DT, &mut glyphs, &mut fa);
        bb.build(&vb, b.settings(), DT, &mut glyphs, &mut fb);
        assert!(fa == fb, "frame {frame} differs");
        plain_differs |= hash_view(&vc) != hash_view(&vb);
    }
    assert!(plain_differs, "the decorations never showed");
}

/// AI-72: two fixed-dt runs with the same options hash equal, the
/// decoration sections included.
#[test]
fn fixed_dt_runs_with_the_same_options_are_identical() {
    let log = fixture("ai-coauthors.log");
    let options: &[(&str, &str)] = &[
        (
            "user-style",
            "assisted => hat=agent\nhuman & !ai => halo=on",
        ),
        ("agent-key", "true"),
        ("ai-style", "shades"),
    ];
    let first = hashes(&log, options, 900, 1);
    assert_eq!(first, hashes(&log, options, 900, 1));
    // other options give other frames (here: the hash seed recolours;
    // the flutter phase does not follow it, see `user.rs`)
    let seeded: Vec<(&str, &str)> = options
        .iter()
        .copied()
        .chain([("hash-seed", "7")])
        .collect();
    assert_ne!(first, hashes(&log, &seeded, 900, 1));
}
