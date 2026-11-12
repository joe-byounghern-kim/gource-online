// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! The per-frame scene snapshot handed from the simulation to the renderer.
//!
//! [`SceneView`] is the contract between `gource-core` (which decides what is
//! visible, where, and in which colour) and `gource-render` (which turns it
//! into vertex batches exactly the way `Gource::draw` does). It is plain data:
//! the simulation fills it once per frame with
//! `Simulation::extract_view`, reusing its allocations, so steady-state frames
//! allocate nothing.
//!
//! Positions are in world units on the z = 0 plane unless a field says
//! otherwise. Every list is in Gource's draw order, so rendering it front to
//! back reproduces Gource's overdraw.

use std::ops::Range;

use glam::{Mat4, Vec2, Vec3, Vec4};

/// Byte range into [`SceneView::text`].
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct TextRange {
    pub start: u32,
    pub end: u32,
}

impl TextRange {
    pub fn is_empty(self) -> bool {
        self.start >= self.end
    }
}

/// The camera as `ZoomCamera` holds it; matrices are derived exactly as
/// `display.mode3D(fov, znear, zfar)` + `gluLookAt` do.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct CameraView {
    pub pos: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub fov_y_degrees: f32,
    pub z_near: f32,
    pub z_far: f32,
}

impl Default for CameraView {
    fn default() -> Self {
        CameraView {
            pos: Vec3::new(0.0, 0.0, -300.0),
            target: Vec3::ZERO,
            up: Vec3::new(0.0, -1.0, 0.0),
            fov_y_degrees: 90.0,
            z_near: 0.1,
            z_far: 10001.0,
        }
    }
}

impl CameraView {
    /// `gluLookAt(pos, target, up)`, following the GLU reference formula.
    pub fn view_matrix(&self) -> Mat4 {
        let f = (self.target - self.pos).normalize();
        let up = self.up.normalize();
        let s = f.cross(up).normalize();
        let u = s.cross(f);
        let rotation = Mat4::from_cols(
            Vec4::new(s.x, u.x, -f.x, 0.0),
            Vec4::new(s.y, u.y, -f.y, 0.0),
            Vec4::new(s.z, u.z, -f.z, 0.0),
            Vec4::W,
        );
        rotation * Mat4::from_translation(-self.pos)
    }

    /// `gluPerspective(fov, aspect, znear, zfar)`, following the GLU reference formula.
    pub fn projection_matrix(&self, aspect: f32) -> Mat4 {
        let f = 1.0 / libm::tanf(self.fov_y_degrees.to_radians() * 0.5);
        let (n, far) = (self.z_near, self.z_far);
        Mat4::from_cols(
            Vec4::new(f / aspect, 0.0, 0.0, 0.0),
            Vec4::new(0.0, f, 0.0, 0.0),
            Vec4::new(0.0, 0.0, (far + n) / (n - far), -1.0),
            Vec4::new(0.0, 0.0, 2.0 * far * n / (n - far), 0.0),
        )
    }

    /// Projection × view for a viewport of `viewport` pixels.
    pub fn world_to_clip(&self, viewport: Vec2) -> Mat4 {
        let aspect = if viewport.y > 0.0 {
            viewport.x / viewport.y
        } else {
            1.0
        };
        self.projection_matrix(aspect) * self.view_matrix()
    }

    /// `gluProject` followed by Gource's `y = viewport.h - y` flip: world
    /// point to top-left-origin screen pixels.
    pub fn project(&self, viewport: Vec2, world: Vec3) -> Vec2 {
        project_with(&self.world_to_clip(viewport), viewport, world)
    }
}

/// [`CameraView::project`] with a precomputed `world_to_clip` matrix.
pub fn project_with(world_to_clip: &Mat4, viewport: Vec2, world: Vec3) -> Vec2 {
    let clip = *world_to_clip * world.extend(1.0);
    let w = if clip.w != 0.0 { clip.w } else { 1.0 };
    let ndc = clip.truncate() / w;
    let x = (ndc.x * 0.5 + 0.5) * viewport.x;
    let y = (ndc.y * 0.5 + 0.5) * viewport.y;
    Vec2::new(x, viewport.y - y)
}

/// One directory node (`RDirNode`).
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct DirView {
    /// Index of the parent in [`SceneView::dirs`]; `None` for the root.
    pub parent: Option<u32>,
    pub depth: u32,
    pub pos: Vec2,
    /// Spline control point (`spos`) for the edge to the parent.
    pub spos: Vec2,
    /// `dir_radius`; the node's quad-tree bounds are `pos ± radius`.
    pub radius: f32,
    /// `getColour()`.
    pub colour: Vec4,
    /// Result of `checkFrustum` against the frame's camera.
    pub in_frustum: bool,
    /// `isVisible()`.
    pub visible: bool,
    /// Draw the edge to the parent. Mirrors `updateEdgeVBO`: the node has a
    /// parent, `!hide_root || parent has a parent`, and every ancestor below
    /// the root is visible; false when `hide_tree`.
    pub draw_edge: bool,
    /// Draw this node's bloom: `in_frustum && visible && !hide_bloom`.
    pub draw_bloom: bool,
}

/// One drawn file (`RFile`): only non-hidden files of in-frustum dirs, in
/// `updateFilesVBO` order. Empty when `hide_files`.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct FileView {
    /// `getAbsolutePos()`: centre of the file disc.
    pub pos: Vec2,
    /// `dims` (width, height) of the graphic quad.
    pub size: Vec2,
    /// `getColour()`.
    pub colour: Vec3,
    /// `getAlpha()`.
    pub alpha: f32,
}

/// Texture used for a user quad.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum UserTexture {
    /// `data/user.png`.
    #[default]
    Default,
}

/// One user (`RUser`) in `users` map order (sorted by name). Empty when
/// `hide_users`.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct UserView {
    pub pos: Vec2,
    /// `dims`, already scaled for `fixed_user_size` as `updateVBOs` does.
    pub size: Vec2,
    pub colour: Vec3,
    pub alpha: f32,
    pub texture: UserTexture,
    /// Gear drawn with the avatar (AI decorations); the default draws
    /// nothing.
    pub decor: UserDecor,
}

/// A decoration a contributor can wear (`style` properties of the same
/// names). The discriminant is the bit in [`UserDecor::mask`] and the index
/// into [`UserDecor::tint`].
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum DecoKind {
    /// Behind the avatar, from the shoulders down.
    Cape = 0,
    /// Behind the avatar, above the head.
    Halo = 1,
    /// In front, over the eyes.
    Shades = 2,
    /// In front, on the head.
    Hat = 3,
    /// In front, lower right of the body; shape from [`UserDecor::badge`].
    Badge = 4,
    /// Additive glow around the figure, drawn in the bloom stream.
    Glow = 5,
}

impl DecoKind {
    pub const COUNT: usize = 6;

    pub const ALL: [DecoKind; DecoKind::COUNT] = [
        DecoKind::Cape,
        DecoKind::Halo,
        DecoKind::Shades,
        DecoKind::Hat,
        DecoKind::Badge,
        DecoKind::Glow,
    ];

    pub const fn bit(self) -> u8 {
        1 << self as u8
    }

    pub const fn index(self) -> usize {
        self as usize
    }

    /// The style property name.
    pub const fn name(self) -> &'static str {
        match self {
            DecoKind::Cape => "cape",
            DecoKind::Halo => "halo",
            DecoKind::Shades => "shades",
            DecoKind::Hat => "hat",
            DecoKind::Badge => "badge",
            DecoKind::Glow => "glow",
        }
    }
}

/// Decorations of one user (AI-51). Every input the renderer needs is here,
/// so frame building needs no clock and stays deterministic.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct UserDecor {
    /// [`DecoKind::bit`] per enabled decoration; 0 = none.
    pub mask: u8,
    /// Bits whose alpha is multiplied by `envelope` (positive polarity).
    pub dynamic: u8,
    /// Bits whose alpha is multiplied by `1 - envelope` (inverse polarity;
    /// reserved, always 0 before the inverse-polarity milestone).
    pub inverse: u8,
    /// [`crate::agents::BadgeShape`] index.
    pub badge: u8,
    /// RGB per [`DecoKind`].
    pub tint: [[u8; 3]; DecoKind::COUNT],
    /// The assist envelope (`attr_env`), 0..=1.
    pub envelope: f32,
    /// Decoration quad scale about the avatar centre (1 = default).
    pub scale: f32,
    /// The pawn's velocity at extraction, world units per second.
    pub vel: Vec2,
    /// Cape flutter phase in `[0, 1)`, advanced by simulation time.
    pub phase: f32,
    /// 1, or 0 with `decor-motion=off` (no lean, lift or flutter).
    pub motion: f32,
}

impl UserDecor {
    pub fn has(&self, kind: DecoKind) -> bool {
        self.mask & kind.bit() != 0
    }

    /// Alpha factor of `kind` from its polarity: `envelope`, `1 -
    /// envelope` or 1.
    pub fn fade(&self, kind: DecoKind) -> f32 {
        if self.dynamic & kind.bit() != 0 {
            self.envelope
        } else if self.inverse & kind.bit() != 0 {
            1.0 - self.envelope
        } else {
            1.0
        }
    }
}

/// One unfinished active action beam (`RAction::drawToVBO`), in user order
/// then `activeActions` order. Empty when `hide_users`.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct ActionView {
    /// `source->getPos()`.
    pub src: Vec2,
    /// `target->getAbsolutePos()`.
    pub dest: Vec2,
    /// `target->getSize()`.
    pub target_size: f32,
    pub colour: Vec3,
    pub progress: f32,
}

/// Which `FXFont` a piece of text is drawn with; the renderer maps roles to
/// pixel sizes from [`crate::settings::Settings`] (then × `ui_scale`).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum FontRole {
    /// `fontdirname`: `scaled_dirname_font_size`.
    DirName,
    /// `file_font`: `scaled_filename_font_size`.
    FileName,
    /// `file_selected_font`: 18.
    FileSelected,
    /// User font: `scaled_user_font_size`.
    UserName,
    /// Selected user font (`RUser::updateFont`): 18.
    UserSelected,
    /// `fontmedium`: `scaled_font_size` (date, title).
    Medium,
    /// `font`: 14 × `font_scale` (messages, debug text).
    Small,
    /// `fontlarge`: 42 × `font_scale` (splash).
    Large,
    /// Text box font: 18 × `font_scale`.
    TextBox,
    /// File key font: `scaled_font_size`.
    Key,
}

/// Where a label is anchored.
#[derive(Copy, Clone, Debug, PartialEq)]
pub enum LabelAnchor {
    /// Directory name at the spline label position of the edge from
    /// `dirs[dir]` to its parent (`SplineEdge::getLabelPos`, screen space,
    /// computed by the renderer from the projected endpoints, `spos` and
    /// `dir_name_position`).
    Dir { dir: u32 },
    /// World point projected to screen; text drawn with its top-left there
    /// (file names: absolute position + (5.5, -1) or (5.5, -2) if selected).
    WorldTopLeft { pos: Vec2 },
    /// World point projected to screen; text centred horizontally and drawn
    /// so its bottom sits on the point (user names: `pos.y - dims.y / 2`,
    /// then `x -= namewidth / 2`, `y -= font.getMaxHeight()`).
    WorldCentredAbove { pos: Vec2 },
}

/// One name label, in Gource's text draw order: directory and file names in
/// tree order, then user names, then the selected user/file names on top.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LabelView {
    pub anchor: LabelAnchor,
    pub text: TextRange,
    pub font: FontRole,
    /// RGB and alpha after all of Gource's fade rules.
    pub colour: Vec4,
}

/// Extension counts for the file key (`FileKey::inc` / `dec`), in extension
/// order. Only extensions with files on screen are listed; an extension
/// that is not listed has count 0. The renderer owns the key's sorting,
/// sliding and fading.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ExtensionCount {
    pub ext: TextRange,
    pub colour: Vec3,
    pub count: u32,
}

/// One row of the agent key (AI-59): an agent seen so far and the number
/// of played commits that credit it.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct AgentKeyRow {
    /// Display name.
    pub name: TextRange,
    pub colour: Vec3,
    pub count: u32,
}

/// Hover tooltip (`TextBox`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextBoxView {
    pub lines: Vec<TextRange>,
    pub colour: Vec3,
    /// Mouse position in screen pixels.
    pub pos: Vec2,
}

/// Screen-space overlays.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HudView {
    /// Formatted current date (`displaydate`), drawn top centre.
    pub date: Option<TextRange>,
    /// `--title`, drawn bottom left.
    pub title: Option<TextRange>,
    /// Status message (`setMessage`) and its remaining seconds.
    pub message: Option<(TextRange, f32)>,
    /// Whether the file key is shown (`show_key`).
    pub show_key: bool,
    pub extensions: Vec<ExtensionCount>,
    pub textbox: Option<TextBoxView>,
    /// Remaining splash seconds (0 = none).
    pub splash: f32,
    /// Whether the agent key is shown (`agent-key`).
    pub show_agent_key: bool,
    /// Agent key rows, count descending then id; at most
    /// [`AGENT_KEY_ROWS`], filled only while the key is shown.
    pub agents: Vec<AgentKeyRow>,
    /// Agents seen beyond the listed rows (drawn as "N more").
    pub agents_more: u32,
    /// The line drawn under the date while `ai-filter` is not `all`.
    pub filter_note: Option<TextRange>,
    /// Compact ASCII PR state/verdict key, drawn by the existing text batch.
    pub pr_key: Vec<TextRange>,
    /// Repository-wide flow summary, never per-person metrics.
    pub pr_flow: Option<TextRange>,
    /// Immutable lifecycle, review and file coverage shown separately.
    pub pr_coverage: Vec<TextRange>,
}

/// Most rows of the agent key.
pub const AGENT_KEY_ROWS: usize = 8;

/// Everything the renderer needs for one frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SceneView {
    /// Drawable size in device pixels.
    pub viewport: Vec2,
    /// Device pixels per CSS pixel. Screen-space constants (edge width,
    /// shadow offsets, font sizes, HUD margins) are multiplied by it.
    pub ui_scale: f32,
    pub camera: CameraView,
    pub background: Vec3,
    /// Directory nodes in depth-first pre-order (root first, children in
    /// `RDirNode::children` order).
    pub dirs: Vec<DirView>,
    pub files: Vec<FileView>,
    pub users: Vec<UserView>,
    pub actions: Vec<ActionView>,
    pub labels: Vec<LabelView>,
    pub hud: HudView,
    /// Backing storage for every [`TextRange`] in this view.
    pub text: String,
}

impl SceneView {
    /// Empty all lists, keeping their capacity.
    pub fn clear(&mut self) {
        self.dirs.clear();
        self.files.clear();
        self.users.clear();
        self.actions.clear();
        self.labels.clear();
        self.hud.date = None;
        self.hud.title = None;
        self.hud.message = None;
        self.hud.extensions.clear();
        self.hud.textbox = None;
        self.hud.splash = 0.0;
        self.hud.show_agent_key = false;
        self.hud.agents.clear();
        self.hud.agents_more = 0;
        self.hud.filter_note = None;
        self.hud.pr_key.clear();
        self.hud.pr_flow = None;
        self.hud.pr_coverage.clear();
        self.text.clear();
    }

    /// Append `s` to the text store.
    pub fn push_text(&mut self, s: &str) -> TextRange {
        let start = self.text.len() as u32;
        self.text.push_str(s);
        TextRange {
            start,
            end: self.text.len() as u32,
        }
    }

    pub fn str(&self, range: TextRange) -> &str {
        &self.text[range.start as usize..range.end as usize]
    }

    pub fn range(range: TextRange) -> Range<usize> {
        range.start as usize..range.end as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_target_to_viewport_centre() {
        let cam = CameraView::default();
        let vp = Vec2::new(1280.0, 720.0);
        let p = cam.project(vp, Vec3::ZERO);
        assert!(
            (p.x - 640.0).abs() < 1e-3 && (p.y - 360.0).abs() < 1e-3,
            "{p:?}"
        );
    }

    #[test]
    fn world_axes_map_like_screen_axes() {
        // Gource's camera looks down +z with up = -y, so world +x is screen
        // right and world +y is screen down.
        let cam = CameraView::default();
        let vp = Vec2::new(1280.0, 720.0);
        let c = cam.project(vp, Vec3::ZERO);
        let px = cam.project(vp, Vec3::new(10.0, 0.0, 0.0));
        let py = cam.project(vp, Vec3::new(0.0, 10.0, 0.0));
        assert!(px.x > c.x && (px.y - c.y).abs() < 1e-3);
        assert!(py.y > c.y && (py.x - c.x).abs() < 1e-3);
        // fov 90: at distance 300 the half-height of the view is 300 units.
        let edge = cam.project(vp, Vec3::new(0.0, 300.0, 0.0));
        assert!((edge.y - 720.0).abs() < 1e-2, "{edge:?}");
    }

    #[test]
    fn text_store_round_trips() {
        let mut v = SceneView::default();
        let a = v.push_text("src");
        let b = v.push_text("main.rs");
        assert_eq!(v.str(a), "src");
        assert_eq!(v.str(b), "main.rs");
        v.clear();
        assert!(v.text.is_empty());
    }
}
