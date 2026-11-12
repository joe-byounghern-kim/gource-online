// SPDX-License-Identifier: GPL-3.0-or-later

//! AI decorations (AI-52 to AI-58, AI-61): sprite quads from the decoration
//! atlas around each avatar, and the glow quads of the bloom stream.
//!
//! The quads go into `frame.quads` in two blocks around the avatar block,
//! each in `view.users` order: the back block (cape, then halo, per user)
//! before the avatars and the front block (shades, hat, badge) after them,
//! so the avatar range stays contiguous and each block is one draw command
//! (two at most for the whole scene). Glow quads are appended to the bloom
//! stream after the directory blooms and drawn by the same command.
//!
//! Positions are the anchor rectangles below, as `(u0, v0, u1, v1)`
//! fractions of the avatar quad (`u` right, `v` down; the avatar art is
//! 384x512 with the head centre near (0.49, 0.31)), scaled by
//! `UserDecor::scale` about the avatar centre. Every input comes from the
//! view (velocity, phase, envelope), so frames are deterministic.

use glam::{Mat4, Vec2, Vec4};

use gource_core::view::{DecoKind, SceneView, UserDecor, UserView, project_with};

use crate::deco_atlas::DecoCell;
use crate::emit::{bloom_quad, quad4};
use crate::{BloomVertex, QuadVertex};

/// `TEXTURE_MAX_LEVEL` of the decoration atlas: at mip 2 the 4 px gutter
/// of a 64 px cell is still one texel wide, so no neighbour bleeds in.
pub const DECO_TEXTURE_MAX_LEVEL: i32 = 2;

/// Anchor of the cape: top edge at the shoulders, wider than the torso so
/// both shoulders and flanks show beside the avatar, hem just below its base.
pub(crate) const CAPE_RECT: [f32; 4] = [-0.16, 0.43, 1.16, 1.00];
/// Anchor of the halo, above and behind the head.
pub(crate) const HALO_RECT: [f32; 4] = [0.24, -0.04, 0.74, 0.10];
/// Anchor of the shades, over the eyes.
pub(crate) const SHADES_RECT: [f32; 4] = [0.27, 0.24, 0.71, 0.36];
/// Anchor of the hat, on the head.
pub(crate) const HAT_RECT: [f32; 4] = [0.20, -0.14, 0.78, 0.17];
/// Anchor of the badge, lower right of the body.
pub(crate) const BADGE_RECT: [f32; 4] = [0.70, 0.68, 0.92, 0.84];

/// Glow radius in avatar widths, centred on the avatar.
pub(crate) const GLOW_RADIUS: f32 = 3.0;
/// Glow colour strength (the bloom shader clamps colour to 1).
pub(crate) const GLOW_STRENGTH: f32 = 0.85;

/// Speed (world units per second) at which the cape leans and lifts fully.
const LEAN_SPEED: f32 = 250.0;
/// Full sideways lean of the cape's bottom edge, in avatar widths.
const LEAN: f32 = 0.6;
/// Full lift of the cape's bottom edge, in avatar heights.
const LIFT: f32 = 0.2;
/// Flutter amplitude of the cape's bottom edge, in avatar widths.
const FLUTTER: f32 = 0.04;
/// Resting billow: the hem corners spread apart by up to this much (avatar
/// widths each) and back once per flutter cycle, fading out as the cape
/// leans, so a standing cape breathes instead of hanging still.
const BILLOW: f32 = 0.03;
/// Height of a fully faded cape, as a fraction of its height: a cape fading
/// out also folds up toward the shoulders (and back down when fading in).
const FADE_FOLD: f32 = 0.35;

/// Smallest badge height in device pixels per `ui_scale` unit.
pub(crate) const BADGE_MIN_PX: f32 = 8.0;
/// Largest badge height the minimum may reach, in avatar heights.
const BADGE_MAX_FRACTION: f32 = 0.4;

/// What the passes need from the frame.
pub(crate) struct DecoCtx<'a> {
    pub view: &'a SceneView,
    pub world_to_clip: Mat4,
    /// Device pixels per logical pixel.
    pub ui_scale: f32,
}

/// The avatar quad of `u`: top-left corner and size.
fn avatar(u: &UserView) -> (Vec2, Vec2) {
    (u.pos - u.size * 0.5, u.size)
}

fn colour(d: &UserDecor, kind: DecoKind, alpha: f32) -> Vec4 {
    let [r, g, b] = d.tint[kind.index()];
    Vec4::new(
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        alpha * d.fade(kind),
    )
}

/// Corners (top-left, top-right, bottom-right, bottom-left) of anchor
/// `rect` on `u`'s avatar, scaled about the avatar centre.
fn corners(u: &UserView, rect: [f32; 4]) -> [Vec2; 4] {
    let (origin, size) = avatar(u);
    let at = |x: f32, y: f32| origin + Vec2::new(x, y) * size;
    [
        at(rect[0], rect[1]),
        at(rect[2], rect[1]),
        at(rect[2], rect[3]),
        at(rect[0], rect[3]),
    ]
}

fn scaled(u: &UserView, mut c: [Vec2; 4]) -> [Vec2; 4] {
    let s = u.decor.scale;
    if s != 1.0 {
        for p in &mut c {
            *p = u.pos + (*p - u.pos) * s;
        }
    }
    c
}

/// Cape corners with lean, lift, flutter and billow (AI-56): the top edge
/// stays at the shoulders, the bottom edge streams against the direction of
/// motion and, at rest, sways and spreads gently. While the cape fades
/// (`UserDecor::fade`), the bottom edge also folds up toward the shoulders.
pub(crate) fn cape_corners(u: &UserView) -> [Vec2; 4] {
    let d = &u.decor;
    let [tl, tr, br, bl] = corners(u, CAPE_RECT);
    let (w, h) = (u.size.x, u.size.y);
    let lean = (-d.vel.x / LEAN_SPEED).clamp(-1.0, 1.0) * LEAN * w;
    let speed = (d.vel.length() / LEAN_SPEED).min(1.0);
    let lift = speed * LIFT * h;
    let angle = 2.0 * core::f32::consts::PI * d.phase;
    let flutter = FLUTTER * w * libm::sinf(angle);
    let billow = BILLOW * w * (1.0 - speed) * 0.5 * (1.0 - libm::cosf(angle)) * d.motion;
    let shift = Vec2::new((lean + flutter) * d.motion, -lift * d.motion);
    let spread = Vec2::new(billow, 0.0);
    let (mut br, mut bl) = (br + shift + spread, bl + shift - spread);
    let fold = FADE_FOLD + (1.0 - FADE_FOLD) * d.fade(DecoKind::Cape).clamp(0.0, 1.0);
    if fold < 1.0 {
        br = tr + (br - tr) * fold;
        bl = tl + (bl - tl) * fold;
    }
    scaled(u, [tl, tr, br, bl])
}

/// Badge corners, grown about their centre to at least
/// `BADGE_MIN_PX × ui_scale` device pixels tall (capped at 0.4 of the
/// avatar's height on screen), so the shape stays visible when zoomed out.
pub(crate) fn badge_corners(ctx: &DecoCtx, u: &UserView) -> [Vec2; 4] {
    let c = scaled(u, corners(u, BADGE_RECT));
    let vp = ctx.view.viewport;
    let project = |p: Vec2| project_with(&ctx.world_to_clip, vp, p.extend(0.0));
    let avatar_px = (project(u.pos + Vec2::new(0.0, u.size.y * 0.5))
        - project(u.pos - Vec2::new(0.0, u.size.y * 0.5)))
    .length();
    let world_h = c[3].y - c[0].y;
    if avatar_px <= 0.0 || u.size.y <= 0.0 || world_h <= 0.0 {
        return c;
    }
    let badge_px = world_h * avatar_px / u.size.y;
    let min_px = (BADGE_MIN_PX * ctx.ui_scale).min(BADGE_MAX_FRACTION * avatar_px);
    if badge_px >= min_px {
        return c;
    }
    let grow = min_px / badge_px;
    let centre = (c[0] + c[2]) * 0.5;
    c.map(|p| centre + (p - centre) * grow)
}

/// Texcoords of `cell` in [`quad4`] order.
fn cell_tc(cell: DecoCell) -> [f32; 4] {
    cell.uv()
}

/// The back block: per user, the cape, then the halo.
pub(crate) fn back_block(ctx: &DecoCtx, out: &mut Vec<QuadVertex>) {
    for u in &ctx.view.users {
        let d = &u.decor;
        if d.mask & (DecoKind::Cape.bit() | DecoKind::Halo.bit()) == 0 {
            continue;
        }
        if d.has(DecoKind::Cape) {
            let c = colour(d, DecoKind::Cape, u.alpha);
            if c.w > 0.0 {
                quad4(out, cape_corners(u), c, cell_tc(DecoCell::Cape));
            }
        }
        if d.has(DecoKind::Halo) {
            let c = colour(d, DecoKind::Halo, u.alpha);
            if c.w > 0.0 {
                let corners = scaled(u, corners(u, HALO_RECT));
                quad4(out, corners, c, cell_tc(DecoCell::Halo));
            }
        }
    }
}

/// The front block: per user, the shades, the hat, then the badge.
pub(crate) fn front_block(ctx: &DecoCtx, out: &mut Vec<QuadVertex>) {
    let front = DecoKind::Shades.bit() | DecoKind::Hat.bit() | DecoKind::Badge.bit();
    for u in &ctx.view.users {
        let d = &u.decor;
        if d.mask & front == 0 {
            continue;
        }
        for (kind, rect, cell) in [
            (DecoKind::Shades, SHADES_RECT, DecoCell::Shades),
            (DecoKind::Hat, HAT_RECT, DecoCell::Hat),
        ] {
            if d.has(kind) {
                let c = colour(d, kind, u.alpha);
                if c.w > 0.0 {
                    quad4(out, scaled(u, corners(u, rect)), c, cell_tc(cell));
                }
            }
        }
        if d.has(DecoKind::Badge) {
            let c = colour(d, DecoKind::Badge, u.alpha);
            if c.w > 0.0 {
                let cell = DecoCell::BADGES[d.badge as usize % DecoCell::BADGES.len()];
                quad4(out, badge_corners(ctx, u), c, cell_tc(cell));
            }
        }
    }
}

/// Glow quads (AI-58), in the bloom format of directory nodes: colour
/// `tint × 0.85 × alpha × fade`, radius `3 × width × scale`, texcoord
/// `(radius, centre.x, centre.y, 0)`.
pub(crate) fn glow(view: &SceneView, out: &mut Vec<BloomVertex>) {
    for u in &view.users {
        let d = &u.decor;
        if !d.has(DecoKind::Glow) {
            continue;
        }
        let strength = GLOW_STRENGTH * u.alpha * d.fade(DecoKind::Glow);
        if strength <= 0.0 {
            continue;
        }
        let [r, g, b] = d.tint[DecoKind::Glow.index()];
        let col = Vec4::new(
            r as f32 / 255.0 * strength,
            g as f32 / 255.0 * strength,
            b as f32 / 255.0 * strength,
            1.0,
        );
        let radius = GLOW_RADIUS * u.size.x * d.scale;
        let dims = Vec2::splat(radius * 2.0);
        bloom_quad(
            out,
            u.pos - dims * 0.5,
            dims,
            col,
            [radius, u.pos.x, u.pos.y, 0.0],
        );
    }
}

#[cfg(test)]
mod tests;
