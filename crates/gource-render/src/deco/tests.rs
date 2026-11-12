// SPDX-License-Identifier: GPL-3.0-or-later

//! Decoration blocks, draw commands, geometry, glow and the AI HUD text.

use glam::{Vec2, Vec3, Vec4};

use gource_core::settings::Settings;
use gource_core::view::{
    ActionView, AgentKeyRow, CameraView, DecoKind, DirView, FileView, FontRole, LabelAnchor,
    LabelView, SceneView, UserDecor, UserView,
};

use super::*;
use crate::deco_atlas::{CELL_SIZE, GUTTER};
use crate::{DrawCommand, Frame, FrameBuilder, MonospaceGlyphs, Space, Stream, TextureId};

fn dir(parent: Option<u32>, pos: Vec2) -> DirView {
    DirView {
        parent,
        depth: 1,
        pos,
        spos: pos * 0.5,
        radius: 12.0,
        colour: Vec4::ONE,
        in_frustum: true,
        visible: true,
        draw_edge: parent.is_some(),
        draw_bloom: true,
    }
}

/// A typical frame: two dirs, files, a beam, three users with names and the
/// date; `decor` per user.
fn scene(viewport: Vec2, ui_scale: f32, decor: [UserDecor; 3]) -> SceneView {
    let mut v = SceneView {
        viewport,
        ui_scale,
        background: Vec3::splat(0.1),
        camera: CameraView::default(),
        ..SceneView::default()
    };
    v.dirs.push(dir(None, Vec2::ZERO));
    v.dirs.push(dir(Some(0), Vec2::new(60.0, 0.0)));
    for i in 0..4 {
        v.files.push(FileView {
            pos: Vec2::new(60.0 + i as f32 * 8.0, 8.0),
            size: Vec2::splat(8.0),
            colour: Vec3::ONE,
            alpha: 1.0,
        });
    }
    for (i, d) in decor.into_iter().enumerate() {
        let pos = Vec2::new(-100.0 + i as f32 * 100.0, 80.0);
        v.users.push(UserView {
            pos,
            size: Vec2::new(20.0, 20.0 * 512.0 / 384.0),
            colour: Vec3::new(0.9, 0.6, 0.6),
            alpha: 0.8,
            decor: d,
            ..UserView::default()
        });
        let text = v.push_text("user");
        v.labels.push(LabelView {
            anchor: LabelAnchor::WorldCentredAbove { pos },
            text,
            font: FontRole::UserName,
            colour: Vec4::ONE,
        });
    }
    v.actions.push(ActionView {
        src: v.users[0].pos,
        dest: v.files[0].pos,
        target_size: 8.0,
        colour: Vec3::ONE,
        progress: 0.5,
    });
    v.hud.date = Some(v.push_text("Monday, 01 January, 2024"));
    v
}

fn decor(mask: u8) -> UserDecor {
    UserDecor {
        mask,
        dynamic: mask,
        badge: 3,
        tint: [[255, 128, 0]; DecoKind::COUNT],
        envelope: 0.5,
        scale: 1.0,
        motion: 1.0,
        ..UserDecor::default()
    }
}

const ALL: u8 = 0b11_1111;
const NONE: UserDecor = UserDecor {
    mask: 0,
    dynamic: 0,
    inverse: 0,
    badge: 0,
    tint: [[0; 3]; DecoKind::COUNT],
    envelope: 0.0,
    scale: 0.0,
    vel: Vec2::ZERO,
    phase: 0.0,
    motion: 0.0,
};

fn build(view: &SceneView, settings: &Settings) -> Frame {
    let mut b = FrameBuilder::new();
    let mut f = Frame::default();
    b.build(view, settings, 1.0 / 60.0, &mut MonospaceGlyphs, &mut f);
    f
}

fn deco_commands(f: &Frame) -> Vec<&DrawCommand> {
    f.commands
        .iter()
        .filter(|c| c.texture == TextureId::Deco)
        .collect()
}

fn users_command(f: &Frame) -> &DrawCommand {
    f.commands
        .iter()
        .find(|c| c.texture == TextureId::User && c.program == crate::Program::Textured)
        .unwrap()
}

const VP: Vec2 = Vec2::new(1280.0, 720.0);

#[test]
fn ten_commands_without_decorations_twelve_with_both_blocks() {
    let s = Settings::default();
    let plain = build(&scene(VP, 1.0, [NONE; 3]), &s);
    assert_eq!(plain.commands.len(), 10);

    let full = build(&scene(VP, 1.0, [decor(ALL), NONE, decor(ALL)]), &s);
    assert_eq!(full.commands.len(), 12);
    // the two new commands sit around the avatars; the rest is unchanged
    let mut without: Vec<DrawCommand> = full
        .commands
        .iter()
        .filter(|c| c.texture != TextureId::Deco)
        .copied()
        .collect();
    assert_eq!(without.len(), 10);
    for (a, b) in without.iter_mut().zip(&plain.commands) {
        if a.stream == Stream::Quads && a.space == Space::World || a.stream == Stream::Bloom {
            // ranges move or grow with the decoration and glow quads
            a.first_quad = b.first_quad;
            a.quad_count = b.quad_count;
        }
        assert_eq!(a.texture, b.texture);
        assert_eq!(a.program, b.program);
    }
    let i = full
        .commands
        .iter()
        .position(|c| c.texture == TextureId::Deco)
        .unwrap();
    assert_eq!(full.commands[i - 1].texture, TextureId::User);
    assert_eq!(full.commands[i + 1].texture, TextureId::User);
    assert_eq!(full.commands[i + 2].texture, TextureId::Deco);
    assert_eq!(full.commands[i + 3].stream, Stream::Bloom);

    // one block only: 11
    let back_only = build(
        &scene(VP, 1.0, [decor(DecoKind::Cape.bit()), NONE, NONE]),
        &s,
    );
    assert_eq!(back_only.commands.len(), 11);
    let front_only = build(
        &scene(VP, 1.0, [decor(DecoKind::Hat.bit()), NONE, NONE]),
        &s,
    );
    assert_eq!(front_only.commands.len(), 11);
    // glow alone adds no command
    let glow_only = build(
        &scene(VP, 1.0, [decor(DecoKind::Glow.bit()), NONE, NONE]),
        &s,
    );
    assert_eq!(glow_only.commands.len(), 10);
}

#[test]
fn blocks_are_contiguous_around_the_avatars_in_user_order() {
    let s = Settings::default();
    let f = build(&scene(VP, 1.0, [decor(ALL), NONE, decor(ALL)]), &s);
    let deco = deco_commands(&f);
    let (back, front) = (deco[0], deco[1]);
    let users = users_command(&f);
    assert_eq!(back.first_quad + back.quad_count, users.first_quad);
    assert_eq!(users.first_quad + users.quad_count, front.first_quad);
    // cape + halo per decorated user; shades + hat + badge per user
    assert_eq!(back.quad_count, 4);
    assert_eq!(front.quad_count, 6);
    assert_eq!(users.quad_count, 3);
    // user order: the first decorated user's cape comes first
    let cape = &f.quads[back.first_quad as usize * 4];
    assert!(cape.pos[0] < -100.0 + 10.0);
    let halo = &f.quads[(back.first_quad as usize + 1) * 4];
    assert_eq!(
        [halo.uv[0], halo.uv[1]],
        [DecoCell::Halo.uv()[0], DecoCell::Halo.uv()[1]]
    );
    // the badge uses its shape's cell
    let badge = &f.quads[(front.first_quad as usize + 2) * 4];
    assert_eq!(badge.uv[0], DecoCell::BADGES[3].uv()[0]);
    assert_eq!(badge.uv[1], DecoCell::BADGES[3].uv()[1]);
    // the user shadow still covers exactly the avatars
    let shadow = f
        .commands
        .iter()
        .find(|c| c.texture == TextureId::User && c.program != crate::Program::Textured)
        .unwrap();
    assert_eq!(
        (shadow.first_quad, shadow.quad_count),
        (users.first_quad, users.quad_count)
    );
}

#[test]
fn alpha_follows_the_user_and_the_polarity() {
    let s = Settings::default();
    let mut d = decor(DecoKind::Cape.bit() | DecoKind::Hat.bit());
    d.dynamic = DecoKind::Cape.bit();
    let f = build(&scene(VP, 1.0, [d, NONE, NONE]), &s);
    let deco = deco_commands(&f);
    let cape = &f.quads[deco[0].first_quad as usize * 4];
    let hat = &f.quads[deco[1].first_quad as usize * 4];
    assert_eq!(cape.colour, [1.0, 128.0 / 255.0, 0.0, 0.8 * 0.5]);
    assert_eq!(hat.colour[3], 0.8);
    // inverse polarity fades the other way
    d.dynamic = 0;
    d.inverse = DecoKind::Hat.bit();
    d.envelope = 0.25;
    let f = build(&scene(VP, 1.0, [d, NONE, NONE]), &s);
    let hat = &f.quads[deco_commands(&f)[1].first_quad as usize * 4];
    assert_eq!(hat.colour[3], 0.8 * 0.75);
    // a decoration at zero alpha is not emitted
    let mut d = decor(DecoKind::Cape.bit());
    d.envelope = 0.0;
    let f = build(&scene(VP, 1.0, [d, NONE, NONE]), &s);
    assert!(deco_commands(&f).is_empty());
}

#[test]
fn cape_leans_against_motion_lifts_and_flutters() {
    let mut u = UserView {
        pos: Vec2::ZERO,
        size: Vec2::new(20.0, 30.0),
        alpha: 1.0,
        // fully faded in, so the fold below does not apply yet
        decor: UserDecor {
            dynamic: 0,
            ..decor(DecoKind::Cape.bit())
        },
        ..UserView::default()
    };
    let rest = corners(&u, CAPE_RECT);
    // at rest (phase 0: no flutter) the cape is its anchor rectangle
    assert_eq!(cape_corners(&u), rest);

    u.decor.vel = Vec2::new(500.0, 0.0);
    let c = cape_corners(&u);
    assert_eq!(&c[..2], &rest[..2], "the top edge stays at the shoulders");
    let shift = c[2] - rest[2];
    assert!((shift.x - -0.6 * 20.0).abs() < 1e-4, "{shift:?}");
    assert!((shift.y - -0.2 * 30.0).abs() < 1e-4, "{shift:?}");
    assert!(((c[3] - rest[3]) - shift).length() < 1e-4);

    // at rest the hem sways (flutter) and billows: the corners spread apart
    u.decor.vel = Vec2::ZERO;
    u.decor.phase = 0.25;
    let c = cape_corners(&u);
    assert!(((c[2] - rest[2]).x - (0.04 + 0.015) * 20.0).abs() < 1e-5);
    assert!(((c[3] - rest[3]).x - (0.04 - 0.015) * 20.0).abs() < 1e-5);
    u.decor.phase = 0.5;
    let c = cape_corners(&u);
    assert!(
        ((c[2] - rest[2]).x - 0.03 * 20.0).abs() < 1e-4,
        "full billow"
    );
    assert!(
        ((c[3] - rest[3]).x + 0.03 * 20.0).abs() < 1e-4,
        "full billow"
    );
    // a fast cape does not billow
    u.decor.vel = Vec2::new(0.0, 400.0);
    let c = cape_corners(&u);
    assert!(((c[2] - c[3]).x - (rest[2] - rest[3]).x).abs() < 1e-4);
    u.decor.phase = 0.0;

    // a fading cape folds up toward the shoulders
    u.decor.vel = Vec2::ZERO;
    u.decor.dynamic = DecoKind::Cape.bit();
    u.decor.envelope = 0.0;
    let c = cape_corners(&u);
    assert_eq!(&c[..2], &rest[..2]);
    assert!(((c[2] - c[1]) - (rest[2] - rest[1]) * 0.35).length() < 1e-4);
    u.decor.envelope = 1.0;
    assert_eq!(cape_corners(&u), rest);

    // decor-motion=off freezes lean, lift and flutter
    u.decor.vel = Vec2::new(-300.0, 200.0);
    u.decor.motion = 0.0;
    assert_eq!(cape_corners(&u), rest);

    // scale works about the avatar centre
    u.decor.scale = 2.0;
    let c = cape_corners(&u);
    assert_eq!(c[0], rest[0] * 2.0);
}

#[test]
fn glow_goes_to_the_bloom_stream_and_ignores_hide_bloom() {
    let mut d = decor(DecoKind::Glow.bit());
    d.dynamic = 0;
    let v = scene(VP, 1.0, [NONE, d, NONE]);
    let s = Settings::default();
    let f = build(&v, &s);
    let bloom = f
        .commands
        .iter()
        .find(|c| c.stream == Stream::Bloom)
        .unwrap();
    // two dir blooms, then the glow, in one command
    assert_eq!((bloom.first_quad, bloom.quad_count), (0, 3));
    let g = &f.bloom[8];
    let u = &v.users[1];
    let r = GLOW_RADIUS * u.size.x;
    assert_eq!(g.texcoord, [r, u.pos.x, u.pos.y, 0.0]);
    let strength = GLOW_STRENGTH * u.alpha;
    assert_eq!(g.colour, [strength, 128.0 / 255.0 * strength, 0.0, 1.0]);
    assert_eq!(g.pos, (u.pos - Vec2::splat(r)).to_array());

    let hidden_bloom = Settings {
        hide_bloom: true,
        ..Settings::default()
    };
    let f = build(&v, &hidden_bloom);
    assert_eq!(f.bloom.len(), 4, "only the glow");
    for hide in [
        Settings {
            hide_decorations: true,
            ..Settings::default()
        },
        Settings {
            hide_users: true,
            ..Settings::default()
        },
    ] {
        let f = build(&v, &hide);
        assert_eq!(f.bloom.len(), 8, "only the dir blooms");
        assert!(deco_commands(&f).is_empty());
    }
}

#[test]
fn hide_decorations_gives_the_plain_frame() {
    let plain = build(&scene(VP, 1.0, [NONE; 3]), &Settings::default());
    let hidden = Settings {
        hide_decorations: true,
        ..Settings::default()
    };
    let f = build(&scene(VP, 1.0, [decor(ALL); 3]), &hidden);
    assert_eq!(f, plain);
}

/// Device-pixel height of quad `q` of `f` drawn in world space.
fn screen_height(f: &Frame, v: &SceneView, q: usize) -> f32 {
    let m = v.camera.world_to_clip(v.viewport);
    let p = |i: usize| {
        let p = f.quads[q * 4 + i].pos;
        project_with(&m, v.viewport, Vec2::from(p).extend(0.0))
    };
    (p(3) - p(0)).length()
}

#[test]
fn badge_keeps_a_minimum_size_that_scales_with_ui_scale() {
    let badge = |vp: Vec2, ui: f32, camera_z: f32| {
        let mut v = scene(vp, ui, [decor(DecoKind::Badge.bit()), NONE, NONE]);
        v.camera.pos.z = camera_z;
        let f = build(&v, &Settings::default());
        let q = deco_commands(&f)[0].first_quad as usize;
        let users = users_command(&f).first_quad as usize;
        (screen_height(&f, &v, q), screen_height(&f, &v, users))
    };
    // at the default distance the badge is held at 8 px x ui_scale (the
    // avatar is 32 px tall, the anchor would give 5 px)
    let (b1, a1) = badge(VP, 1.0, -300.0);
    assert!((b1 - BADGE_MIN_PX).abs() < 1e-3, "{b1}");
    let (b2, a2) = badge(VP * 2.0, 2.0, -300.0);
    assert!((b2 - 2.0 * BADGE_MIN_PX).abs() < 1e-3, "{b2}");
    assert!((b1 / a1 - b2 / a2).abs() < 1e-4, "{} {}", b1 / a1, b2 / a2);
    // close up the anchor rectangle is already larger
    let (b, a) = badge(VP, 1.0, -100.0);
    assert!((b / a - (BADGE_RECT[3] - BADGE_RECT[1])).abs() < 1e-4);
    // far away the minimum is capped at 0.4 of the avatar
    let (b, a) = badge(VP, 1.0, -9000.0);
    assert!((b / a - 0.4).abs() < 1e-3, "{}", b / a);
}

#[test]
fn agent_key_and_filter_line_are_text_only() {
    let s = Settings::default();
    let base = scene(VP, 1.0, [NONE; 3]);
    let plain = build(&base, &s);

    let mut v = base.clone();
    v.hud.show_agent_key = true;
    for (name, count) in [("Claude", 1204), ("Codex", 311)] {
        let name = v.push_text(name);
        v.hud.agents.push(AgentKeyRow {
            name,
            colour: Vec3::new(0.9, 0.6, 0.0),
            count,
        });
    }
    v.hud.agents_more = 2;
    v.hud.filter_note = Some(v.push_text("AI commits: only assisted"));
    let f = build(&v, &s);
    assert_eq!(f.commands.len(), plain.commands.len());
    assert_eq!(f.quads, plain.quads);
    let glyphs = |f: &Frame| f.text.len() / 4;
    // the test glyphs draw every character, spaces too: bullets and their
    // space, names, counts, "2 more", and the filter line in the date font,
    // which has a drop shadow (each glyph twice)
    let added = 2 * 2
        + "Claude".len()
        + "Codex".len()
        + "1,204".len()
        + "311".len()
        + "2 more".len()
        + 2 * "AI commits: only assisted".len();
    assert_eq!(glyphs(&f), glyphs(&plain) + added);
    // the bullet carries the agent colour
    assert!(f.text.iter().any(|q| q.colour == [0.9, 0.6, 0.0, 1.0]));
    // the key starts below the web page's top-right toolbar (CSS y 12 to
    // 56): its first glyph, the bullet, is the first one in the agent colour
    let bullet_top = f
        .text
        .iter()
        .filter(|q| q.colour == [0.9, 0.6, 0.0, 1.0])
        .map(|q| q.pos[1])
        .fold(f32::INFINITY, f32::min);
    assert!(bullet_top >= 64.0, "{bullet_top}");

    // the filter line sits under the date, or in its place with hide=date
    let line_y = |v: &SceneView, s: &Settings| {
        let f = build(v, s);
        let plain = build(
            &SceneView {
                hud: gource_core::view::HudView {
                    filter_note: None,
                    ..v.hud.clone()
                },
                ..v.clone()
            },
            s,
        );
        // the glyphs the filter line added are the last ones of the text
        // command that also holds the date
        let first_new = (0..f.text.len() / 4)
            .find(|&q| plain.text.get(q * 4).map(|v| v.pos) != Some(f.text[q * 4].pos))
            .unwrap();
        f.text[first_new * 4].pos[1]
    };
    let mut only_note = base.clone();
    only_note.hud.filter_note = Some(only_note.push_text("AI commits: only assisted"));
    let under = line_y(&only_note, &s);
    let mut no_date = only_note.clone();
    no_date.hud.date = None;
    let hide_date = Settings {
        hide_date: true,
        ..Settings::default()
    };
    let instead = line_y(&no_date, &hide_date);
    let row = s.scaled_font_size() as f32 + 6.0;
    assert!((under - instead - row).abs() < 1e-3, "{under} {instead}");
}

#[test]
fn deco_texture_mip_levels_keep_the_gutter() {
    assert_eq!(DECO_TEXTURE_MAX_LEVEL, 2);
    const { assert!(GUTTER >> DECO_TEXTURE_MAX_LEVEL >= 1) };
    const { assert!(CELL_SIZE >> DECO_TEXTURE_MAX_LEVEL >= 16) };
}

#[test]
fn pr_key_coverage_and_repository_flow_reuse_the_existing_text_pass() {
    let s = Settings::default();
    let base = scene(Vec2::new(390.0, 384.0), 1.0, [NONE; 3]);
    let plain = build(&base, &s);
    let mut view = base.clone();
    for line in [
        "PR: P open D draft M merged X closed",
        "Review: A approved ! changes C comment",
    ] {
        let text = view.push_text(line);
        view.hud.pr_key.push(text);
    }
    let text = view.push_text("Lifecycle: partial");
    view.hud.pr_coverage.push(text);
    view.hud.pr_flow = Some(view.push_text("PRs repository-wide: 7 open"));
    let rich = build(&view, &s);
    assert_eq!(rich.commands.len(), plain.commands.len());
    assert_eq!(rich.quads, plain.quads);
    assert!(
        rich.text.len() > plain.text.len(),
        "PR meanings must be drawn, not just stored"
    );
    let key_capacity = view.hud.pr_key.capacity();
    let coverage_capacity = view.hud.pr_coverage.capacity();
    view.clear();
    assert!(view.hud.pr_key.is_empty());
    assert!(view.hud.pr_coverage.is_empty());
    assert!(view.hud.pr_flow.is_none());
    assert_eq!(view.hud.pr_key.capacity(), key_capacity);
    assert_eq!(view.hud.pr_coverage.capacity(), coverage_capacity);
}

#[test]
fn pr_hud_rows_fit_without_overlapping_at_short_height_and_large_fonts() {
    for (height, font_size) in [(200.0, 14), (384.0, 48), (800.0, 14)] {
        let s = Settings {
            font_size,
            ..Settings::default()
        };
        let mut view = SceneView {
            viewport: Vec2::new(390.0, height),
            ui_scale: 1.0,
            ..SceneView::default()
        };
        for line in [
            "P open D draft M merged X closed",
            "A approved ! changes C comment",
            "Age: replay days / target in hover",
        ] {
            let text = view.push_text(line);
            view.hud.pr_key.push(text);
        }
        for line in ["Lifecycle: complete", "Reviews: partial", "Files: unknown"] {
            let text = view.push_text(line);
            view.hud.pr_coverage.push(text);
        }
        view.hud.pr_flow = Some(view.push_text(
            "PRs (repository): 18446744073709551615 open, oldest 18446744073709551615d",
        ));
        let frame = build(&view, &s);
        let mut rows: Vec<(f32, f32)> = Vec::new();
        for glyph in frame.text.as_chunks::<4>().0 {
            let y = glyph.iter().map(|v| v.pos[1]).fold(f32::INFINITY, f32::min);
            let bottom = glyph
                .iter()
                .map(|v| v.pos[1])
                .fold(f32::NEG_INFINITY, f32::max);
            let right = glyph
                .iter()
                .map(|v| v.pos[0])
                .fold(f32::NEG_INFINITY, f32::max);
            assert!(
                right <= 380.0 + 1.0,
                "row clips horizontally at {height}/{font_size}: {right}"
            );
            assert!(
                y >= 0.0 && bottom <= height - 20.0 + 1.0,
                "row clips vertically at {height}/{font_size}: {y}..{bottom}"
            );
            // Immediate-mode shadow is a second glyph one pixel below the main one.
            if let Some(row) = rows.iter_mut().find(|row| (row.0 - y).abs() <= 1.0) {
                row.1 = row.1.max(bottom);
            } else {
                rows.push((y, bottom));
            }
        }
        rows.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert_eq!(
            rows.len(),
            7,
            "all meanings, coverage and flow remain visible"
        );
        assert!(
            rows.windows(2).all(|r| r[0].1 <= r[1].0),
            "overlapping PR HUD rows: {rows:?}"
        );
        assert!(frame.commands.len() <= 12);
    }
}

#[test]
fn pr_hud_stays_clear_of_mobile_keyboard_hint_and_player() {
    let mut view = SceneView {
        viewport: Vec2::new(390.0, 800.0),
        ui_scale: 1.0,
        ..SceneView::default()
    };
    view.hud.date = Some(view.push_text("Monday, 01 January, 2024"));
    for line in ["Lifecycle: complete", "Reviews: partial", "Files: unknown"] {
        let text = view.push_text(line);
        view.hud.pr_coverage.push(text);
    }
    view.hud.pr_flow = Some(view.push_text("PRs (repository): 1 open, oldest 47d"));
    for line in [
        "PR: P open / D draft / M merged / X closed",
        "Review: A approved / ! changes / C comment",
        "Age: replay days / target in hover",
    ] {
        let text = view.push_text(line);
        view.hud.pr_key.push(text);
    }
    let frame = build(&view, &Settings::default());
    let mut pr_glyphs = 0;
    for glyph in frame.text.as_chunks::<4>().0 {
        let top = glyph.iter().map(|v| v.pos[1]).fold(f32::INFINITY, f32::min);
        let bottom = glyph
            .iter()
            .map(|v| v.pos[1])
            .fold(f32::NEG_INFINITY, f32::max);
        if top < 45.0 {
            continue; // The unchanged ordinary date occupies its existing top row.
        }
        pr_glyphs += 1;
        assert!(
            top >= 104.0,
            "PR text under the mobile keyboard hint: {top}"
        );
        assert!(
            bottom <= 620.0,
            "PR text under the fixed mobile player: {bottom}"
        );
    }
    assert!(pr_glyphs > 0);
    assert!(frame.commands.len() <= 12);
}

#[test]
fn rich_hud_reserves_f1_columns_without_moving_or_evicting_agent_rows() {
    let bounds = |quad: &[crate::QuadVertex]| {
        [
            quad.iter().map(|v| v.pos[0]).fold(f32::INFINITY, f32::min),
            quad.iter().map(|v| v.pos[1]).fold(f32::INFINITY, f32::min),
            quad.iter()
                .map(|v| v.pos[0])
                .fold(f32::NEG_INFINITY, f32::max),
            quad.iter()
                .map(|v| v.pos[1])
                .fold(f32::NEG_INFINITY, f32::max),
        ]
    };
    for scale in [1.0, 2.0, 0.666_666_7] {
        for first in ["Claude", "a-very-long-registered-agent-name"] {
            let mut view = SceneView {
                viewport: Vec2::new(390.0, 800.0) * scale,
                ui_scale: scale,
                ..SceneView::default()
            };
            view.hud.show_agent_key = true;
            for name in [
                first, "Codex", "Copilot", "Cursor", "Gemini", "OpenAI", "Grok", "Aider",
            ] {
                let name = view.push_text(name);
                view.hud.agents.push(AgentKeyRow {
                    name,
                    colour: Vec3::new(0.2, 0.3, 0.4),
                    count: 1,
                });
            }
            view.hud.agents_more = 2;
            let f1 = build(&view, &Settings::default());
            for line in ["Lifecycle: complete", "Reviews: partial", "Files: unknown"] {
                let text = view.push_text(line);
                view.hud.pr_coverage.push(text);
            }
            view.hud.pr_flow = Some(view.push_text("PRs (repository): 1 open, oldest 47d"));
            for line in [
                "PR: P open / D draft / M merged / X closed",
                "Review: A approved / ! changes / C comment",
                "Age: replay days / target in hover",
            ] {
                let text = view.push_text(line);
                view.hud.pr_key.push(text);
            }
            let combined = build(&view, &Settings::default());
            let pr_len = combined.text.len() - f1.text.len();
            assert!(pr_len > 0);
            assert_eq!(&combined.text[pr_len..], f1.text, "F1 pixels are unchanged");
            let mut rows = Vec::new();
            for pr in combined.text[..pr_len].as_chunks::<4>().0 {
                let p = bounds(pr);
                if pr[0].colour[0] > 0.9 {
                    rows.push(p[1]);
                }
                for agent in f1.text.as_chunks::<4>().0 {
                    let a = bounds(agent);
                    assert!(
                        p[2] <= a[0] || a[2] <= p[0] || p[3] <= a[1] || a[3] <= p[1],
                        "PR/F1 glyph intersection at scale {scale}: {p:?} and {a:?}"
                    );
                }
            }
            rows.sort_by(f32::total_cmp);
            rows.dedup_by(|a, b| (*a - *b).abs() < scale);
            assert_eq!(rows.len(), 7, "all PR meanings and coverage survive");
            assert_eq!(combined.commands.len(), f1.commands.len());
            assert!(combined.commands.len() <= 12);
        }
    }
}
