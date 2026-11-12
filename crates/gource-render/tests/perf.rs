// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Frame build timing. Run with
//! `cargo test -p gource-render --release --test perf -- --ignored --nocapture`.

mod common;

use std::time::Instant;

use gource_render::gource_core::settings::Settings;
use gource_render::{Frame, FrameBuilder, MonospaceGlyphs};

#[test]
#[ignore = "timing; run in release"]
fn build_20k_files_2k_dirs() {
    let view = common::scene(2000, 20_000, 100);
    let settings = Settings::default();
    let mut glyphs = MonospaceGlyphs;
    let mut builder = FrameBuilder::new();
    let mut frame = Frame::default();

    for _ in 0..10 {
        builder.build(&view, &settings, 1.0 / 60.0, &mut glyphs, &mut frame);
    }

    let runs = 200;
    let mut best = f64::MAX;
    let start = Instant::now();
    for _ in 0..runs {
        let t = Instant::now();
        builder.build(&view, &settings, 1.0 / 60.0, &mut glyphs, &mut frame);
        best = best.min(t.elapsed().as_secs_f64());
    }
    let mean = start.elapsed().as_secs_f64() / runs as f64;

    println!(
        "FrameBuilder::build, 20k files / 2k dirs / 100 users / {} labels: mean {:.3} ms, best {:.3} ms; \
         {} quads, {} bloom quads, {} glyph quads, {} commands",
        view.labels.len(),
        mean * 1e3,
        best * 1e3,
        frame.quads.len() / 4,
        frame.bloom.len() / 4,
        frame.text.len() / 4,
        frame.commands.len(),
    );
    assert!(frame.commands.len() <= 16);
}
