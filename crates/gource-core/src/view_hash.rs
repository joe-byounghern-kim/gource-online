// SPDX-License-Identifier: GPL-3.0-or-later

//! A deterministic fingerprint of a [`SceneView`].
//!
//! [`hash_view`] folds everything a frame is drawn from into one 64-bit
//! FNV-1a hash, so two runs (or a native and a wasm32 run, or the Rust port
//! and a dump of the C++ reference) can be compared by one number per frame.
//!
//! # Encoding (version 1)
//!
//! FNV-1a (64 bit: offset basis `0xcbf29ce484222325`, prime `0x100000001b3`)
//! over this byte stream, all integers little-endian:
//!
//! - `f32` is its IEEE-754 bit pattern as a `u32`, so `-0.0`, `0.0` and NaN
//!   payloads are all distinct and nothing is rounded;
//! - `bool` is one byte, 0 or 1; an enum is one byte holding its variant
//!   number in declaration order (plus the variant's fields);
//! - `Option<T>` is a byte 0, or a byte 1 followed by the `T`;
//! - a string is its byte length as a `u64`, then its UTF-8 bytes; text kept
//!   in the view's text store is hashed as the string its range names, not as
//!   the range;
//! - a vector is its components in order (`x, y[, z[, w]]`);
//! - a list is a four-byte section tag, its length as a `u64`, then its
//!   elements, each with every field in declaration order.
//!
//! The stream is: the tag `VIEW` and the encoding version (`u32`), the
//! viewport, `ui_scale`, the camera (`pos`, `target`, `up`, `fov_y_degrees`,
//! `z_near`, `z_far`), `background`, then the lists `DIRS`, `FILE`, `USER`,
//! `ACTN`, `LABL` and the HUD (`date`, `title`, `message`, `show_key`, the
//! list `EXTS`, the text box, `splash`).
//!
//! Every field is read by name rather than destructured, so a field added to
//! the view later still compiles; it is simply not part of the hash until it
//! is added here (and [`ENCODING_VERSION`] is bumped).
//!
//! AI decorations extend version 1 without changing it ([`hash_ai`]): a
//! view without decorations, agent key or AI filter line hashes exactly as
//! before; otherwise these sections follow the HUD, each only when present:
//!
//! - `DECO`: the number of users with a non-zero `decor.mask`, then for each
//!   of them its index in `USER` (`u32`), `mask`, `dynamic`, `inverse` and
//!   `badge` (one byte each), the six RGB tints (18 bytes), `envelope`,
//!   `scale`, `vel`, `phase` and `motion`;
//! - `AKEY` (agent key shown): the rows (name, colour, count) and
//!   `agents_more` (`u32`);
//! - `FNOT`: the filter line.
//!
//! PR presentation similarly adds `PKEY`, `PCOV` and `PFLO` only when
//! populated. Views without rich PR presentation retain their original bytes.

use glam::{Vec2, Vec3, Vec4};

use crate::view::{LabelAnchor, SceneView, TextRange};

/// Bumped whenever the byte stream changes, so old and new hashes are never
/// compared by accident.
pub const ENCODING_VERSION: u32 = 1;

const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// Incremental 64-bit FNV-1a.
#[derive(Copy, Clone, Debug)]
pub struct Fnv1a(u64);

impl Default for Fnv1a {
    fn default() -> Self {
        Fnv1a(OFFSET_BASIS)
    }
}

impl Fnv1a {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ u64::from(b)).wrapping_mul(PRIME);
        }
    }

    pub fn finish(&self) -> u64 {
        self.0
    }

    fn u8(&mut self, v: u8) {
        self.write(&[v]);
    }

    fn bool(&mut self, v: bool) {
        self.u8(u8::from(v));
    }

    fn u32(&mut self, v: u32) {
        self.write(&v.to_le_bytes());
    }

    fn u64(&mut self, v: u64) {
        self.write(&v.to_le_bytes());
    }

    fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }

    fn str(&mut self, s: &str) {
        self.u64(s.len() as u64);
        self.write(s.as_bytes());
    }

    fn vec2(&mut self, v: Vec2) {
        v.to_array().into_iter().for_each(|c| self.f32(c));
    }

    fn vec3(&mut self, v: Vec3) {
        v.to_array().into_iter().for_each(|c| self.f32(c));
    }

    fn vec4(&mut self, v: Vec4) {
        v.to_array().into_iter().for_each(|c| self.f32(c));
    }

    /// Start of a list: its tag and length.
    fn list(&mut self, tag: &[u8; 4], len: usize) {
        self.write(tag);
        self.u64(len as u64);
    }
}

/// The version-1 fingerprint of `view`; see the module documentation.
pub fn hash_view(view: &SceneView) -> u64 {
    let mut h = Fnv1a::new();
    let text = |range: TextRange| view.str(range);

    h.write(b"VIEW");
    h.u32(ENCODING_VERSION);
    h.vec2(view.viewport);
    h.f32(view.ui_scale);
    let camera = &view.camera;
    h.vec3(camera.pos);
    h.vec3(camera.target);
    h.vec3(camera.up);
    h.f32(camera.fov_y_degrees);
    h.f32(camera.z_near);
    h.f32(camera.z_far);
    h.vec3(view.background);

    h.list(b"DIRS", view.dirs.len());
    for dir in &view.dirs {
        match dir.parent {
            Some(parent) => {
                h.u8(1);
                h.u32(parent);
            }
            None => h.u8(0),
        }
        h.u32(dir.depth);
        h.vec2(dir.pos);
        h.vec2(dir.spos);
        h.f32(dir.radius);
        h.vec4(dir.colour);
        h.bool(dir.in_frustum);
        h.bool(dir.visible);
        h.bool(dir.draw_edge);
        h.bool(dir.draw_bloom);
    }

    h.list(b"FILE", view.files.len());
    for file in &view.files {
        h.vec2(file.pos);
        h.vec2(file.size);
        h.vec3(file.colour);
        h.f32(file.alpha);
    }

    h.list(b"USER", view.users.len());
    for user in &view.users {
        h.vec2(user.pos);
        h.vec2(user.size);
        h.vec3(user.colour);
        h.f32(user.alpha);
        h.u8(user.texture as u8);
    }

    h.list(b"ACTN", view.actions.len());
    for action in &view.actions {
        h.vec2(action.src);
        h.vec2(action.dest);
        h.f32(action.target_size);
        h.vec3(action.colour);
        h.f32(action.progress);
    }

    h.list(b"LABL", view.labels.len());
    for label in &view.labels {
        match label.anchor {
            LabelAnchor::Dir { dir } => {
                h.u8(0);
                h.u32(dir);
            }
            LabelAnchor::WorldTopLeft { pos } => {
                h.u8(1);
                h.vec2(pos);
            }
            LabelAnchor::WorldCentredAbove { pos } => {
                h.u8(2);
                h.vec2(pos);
            }
        }
        h.str(text(label.text));
        h.u8(label.font as u8);
        h.vec4(label.colour);
    }

    let hud = &view.hud;
    for range in [hud.date, hud.title] {
        match range {
            Some(range) => {
                h.u8(1);
                h.str(text(range));
            }
            None => h.u8(0),
        }
    }
    match hud.message {
        Some((range, seconds)) => {
            h.u8(1);
            h.str(text(range));
            h.f32(seconds);
        }
        None => h.u8(0),
    }
    h.bool(hud.show_key);
    h.list(b"EXTS", hud.extensions.len());
    for extension in &hud.extensions {
        h.str(text(extension.ext));
        h.vec3(extension.colour);
        h.u32(extension.count);
    }
    match &hud.textbox {
        Some(textbox) => {
            h.u8(1);
            h.u64(textbox.lines.len() as u64);
            for &line in &textbox.lines {
                h.str(text(line));
            }
            h.vec3(textbox.colour);
            h.vec2(textbox.pos);
        }
        None => h.u8(0),
    }
    h.f32(hud.splash);

    hash_ai(&mut h, view);
    hash_pr(&mut h, view);

    h.finish()
}

/// The AI sections of [`hash_view`] (see the module documentation); writes
/// nothing for a view without decorations, agent key or filter line.
fn hash_ai(h: &mut Fnv1a, view: &SceneView) {
    let decorated = view.users.iter().filter(|u| u.decor.mask != 0).count();
    if decorated > 0 {
        h.list(b"DECO", decorated);
        for (i, user) in view.users.iter().enumerate() {
            let d = &user.decor;
            if d.mask == 0 {
                continue;
            }
            h.u32(i as u32);
            h.write(&[d.mask, d.dynamic, d.inverse, d.badge]);
            for tint in &d.tint {
                h.write(tint);
            }
            h.f32(d.envelope);
            h.f32(d.scale);
            h.vec2(d.vel);
            h.f32(d.phase);
            h.f32(d.motion);
        }
    }
    let hud = &view.hud;
    if hud.show_agent_key {
        h.list(b"AKEY", hud.agents.len());
        for row in &hud.agents {
            h.str(view.str(row.name));
            h.vec3(row.colour);
            h.u32(row.count);
        }
        h.u32(hud.agents_more);
    }
    if let Some(note) = hud.filter_note {
        h.write(b"FNOT");
        h.str(view.str(note));
    }
}

fn hash_pr(h: &mut Fnv1a, view: &SceneView) {
    for (tag, lines) in [
        (b"PKEY", &view.hud.pr_key),
        (b"PCOV", &view.hud.pr_coverage),
    ] {
        if !lines.is_empty() {
            h.list(tag, lines.len());
            for &line in lines {
                h.str(view.str(line));
            }
        }
    }
    if let Some(flow) = view.hud.pr_flow {
        h.write(b"PFLO");
        h.str(view.str(flow));
    }
}

#[cfg(test)]
mod tests {
    use crate::view::{
        ActionView, DirView, ExtensionCount, FileView, FontRole, LabelView, TextBoxView, UserView,
    };

    use super::*;

    fn fnv(bytes: &[u8]) -> u64 {
        let mut h = Fnv1a::new();
        h.write(bytes);
        h.finish()
    }

    #[test]
    fn fnv1a_matches_the_reference_vectors() {
        // From the FNV test suite (isthe.com/chongo/src/fnv/test_fnv.c).
        assert_eq!(fnv(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(fnv(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn fnv1a_is_incremental() {
        let mut h = Fnv1a::new();
        h.write(b"foo");
        h.write(b"bar");
        assert_eq!(h.finish(), fnv(b"foobar"));
    }

    /// A view with something in every list and every optional.
    fn full_view() -> SceneView {
        let mut v = SceneView {
            viewport: Vec2::new(640.0, 480.0),
            ui_scale: 2.0,
            background: Vec3::new(0.1, 0.2, 0.3),
            ..SceneView::default()
        };
        v.dirs.push(DirView {
            parent: Some(0),
            depth: 1,
            pos: Vec2::new(1.0, 2.0),
            spos: Vec2::new(3.0, 4.0),
            radius: 5.0,
            colour: Vec4::new(0.5, 0.6, 0.7, 0.8),
            in_frustum: true,
            visible: true,
            draw_edge: true,
            draw_bloom: true,
        });
        v.files.push(FileView {
            pos: Vec2::new(1.0, 1.0),
            size: Vec2::new(8.0, 8.0),
            colour: Vec3::new(0.9, 0.8, 0.7),
            alpha: 0.5,
        });
        v.users.push(UserView {
            pos: Vec2::new(2.0, 2.0),
            size: Vec2::new(20.0, 20.0),
            colour: Vec3::new(0.4, 0.4, 0.4),
            alpha: 1.0,
            ..UserView::default()
        });
        v.actions.push(ActionView {
            src: Vec2::new(0.0, 1.0),
            dest: Vec2::new(2.0, 3.0),
            target_size: 4.0,
            colour: Vec3::new(0.0, 1.0, 0.0),
            progress: 0.25,
        });
        let name = v.push_text("main.rs");
        v.labels.push(LabelView {
            anchor: LabelAnchor::WorldTopLeft {
                pos: Vec2::new(9.0, 9.0),
            },
            text: name,
            font: FontRole::FileName,
            colour: Vec4::ONE,
        });
        v.hud.date = Some(v.push_text("Monday"));
        v.hud.title = Some(v.push_text("title"));
        let message = v.push_text("hello");
        v.hud.message = Some((message, 1.5));
        v.hud.show_key = true;
        let ext = v.push_text("rs");
        v.hud.extensions.push(ExtensionCount {
            ext,
            colour: Vec3::new(1.0, 0.5, 0.0),
            count: 3,
        });
        let line = v.push_text("line");
        v.hud.textbox = Some(TextBoxView {
            lines: vec![line],
            colour: Vec3::ONE,
            pos: Vec2::new(5.0, 6.0),
        });
        v.hud.splash = 2.0;
        v
    }

    /// Hash of `full_view()` after `change`.
    fn changed(change: impl FnOnce(&mut SceneView)) -> u64 {
        let mut v = full_view();
        change(&mut v);
        hash_view(&v)
    }

    #[test]
    fn equal_views_hash_equal() {
        assert_eq!(hash_view(&full_view()), hash_view(&full_view()));
        assert_eq!(
            hash_view(&SceneView::default()),
            hash_view(&SceneView::default())
        );
        assert_ne!(hash_view(&full_view()), hash_view(&SceneView::default()));
    }

    #[test]
    fn every_part_of_the_view_matters() {
        let base = hash_view(&full_view());
        let variants: Vec<(&str, u64)> = vec![
            ("viewport", changed(|v| v.viewport.x = 641.0)),
            ("ui_scale", changed(|v| v.ui_scale = 1.0)),
            ("camera", changed(|v| v.camera.pos.z = -301.0)),
            ("camera fov", changed(|v| v.camera.fov_y_degrees = 80.0)),
            ("background", changed(|v| v.background.z = 0.31)),
            ("dir parent", changed(|v| v.dirs[0].parent = None)),
            ("dir depth", changed(|v| v.dirs[0].depth = 2)),
            ("dir pos", changed(|v| v.dirs[0].pos.y = 2.5)),
            ("dir spos", changed(|v| v.dirs[0].spos.x = 3.5)),
            ("dir radius", changed(|v| v.dirs[0].radius = 5.5)),
            ("dir colour", changed(|v| v.dirs[0].colour.w = 0.9)),
            ("dir frustum", changed(|v| v.dirs[0].in_frustum = false)),
            ("dir visible", changed(|v| v.dirs[0].visible = false)),
            ("dir edge", changed(|v| v.dirs[0].draw_edge = false)),
            ("dir bloom", changed(|v| v.dirs[0].draw_bloom = false)),
            ("file pos", changed(|v| v.files[0].pos.x = 1.5)),
            ("file size", changed(|v| v.files[0].size.y = 9.0)),
            ("file colour", changed(|v| v.files[0].colour.x = 0.95)),
            ("file alpha", changed(|v| v.files[0].alpha = 0.25)),
            ("user pos", changed(|v| v.users[0].pos.x = 2.5)),
            ("user size", changed(|v| v.users[0].size.x = 21.0)),
            ("user colour", changed(|v| v.users[0].colour.y = 0.41)),
            ("user alpha", changed(|v| v.users[0].alpha = 0.5)),
            ("action src", changed(|v| v.actions[0].src.x = 0.5)),
            ("action dest", changed(|v| v.actions[0].dest.y = 3.5)),
            ("action size", changed(|v| v.actions[0].target_size = 5.0)),
            ("action colour", changed(|v| v.actions[0].colour.z = 0.1)),
            ("action progress", changed(|v| v.actions[0].progress = 0.5)),
            (
                "label anchor kind",
                changed(|v| v.labels[0].anchor = LabelAnchor::Dir { dir: 0 }),
            ),
            (
                "label anchor pos",
                changed(|v| {
                    v.labels[0].anchor = LabelAnchor::WorldTopLeft {
                        pos: Vec2::new(9.0, 9.5),
                    }
                }),
            ),
            (
                "label font",
                changed(|v| v.labels[0].font = FontRole::UserName),
            ),
            ("label colour", changed(|v| v.labels[0].colour.w = 0.5)),
            ("label list", changed(|v| v.labels.clear())),
            ("hud date", changed(|v| v.hud.date = None)),
            ("hud title", changed(|v| v.hud.title = None)),
            (
                "hud message seconds",
                changed(|v| v.hud.message = v.hud.message.map(|(r, _)| (r, 2.5))),
            ),
            ("hud message", changed(|v| v.hud.message = None)),
            ("hud key", changed(|v| v.hud.show_key = false)),
            (
                "hud extension count",
                changed(|v| v.hud.extensions[0].count = 4),
            ),
            (
                "hud extension colour",
                changed(|v| v.hud.extensions[0].colour.y = 0.4),
            ),
            ("hud textbox", changed(|v| v.hud.textbox = None)),
            (
                "hud textbox pos",
                changed(|v| v.hud.textbox.as_mut().unwrap().pos.x = 5.5),
            ),
            ("hud splash", changed(|v| v.hud.splash = 0.0)),
        ];
        for (name, hash) in &variants {
            assert_ne!(*hash, base, "{name} is not part of the hash");
        }
    }

    #[test]
    fn text_is_hashed_by_content_not_position() {
        // Same strings, different order in the backing store.
        let build = |first: &str, second: &str| {
            let mut v = SceneView::default();
            let a = v.push_text(first);
            let b = v.push_text(second);
            v.hud.date = Some(a);
            v.hud.title = Some(b);
            hash_view(&v)
        };
        assert_eq!(build("ab", "c"), build("ab", "c"));
        assert_ne!(
            build("ab", "c"),
            build("a", "bc"),
            "lengths are part of the hash"
        );
        assert_ne!(build("ab", "c"), build("c", "ab"));

        // Different offsets, same strings.
        let mut v = SceneView::default();
        v.push_text("padding");
        let date = v.push_text("ab");
        let title = v.push_text("c");
        v.hud.date = Some(date);
        v.hud.title = Some(title);
        assert_eq!(hash_view(&v), build("ab", "c"));
    }

    #[test]
    fn ai_sections_extend_version_1_only_when_present() {
        use crate::view::{AgentKeyRow, DecoKind, UserDecor};

        let base = hash_view(&full_view());
        // the default decor and an empty, hidden key leave the stream alone
        assert_eq!(changed(|v| v.users[0].decor = UserDecor::default()), base);
        assert_eq!(changed(|v| v.hud.agents_more = 3), base);

        let decor = UserDecor {
            mask: DecoKind::Cape.bit(),
            scale: 1.0,
            motion: 1.0,
            ..UserDecor::default()
        };
        let with = |f: fn(&mut UserDecor)| {
            changed(|v| {
                let mut d = decor;
                f(&mut d);
                v.users[0].decor = d;
            })
        };
        let decorated = with(|_| {});
        assert_ne!(decorated, base);
        let variants = [
            ("mask", with(|d| d.mask |= DecoKind::Hat.bit())),
            ("dynamic", with(|d| d.dynamic = 1)),
            ("inverse", with(|d| d.inverse = 1)),
            ("badge", with(|d| d.badge = 2)),
            ("tint", with(|d| d.tint[5][2] = 9)),
            ("envelope", with(|d| d.envelope = 0.5)),
            ("scale", with(|d| d.scale = 2.0)),
            ("vel", with(|d| d.vel.y = 1.0)),
            ("phase", with(|d| d.phase = 0.1)),
            ("motion", with(|d| d.motion = 0.0)),
        ];
        for (name, hash) in variants {
            assert_ne!(hash, decorated, "{name} is not part of the hash");
        }

        let key = changed(|v| v.hud.show_agent_key = true);
        assert_ne!(key, base);
        let row = changed(|v| {
            v.hud.show_agent_key = true;
            let name = v.push_text("Claude");
            v.hud.agents.push(AgentKeyRow {
                name,
                colour: Vec3::ONE,
                count: 4,
            });
        });
        assert_ne!(row, key);
        let note = changed(|v| v.hud.filter_note = Some(v.push_text("AI commits: only assisted")));
        assert_ne!(note, base);
    }

    #[test]
    fn float_bits_are_distinguished() {
        assert_ne!(
            changed(|v| v.files[0].alpha = 0.0),
            changed(|v| v.files[0].alpha = -0.0)
        );
        assert_ne!(
            changed(|v| v.files[0].alpha = f32::NAN),
            changed(|v| v.files[0].alpha = 0.0)
        );
    }
}
