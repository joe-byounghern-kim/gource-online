// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Synthetic scenes for the integration tests.

use gource_render::gource_core::glam::{Vec2, Vec3, Vec4};
use gource_render::gource_core::view::{
    ActionView, CameraView, DirView, ExtensionCount, FileView, FontRole, LabelAnchor, LabelView,
    SceneView, TextBoxView, UserView,
};

/// A tree of `dirs` directories (each node has up to 4 children) holding
/// `files` files, with `users` users, one action per user, names for every
/// dir, every 10th file and every user, and a full HUD.
pub fn scene(dirs: usize, files: usize, users: usize) -> SceneView {
    let mut v = SceneView {
        viewport: Vec2::new(1920.0, 1080.0),
        ui_scale: 1.0,
        background: Vec3::splat(0.1),
        camera: CameraView {
            pos: glam_vec3(0.0, 0.0, -3000.0),
            ..CameraView::default()
        },
        ..SceneView::default()
    };

    for i in 0..dirs {
        let parent = if i == 0 {
            None
        } else {
            Some(((i - 1) / 4) as u32)
        };
        let a = i as f32 * 2.399_963;
        let r = 30.0 * (i as f32).sqrt();
        let pos = Vec2::new(r * a.cos(), r * a.sin());
        let ppos = parent.map_or(Vec2::ZERO, |p| v.dirs[p as usize].pos);
        let spos = (pos + ppos) * 0.5 + Vec2::new(5.0, -3.0);
        v.dirs.push(DirView {
            parent,
            depth: 1,
            pos,
            spos,
            radius: 10.0 + (i % 7) as f32,
            colour: Vec4::new(0.5, 0.6, 0.7, 1.0),
            in_frustum: true,
            visible: true,
            draw_edge: parent.is_some(),
            draw_bloom: true,
        });
        let name = format!("dir{i}");
        let text = v.push_text(&name);
        v.labels.push(LabelView {
            anchor: LabelAnchor::Dir { dir: i as u32 },
            text,
            font: FontRole::DirName,
            colour: Vec4::new(1.0, 1.0, 1.0, 0.5),
        });
    }

    for i in 0..files {
        let d = &v.dirs[i % dirs];
        let pos = d.pos + Vec2::new((i % 5) as f32 * 8.0, (i % 3) as f32 * 8.0);
        v.files.push(FileView {
            pos,
            size: Vec2::splat(8.0),
            colour: Vec3::new(0.2, 0.8, 0.4),
            alpha: 1.0,
        });
        if i % 10 == 0 {
            let name = format!("file{i}.rs");
            let text = v.push_text(&name);
            v.labels.push(LabelView {
                anchor: LabelAnchor::WorldTopLeft {
                    pos: pos + Vec2::new(5.5, -1.0),
                },
                text,
                font: FontRole::FileName,
                colour: Vec4::new(1.0, 1.0, 1.0, 0.7),
            });
        }
    }

    for i in 0..users {
        let pos = Vec2::new(i as f32 * 25.0 - 200.0, 300.0);
        v.users.push(UserView {
            pos,
            size: Vec2::splat(20.0),
            colour: Vec3::new(0.9, 0.6, 0.6),
            alpha: 1.0,
            ..UserView::default()
        });
        v.actions.push(ActionView {
            src: pos,
            dest: v.files[i % files.max(1)].pos,
            target_size: 8.0,
            colour: Vec3::new(1.0, 0.5, 0.0),
            progress: 0.5,
        });
        let name = format!("user{i}");
        let text = v.push_text(&name);
        v.labels.push(LabelView {
            anchor: LabelAnchor::WorldCentredAbove {
                pos: pos - Vec2::new(0.0, 10.0),
            },
            text,
            font: if i == 0 {
                FontRole::UserSelected
            } else {
                FontRole::UserName
            },
            colour: Vec4::ONE,
        });
    }

    v.hud.date = Some(v.push_text("Monday, 01 January, 2024 12:00:00"));
    v.hud.title = Some(v.push_text("A title"));
    let msg = v.push_text("Message");
    v.hud.message = Some((msg, 3.0));
    v.hud.show_key = true;
    for ext in ["rs", "md", "toml", "c", "h", "a_very_long_extension_name"] {
        let r = v.push_text(ext);
        v.hud.extensions.push(ExtensionCount {
            ext: r,
            colour: Vec3::new(0.3, 0.6, 0.9),
            count: ext.len() as u32,
        });
    }
    let l1 = v.push_text("main.rs");
    let l2 = v.push_text("src/core/");
    v.hud.textbox = Some(TextBoxView {
        lines: vec![l1, l2],
        colour: Vec3::new(0.2, 0.8, 0.4),
        pos: Vec2::new(500.0, 400.0),
    });
    v
}

fn glam_vec3(x: f32, y: f32, z: f32) -> gource_render::gource_core::glam::Vec3 {
    gource_render::gource_core::glam::Vec3::new(x, y, z)
}
