// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

use glam::{Vec2, Vec3, Vec4};

use gource_core::settings::Settings;
use gource_core::view::{
    ActionView, DirView, ExtensionCount, FileView, FontRole, LabelAnchor, LabelView, SceneView,
    TextBoxView, UserView,
};

use super::*;
use crate::spline::{Spline, label_pos};
use crate::{Blend, Frame, FrameBuilder, MonospaceGlyphs, Program, Space, Stream, TextureId};

const VP: Vec2 = Vec2::new(1280.0, 720.0);

fn dir(parent: Option<u32>, pos: Vec2, spos: Vec2, colour: Vec4) -> DirView {
    DirView {
        parent,
        depth: parent.map_or(1, |_| 2),
        pos,
        spos,
        radius: 12.0,
        colour,
        in_frustum: true,
        visible: true,
        draw_edge: parent.is_some(),
        draw_bloom: true,
    }
}

fn label(v: &mut SceneView, anchor: LabelAnchor, text: &str, font: FontRole) -> LabelView {
    LabelView {
        anchor,
        text: v.push_text(text),
        font,
        colour: Vec4::new(1.0, 1.0, 1.0, 0.8),
    }
}

/// A small scene with one of everything.
fn scene() -> SceneView {
    let mut v = SceneView {
        viewport: VP,
        ui_scale: 1.0,
        background: Vec3::splat(0.1),
        ..SceneView::default()
    };
    v.dirs.push(dir(
        None,
        Vec2::ZERO,
        Vec2::ZERO,
        Vec4::new(1.0, 1.0, 1.0, 1.0),
    ));
    v.dirs.push(dir(
        Some(0),
        Vec2::new(100.0, 0.0),
        Vec2::new(50.0, 30.0),
        Vec4::new(0.5, 0.2, 0.1, 1.0),
    ));
    v.dirs.push(dir(
        Some(1),
        Vec2::new(100.0, 100.0),
        Vec2::new(110.0, 50.0),
        Vec4::new(0.2, 0.4, 0.8, 1.0),
    ));
    for i in 0..3 {
        v.files.push(FileView {
            pos: Vec2::new(100.0 + 8.0 * i as f32, 0.0),
            size: Vec2::splat(8.0),
            colour: Vec3::new(0.2, 0.8, 0.4),
            alpha: 1.0,
        });
    }
    for i in 0..2 {
        v.users.push(UserView {
            pos: Vec2::new(-50.0, 40.0 * i as f32),
            size: Vec2::splat(20.0),
            colour: Vec3::new(0.9, 0.6, 0.6),
            alpha: 1.0,
            ..UserView::default()
        });
        v.actions.push(ActionView {
            src: Vec2::new(-50.0, 40.0 * i as f32),
            dest: Vec2::new(100.0, 0.0),
            target_size: 8.0,
            colour: Vec3::new(1.0, 0.5, 0.0),
            progress: 0.25,
        });
    }
    let l = label(
        &mut v,
        LabelAnchor::Dir { dir: 1 },
        "src",
        FontRole::DirName,
    );
    v.labels.push(l);
    let l = label(
        &mut v,
        LabelAnchor::Dir { dir: 2 },
        "core",
        FontRole::DirName,
    );
    v.labels.push(l);
    let l = label(
        &mut v,
        LabelAnchor::WorldTopLeft {
            pos: Vec2::new(105.5, -1.0),
        },
        "main.rs",
        FontRole::FileName,
    );
    v.labels.push(l);
    let l = label(
        &mut v,
        LabelAnchor::WorldCentredAbove {
            pos: Vec2::new(-50.0, -10.0),
        },
        "alice",
        FontRole::UserName,
    );
    v.labels.push(l);
    let l = label(
        &mut v,
        LabelAnchor::WorldCentredAbove {
            pos: Vec2::new(-50.0, 30.0),
        },
        "bob",
        FontRole::UserSelected,
    );
    v.labels.push(l);

    v.hud.date = Some(v.push_text("Monday, 01 January, 2024 12:00:00"));
    v.hud.title = Some(v.push_text("Gource"));
    let msg = v.push_text("Wrote screenshot");
    v.hud.message = Some((msg, 2.0));
    v.hud.show_key = true;
    for (ext, count) in [("rs", 2u32), ("md", 1)] {
        let r = v.push_text(ext);
        v.hud.extensions.push(ExtensionCount {
            ext: r,
            colour: Vec3::new(0.3, 0.6, 0.9),
            count,
        });
    }
    let line = v.push_text("main.rs");
    v.hud.textbox = Some(TextBoxView {
        lines: vec![line],
        colour: Vec3::new(0.2, 0.8, 0.4),
        pos: Vec2::new(400.0, 300.0),
    });
    v
}

fn build(b: &mut FrameBuilder, v: &SceneView, s: &Settings) -> Frame {
    let mut f = Frame::default();
    b.build(v, s, 1.0 / 60.0, &mut MonospaceGlyphs, &mut f);
    f
}

/// Two builds, so the file key (which needs one interval tick) is visible.
fn build_warm(v: &SceneView, s: &Settings) -> Frame {
    let mut b = FrameBuilder::new();
    build(&mut b, v, s);
    build(&mut b, v, s)
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum P {
    Textured,
    Shadow,
    Bloom,
    TextShader,
    TextPlain,
}

type Sig = (Stream, P, Space, TextureId, Blend);

fn sig(c: &DrawCommand) -> Sig {
    let p = match c.program {
        Program::Textured => P::Textured,
        Program::Shadow { strength } => {
            assert_eq!(strength, 0.5);
            P::Shadow
        }
        Program::Bloom => P::Bloom,
        Program::Text {
            shadow_strength, ..
        } => {
            if shadow_strength > 0.0 {
                assert_eq!(shadow_strength, 0.7);
                P::TextShader
            } else {
                P::TextPlain
            }
        }
    };
    (c.stream, p, c.space, c.texture, c.blend)
}

const EDGE_SHADOW: Sig = (
    Stream::Quads,
    P::Shadow,
    Space::Screen,
    TextureId::Beam,
    Blend::Alpha,
);
const EDGES: Sig = (
    Stream::Quads,
    P::Textured,
    Space::Screen,
    TextureId::Beam,
    Blend::Alpha,
);
const FILE_SHADOW: Sig = (
    Stream::Quads,
    P::Shadow,
    Space::World,
    TextureId::File,
    Blend::Alpha,
);
const ACTIONS: Sig = (
    Stream::Quads,
    P::Textured,
    Space::World,
    TextureId::Beam,
    Blend::Alpha,
);
const FILES: Sig = (
    Stream::Quads,
    P::Textured,
    Space::World,
    TextureId::File,
    Blend::Alpha,
);
const USER_SHADOW: Sig = (
    Stream::Quads,
    P::Shadow,
    Space::World,
    TextureId::User,
    Blend::Alpha,
);
const USERS: Sig = (
    Stream::Quads,
    P::Textured,
    Space::World,
    TextureId::User,
    Blend::Alpha,
);
const BLOOM: Sig = (
    Stream::Bloom,
    P::Bloom,
    Space::World,
    TextureId::White,
    Blend::Additive,
);
const NAMES: Sig = (
    Stream::Text,
    P::TextShader,
    Space::Screen,
    TextureId::Glyphs,
    Blend::Alpha,
);
const TEXT: Sig = (
    Stream::Text,
    P::TextPlain,
    Space::Screen,
    TextureId::Glyphs,
    Blend::Alpha,
);
const HUD_QUADS: Sig = (
    Stream::Quads,
    P::Textured,
    Space::Screen,
    TextureId::White,
    Blend::Alpha,
);

fn sigs(f: &Frame) -> Vec<Sig> {
    f.commands.iter().map(sig).collect()
}

fn find(f: &Frame, s: Sig) -> &DrawCommand {
    f.commands
        .iter()
        .find(|c| sig(c) == s)
        .unwrap_or_else(|| panic!("no command {s:?}"))
}

#[test]
fn empty_view_draws_nothing() {
    let mut b = FrameBuilder::new();
    for _ in 0..3 {
        let f = build(&mut b, &SceneView::default(), &Settings::default());
        assert!(f.commands.is_empty());
        assert!(f.quads.is_empty() && f.bloom.is_empty() && f.text.is_empty());
    }
}

#[test]
fn frame_header_matrices_and_clear_colour() {
    let v = scene();
    let f = build_warm(&v, &Settings::default());
    assert_eq!(f.viewport, [1280, 720]);
    assert_eq!(f.clear_colour, [0.1, 0.1, 0.1, 1.0]);
    assert_eq!(f.world_to_clip, v.camera.world_to_clip(VP).to_cols_array());
    // glOrtho(0, w, h, 0, -1, 1): (0,0) → (-1, 1), (w,h) → (1, -1).
    let m = glam::Mat4::from_cols_array(&f.screen_to_clip);
    let a = m * Vec4::new(0.0, 0.0, 0.0, 1.0);
    let b = m * Vec4::new(1280.0, 720.0, 0.0, 1.0);
    assert!((a - Vec4::new(-1.0, 1.0, 0.0, 1.0)).length() < 1e-6);
    assert!((b - Vec4::new(1.0, -1.0, 0.0, 1.0)).length() < 1e-6);
}

#[test]
fn default_command_sequence_follows_gource_draw() {
    let f = build_warm(&scene(), &Settings::default());
    assert_eq!(
        sigs(&f),
        [
            EDGE_SHADOW,
            EDGES,
            FILE_SHADOW,
            ACTIONS,
            FILES,
            USER_SHADOW,
            USERS,
            BLOOM,
            NAMES,
            TEXT,      // selected user name, date, title (merged)
            HUD_QUADS, // file key boxes
            TEXT,      // file key texts
            HUD_QUADS, // text box
            TEXT,      // text box line, message (merged)
        ]
    );
    assert!(f.commands.len() <= 16);
    // Shadow passes reuse the ranges of their textured passes.
    for (shadow, textured) in [(0, 1), (2, 4), (5, 6)] {
        assert_eq!(
            f.commands[shadow].first_quad,
            f.commands[textured].first_quad
        );
        assert_eq!(
            f.commands[shadow].quad_count,
            f.commands[textured].quad_count
        );
    }
    assert_eq!(f.commands[0].offset, [2.0, 2.0]);
    assert_eq!(f.commands[1].offset, [0.0, 0.0]);
    assert_eq!(f.commands[2].offset, [2.0, 2.0]);
    assert_eq!(f.commands[5].offset, [2.0, 2.0]);
}

#[test]
fn first_frame_has_no_file_key_yet() {
    let mut b = FrameBuilder::new();
    let f = build(&mut b, &scene(), &Settings::default());
    assert_eq!(
        sigs(&f)[8..],
        [NAMES, TEXT, HUD_QUADS, TEXT],
        "key waits for its first update tick"
    );
}

#[test]
fn quad_counts_per_stream() {
    let v = scene();
    let f = build_warm(&v, &Settings::default());
    let count = |s: Sig| find(&f, s).quad_count;

    let expected_edges: usize = [(0usize, 1usize), (1, 2)]
        .iter()
        .map(|&(p, c)| {
            let pp = v.camera.project(VP, v.dirs[p].pos.extend(0.0));
            let cp = v.camera.project(VP, v.dirs[c].pos.extend(0.0));
            let sp = v.camera.project(VP, v.dirs[c].spos.extend(0.0));
            crate::spline::edge_detail(pp, cp, sp)
        })
        .sum();
    assert_eq!(count(EDGES) as usize, expected_edges);
    assert_eq!(count(FILES), 3);
    assert_eq!(count(ACTIONS), 2);
    assert_eq!(count(USERS), 2);
    assert_eq!(count(BLOOM), 3);
    assert_eq!(f.bloom.len(), 12);

    // Quads stream: edges + files + actions + users + 2 key entries × 2 + text box × 2.
    assert_eq!(f.quads.len() / 4, expected_edges + 3 + 2 + 2 + 4 + 2);

    // Names: "src" + "core" + "main.rs" + "alice" (shadow from the shader).
    assert_eq!(count(NAMES), 3 + 4 + 7 + 5);
    // Every glyph quad of the text stream is referenced exactly once.
    let text_quads: u32 = f
        .commands
        .iter()
        .filter(|c| c.stream == Stream::Text)
        .map(|c| c.quad_count)
        .sum();
    assert_eq!(text_quads as usize, f.text.len() / 4);
}

#[test]
fn hide_flags_remove_their_passes() {
    let v = scene();
    let base = sigs(&build_warm(&v, &Settings::default()));
    let without =
        |drop: &[Sig]| -> Vec<Sig> { base.iter().copied().filter(|s| !drop.contains(s)).collect() };

    let check = |s: Settings, expected: Vec<Sig>| {
        let f = build_warm(&v, &s);
        assert_eq!(sigs(&f), expected, "{s:?}");
        f
    };

    check(
        Settings {
            hide_tree: true,
            ..Settings::default()
        },
        without(&[EDGE_SHADOW, EDGES]),
    );
    let f = check(
        Settings {
            hide_files: true,
            ..Settings::default()
        },
        without(&[FILE_SHADOW, FILES]),
    );
    // hide_files also hides file names.
    assert_eq!(find(&f, NAMES).quad_count, 3 + 4 + 5);
    let f = check(
        Settings {
            hide_users: true,
            ..Settings::default()
        },
        without(&[ACTIONS, USER_SHADOW, USERS]),
    );
    assert_eq!(find(&f, NAMES).quad_count, 3 + 4 + 7);
    check(
        Settings {
            hide_bloom: true,
            ..Settings::default()
        },
        without(&[BLOOM]),
    );

    let names = |s: Settings| find(&build_warm(&v, &s), NAMES).quad_count;
    assert_eq!(
        names(Settings {
            hide_dirnames: true,
            ..Settings::default()
        }),
        7 + 5
    );
    assert_eq!(
        names(Settings {
            hide_filenames: true,
            ..Settings::default()
        }),
        3 + 4 + 5
    );
    assert_eq!(
        names(Settings {
            hide_usernames: true,
            ..Settings::default()
        }),
        3 + 4 + 7
    );

    // hide_date removes the date glyphs but keeps the title.
    let all = build_warm(&v, &Settings::default());
    let no_date = build_warm(
        &v,
        &Settings {
            hide_date: true,
            ..Settings::default()
        },
    );
    let date_len = v.str(v.hud.date.unwrap()).chars().count();
    // Date glyphs and their immediate-mode shadows.
    assert_eq!(all.text.len() / 4 - no_date.text.len() / 4, 2 * date_len);

    // Everything hidden: only the HUD remains.
    let f = build_warm(
        &v,
        &Settings {
            hide_tree: true,
            hide_files: true,
            hide_users: true,
            hide_bloom: true,
            hide_dirnames: true,
            hide_date: true,
            ..Settings::default()
        },
    );
    assert_eq!(sigs(&f), [TEXT, HUD_QUADS, TEXT, HUD_QUADS, TEXT]);
}

#[test]
fn key_hidden_when_show_key_off() {
    let mut v = scene();
    v.hud.show_key = false;
    v.hud.textbox = None;
    v.hud.message = None;
    let f = build_warm(&v, &Settings::default());
    assert_eq!(sigs(&f)[8..], [NAMES, TEXT]);
}

#[test]
fn edges_match_spline_of_projected_nodes() {
    let v = scene();
    let f = build_warm(&v, &Settings::default());
    let p = |w: Vec2| v.camera.project(VP, w.extend(0.0));
    let mut expected = Vec::new();
    for (pi, ci) in [(0usize, 1usize), (1, 2)] {
        Spline::new(
            p(v.dirs[pi].pos),
            v.dirs[pi].colour,
            p(v.dirs[ci].pos),
            v.dirs[ci].colour,
            p(v.dirs[ci].spos),
        )
        .push_quads(2.5, &mut expected);
    }
    let edges = find(&f, EDGES);
    let first = edges.first_quad as usize * 4;
    assert_eq!(&f.quads[first..first + expected.len()], &expected[..]);
    // The child end of the first edge sits on the projected child node.
    let child = p(v.dirs[1].pos);
    let mid = (Vec2::from(f.quads[first].pos) + Vec2::from(f.quads[first + 1].pos)) * 0.5;
    assert!((mid - child).length() < 1e-3);
}

#[test]
fn hide_root_edges_follow_draw_edge() {
    let mut v = scene();
    v.dirs[1].draw_edge = false; // as the core sets it for hide_root
    let f = build_warm(&v, &Settings::default());
    let p = |w: Vec2| v.camera.project(VP, w.extend(0.0));
    let only = crate::spline::edge_detail(p(v.dirs[1].pos), p(v.dirs[2].pos), p(v.dirs[2].spos));
    assert_eq!(find(&f, EDGES).quad_count as usize, only);
}

#[test]
fn files_and_users_are_centred_quads() {
    let v = scene();
    let f = build_warm(&v, &Settings::default());
    let files = find(&f, FILES);
    let q = &f.quads[files.first_quad as usize * 4..][..4];
    assert_eq!(q[0].pos, [96.0, -4.0]);
    assert_eq!(q[2].pos, [104.0, 4.0]);
    assert_eq!(q[0].uv, [0.0, 0.0]);
    assert_eq!(q[2].uv, [1.0, 1.0]);
    assert_eq!(q[0].colour, [0.2, 0.8, 0.4, 1.0]);

    let users = find(&f, USERS);
    let q = &f.quads[users.first_quad as usize * 4..][..4];
    assert_eq!(q[0].pos, [-60.0, -10.0]);
    assert_eq!(q[1].pos, [-40.0, -10.0]);

    let s = Settings {
        user_scale: 2.5,
        ..Settings::default()
    };
    let f = build_warm(&v, &s);
    assert_eq!(find(&f, USER_SHADOW).offset, [5.0, 5.0]);
}

#[test]
fn bloom_quads_pack_radius_and_centre() {
    let v = scene();
    let s = Settings {
        bloom_multiplier: 1.5,
        bloom_intensity: 0.5,
        ..Settings::default()
    };
    let f = build_warm(&v, &s);
    // dir 1: radius 12 × 2 × 1.5 = 36, quad 72 wide around (100, 0).
    let q = &f.bloom[4..8];
    assert_eq!(q[0].pos, [64.0, -36.0]);
    assert_eq!(q[1].pos, [136.0, -36.0]);
    assert_eq!(q[2].pos, [136.0, 36.0]);
    assert_eq!(q[3].pos, [64.0, 36.0]);
    for vert in q {
        assert_eq!(vert.texcoord, [36.0, 100.0, 0.0, 0.0]);
        assert_eq!(vert.colour, [0.25, 0.1, 0.05, 1.0]);
    }

    let mut v = scene();
    v.dirs[2].draw_bloom = false;
    let f = build_warm(&v, &s);
    assert_eq!(find(&f, BLOOM).quad_count, 2);
}

#[test]
fn action_beam_geometry() {
    let mut out = Vec::new();
    action_quad(
        &mut out,
        Vec2::new(0.0, 0.0),
        Vec2::new(100.0, 0.0),
        8.0,
        Vec3::new(1.0, 0.5, 0.0),
        0.25,
    );
    // n = (1, 0), perp = (0, 1), offset = (0, 4), offset_src = (0, 1.2).
    assert_eq!(out[0].pos, [0.0, -1.2]);
    assert_eq!(out[1].pos, [0.0, 1.2]);
    assert_eq!(out[2].pos, [100.0, 4.0]);
    assert_eq!(out[3].pos, [100.0, -4.0]);
    assert_eq!(out[0].uv, [0.0, 0.0]);
    assert_eq!(out[1].uv, [0.0, 1.0]);
    assert_eq!(out[2].uv, [1.0, 1.0]);
    assert_eq!(out[3].uv, [1.0, 0.0]);
    assert_eq!(out[0].colour, [1.0, 0.5, 0.0, 0.075]);
    assert_eq!(out[2].colour, [1.0, 0.5, 0.0, 0.75]);

    // Degenerate beam (user on the file): normalise keeps the zero vector.
    out.clear();
    action_quad(&mut out, Vec2::ONE, Vec2::ONE, 8.0, Vec3::ONE, 0.0);
    assert!(out.iter().all(|q| q.pos == [1.0, 1.0]));
}

/// Top-left of the first glyph quad a label produced.
fn glyph_origin(f: &Frame, quad: usize) -> Vec2 {
    Vec2::from(f.text[quad * 4].pos)
}

#[test]
fn label_placement() {
    let v = scene();
    let f = build_warm(&v, &Settings::default());
    let p = |w: Vec2| v.camera.project(VP, w.extend(0.0));
    // MonospaceGlyphs at 14 px: bearing (0, -11.2); align top adds ceil(8.4) = 9.
    let bearing_14 = -14.0 * 0.8;

    // Dir "src" at the spline label position of dir 1's edge.
    let lp = label_pos(p(v.dirs[0].pos), p(v.dirs[1].pos), p(v.dirs[1].spos), 0.5);
    let g = glyph_origin(&f, 0);
    assert!(
        (g - Vec2::new(lp.x, lp.y + 9.0 + bearing_14)).length() < 1e-3,
        "{g:?} {lp:?}"
    );

    // File name: top-left at the projected point (3 + 4 glyphs before it).
    let fp = p(Vec2::new(105.5, -1.0));
    let g = glyph_origin(&f, 7);
    assert!((g - Vec2::new(fp.x, fp.y + 9.0 + bearing_14)).length() < 1e-3);

    // User name: centred, baseline one max height above the point.
    let up = p(Vec2::new(-50.0, -10.0));
    let width = 5.0 * 14.0 * 0.6;
    let g = glyph_origin(&f, 14);
    assert!((g - Vec2::new(up.x - width * 0.5, up.y - 14.0 + bearing_14)).length() < 1e-3);

    // Selected user (18 px, immediate): shadow copy first, at +(1, 1).
    let names = find(&f, NAMES);
    let sel = (names.first_quad + names.quad_count) as usize;
    let sp = p(Vec2::new(-50.0, 30.0));
    let width = 3.0 * 18.0 * 0.6;
    let text_origin = Vec2::new(sp.x - width * 0.5, sp.y - 18.0 - 18.0 * 0.8);
    assert!((glyph_origin(&f, sel) - (text_origin + Vec2::ONE)).length() < 1e-3);
    assert_eq!(f.text[sel * 4].colour, [0.0, 0.0, 0.0, 0.8 * 0.7]);
    assert!((glyph_origin(&f, sel + 3) - text_origin).length() < 1e-3);
    assert_eq!(f.text[(sel + 3) * 4].colour, [1.0, 1.0, 1.0, 0.8]);
}

#[test]
fn dir_name_position_moves_dir_labels() {
    let v = scene();
    let s = Settings {
        dir_name_position: 1.0,
        ..Settings::default()
    };
    let f = build_warm(&v, &s);
    let child = v.camera.project(VP, v.dirs[1].pos.extend(0.0));
    let g = glyph_origin(&f, 0);
    assert!((g - Vec2::new(child.x, child.y + 9.0 - 14.0 * 0.8)).length() < 1e-3);
}

#[test]
fn date_title_and_message_positions() {
    let v = scene();
    let f = build_warm(&v, &Settings::default());
    let names = find(&f, NAMES);
    // After the 3-glyph selected name (6 quads with shadow): the date.
    let date_first = (names.first_quad + names.quad_count) as usize + 6;
    let date = v.str(v.hud.date.unwrap());
    // fontmedium 16 px: width 9.6 per char; date_x_offset = (int)(int)w * 0.5.
    let w = (date.chars().count() as f32 * 9.6) as i32;
    let x = (640 - (w as f64 * 0.5) as i32) as f32;
    // align top: ceil(12.8 - 3.2) = 10; bearing -12.8. Shadow copy first.
    let y = 20.0 + 10.0 - 12.8;
    let n = date.chars().count();
    assert!((glyph_origin(&f, date_first) - Vec2::new(x + 1.0, y + 1.0)).length() < 1e-3);
    assert!((glyph_origin(&f, date_first + n) - Vec2::new(x, y)).length() < 1e-3);

    // Title: baseline at (10, h - 10), no top alignment.
    let date_glyphs = date.chars().count();
    let title_first = date_first + 2 * date_glyphs;
    assert!((glyph_origin(&f, title_first + 6) - Vec2::new(10.0, 710.0 - 12.8)).length() < 1e-3);

    // Message: last text, font 14 rounded, at (1, 3 + 9).
    let msg = v.str(v.hud.message.unwrap().0);
    let msg_glyphs = msg.chars().count();
    let msg_first = f.text.len() / 4 - 2 * msg_glyphs;
    assert!(
        (glyph_origin(&f, msg_first + msg_glyphs) - Vec2::new(1.0, 12.0 - 11.2)).length() < 1e-3
    );
}

#[test]
fn date_offset_ignores_small_width_changes() {
    let mut b = FrameBuilder::new();
    let mut v = SceneView {
        viewport: VP,
        ui_scale: 1.0,
        ..SceneView::default()
    };
    let s = Settings::default();
    let mut set = |b: &mut FrameBuilder, text: &str| {
        v.clear();
        v.hud.date = Some(v.push_text(text));
        build(b, &v, &s);
        b.date_x_offset
    };
    // 10 chars × 9.6 = 95.99999 (f32) → (int) 95 × 0.5 → 47.
    assert_eq!(set(&mut b, "0123456789"), 47);
    // 11 chars: 105 → 52, within 5 px: unchanged.
    assert_eq!(set(&mut b, "0123456789a"), 47);
    // 12 chars: 115 → 57: moves.
    assert_eq!(set(&mut b, "0123456789ab"), 57);
}

#[test]
fn textbox_uses_mouse_position() {
    let v = scene();
    let f = build_warm(&v, &Settings::default());
    let last_quads = &f.quads[f.quads.len() - 8..];
    // "main.rs" at 18 px: 7 × 10.8 + 6 = 81; height 2 + 22 = 24.
    // Mouse (400, 300) → corner (400, 276); shadow at +3.
    assert_eq!(last_quads[0].pos, [403.0, 279.0]);
    assert_eq!(last_quads[4].pos, [400.0, 276.0]);
    assert_eq!(last_quads[6].pos, [481.0, 300.0]);
    assert_eq!(last_quads[4].colour, [0.1, 0.4, 0.2, 1.0]);
}

#[test]
fn ui_scale_scales_screen_space_constants() {
    let mut v = scene();
    v.viewport = VP * 2.0;
    v.ui_scale = 2.0;
    let f = build_warm(&v, &Settings::default());
    assert_eq!(find(&f, EDGE_SHADOW).offset, [4.0, 4.0]);
    // World-space offsets are not screen constants.
    assert_eq!(find(&f, FILE_SHADOW).offset, [2.0, 2.0]);
    // Edge half-width 5 device px.
    let e = find(&f, EDGES).first_quad as usize * 4;
    let w = (Vec2::from(f.quads[e].pos) - Vec2::from(f.quads[e + 1].pos)).length();
    assert!((w - 10.0).abs() < 1e-3);
    // Text box at the same logical spot: corner (400, 276) × 2 when the
    // mouse is at device (800, 600).
    v.hud.textbox.as_mut().unwrap().pos = Vec2::new(800.0, 600.0);
    let f = build_warm(&v, &Settings::default());
    let q = &f.quads[f.quads.len() - 4..];
    assert_eq!(q[0].pos, [800.0, 552.0]);
}

#[test]
fn splash_draws_band_and_three_lines() {
    let mut v = scene();
    v.hud.splash = 10.0;
    v.hud.textbox = None;
    v.hud.message = None;
    v.hud.show_key = false;
    v.hud.extensions.clear();
    let f = build_warm(&v, &Settings::default());
    assert_eq!(sigs(&f)[8..], [NAMES, TEXT, HUD_QUADS, TEXT]);
    let band = find(&f, HUD_QUADS);
    let q = &f.quads[band.first_quad as usize * 4..][..4];
    // corner.y = 360 - 40; height 100.
    assert_eq!(q[0].pos, [0.0, 320.0]);
    assert_eq!(q[2].pos, [1280.0, 420.0]);
    assert_eq!(q[0].colour, [0.0, 0.5, 1.0, 10.0f32 * 0.015]);
}

#[test]
fn zero_alpha_and_orphan_labels_are_skipped() {
    let mut v = scene();
    v.labels[2].colour.w = 0.0;
    v.labels[0].anchor = LabelAnchor::Dir { dir: 0 }; // root has no edge
    let f = build_warm(&v, &Settings::default());
    assert_eq!(find(&f, NAMES).quad_count, 4 + 5);
}
