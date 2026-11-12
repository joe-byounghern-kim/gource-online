// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! `RUser` (`user.cpp`) and the `UserForceFunctor` force rule, without the
//! GL drawing.
//!
//! Faithfulness notes:
//! - `actions` (pending) is a `VecDeque` and `activeActions` a `Vec`; both
//!   keep the `std::list` order. Pending actions are created in runtime
//!   order, so the "too old" ones `logic` activates form a prefix.
//! - `actionCount` / `activeCount` are separate counters, as in the C++,
//!   including its quirk: `fileRemoved` erases active actions without
//!   decrementing `activeCount`, so `getActionCount()` can stay above the
//!   real number of actions. `isIdle()` looks at the lists, not the counters.
//! - `colourize()` (used by `changeColours`) sets the raw `colourHash` of the
//!   name, while the constructor (`assignUserImage`) mixes it 60/40 with
//!   white and dims it by 0.9; recoloured users are therefore more saturated
//!   than new ones, as in Gource.
//! - The two `rand()` calls (overlap resolution in `applyForceUser` and
//!   `applyForceAction`) draw from [`RandStream`]s owned by the simulation
//!   (keyed streams [`USER_FORCE_RNG_KEY`] and [`ACTION_FORCE_RNG_KEY`], or
//!   the shared glibc `rand()` with `rand-mode glibc`), y before x,
//!   matching g++'s right-to-left evaluation of the `vec2` arguments.
//! - `UserForceFunctor`'s `std::set<RUser*> seen` becomes a per-user stamp
//!   (`force_seen`), with the same "dedupe only items spanning several
//!   leaves" rule.
//! - No avatars: every user has the `user.png` graphic (384 x 512).
//!
//! Port addition, the assist state of AI attribution (AI-31): while one of
//! the user's actions on screen (the front pending one, or an unfinished
//! active one) belongs to a credited commit, `attr_set` names that commit's
//! attribution set and the envelope `attr_env` rises to 1 over
//! [`ASSIST_FADE_IN`]; after the last credited action it holds for
//! [`ASSIST_HOLD`] and falls over [`ASSIST_FADE_OUT`]. Nothing here reads
//! wall time: `dt` is the scaled simulation step.

use std::collections::VecDeque;

use glam::{Vec2, Vec3};

use crate::action::RAction;
use crate::arena::Id;
use crate::events::StrId;
use crate::file::FileId;
use crate::hash::{DEFAULT_HASH_SEED, colour_hash, keyed_u64, stream_key};
use crate::pawn::Pawn;
use crate::rand::RandStream;
use crate::settings::Settings;
use crate::style::ResolvedStyle;
use crate::tree::DirTree;
use crate::vectors::{cpp_max, cpp_min, length, length2, normalise2};

pub(crate) type UserId = Id<RUser>;

/// `gGourceBeamDist`.
pub(crate) const BEAM_DIST: f32 = 100.0;
/// `gGourceActionDist`.
pub(crate) const ACTION_DIST: f32 = 50.0;
/// `gGourcePersonalSpaceDist`.
pub(crate) const PERSONAL_SPACE_DIST: f32 = 100.0;

/// Pixel size of `data/user.png`.
pub(crate) const USER_GRAPHIC_SIZE: (u32, u32) = (384, 512);

/// Stream key of the `rand()` replacement in `applyForceUser`.
pub(crate) const USER_FORCE_RNG_KEY: u64 = stream_key(b"user.applyForceUser");
/// Stream key of the `rand()` replacement in `applyForceAction`.
pub(crate) const ACTION_FORCE_RNG_KEY: u64 = stream_key(b"user.applyForceAction");

/// Seconds of idleness after which a user is deleted (`isInactive`).
pub(crate) const USER_INACTIVE_SECONDS: f32 = 10.0;

/// Seconds the assist state holds after the last credited action.
pub(crate) const ASSIST_HOLD: f32 = 2.0;
/// Seconds the assist envelope takes to rise from 0 to 1.
pub(crate) const ASSIST_FADE_IN: f32 = 0.25;
/// Seconds the assist envelope takes to fall from 1 to 0.
pub(crate) const ASSIST_FADE_OUT: f32 = 0.75;

/// Stream key of the per-user flutter phase (keyed by the user's log
/// string id, under a fixed seed so `hash-seed` does not reshuffle it).
const DECO_PHASE_KEY: u64 = stream_key(b"deco.phase");

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RUser {
    pub(crate) pawn: Pawn,

    /// Pending actions (`actions`).
    pub(crate) actions: VecDeque<RAction>,
    /// `activeActions`.
    pub(crate) active_actions: Vec<RAction>,
    /// `actionCount`.
    pub(crate) action_count: usize,
    /// `activeCount`.
    pub(crate) active_count: usize,

    pub(crate) action_interval: f32,
    pub(crate) last_action: f32,
    pub(crate) min_units_ps: f32,

    pub(crate) usercol: Vec3,
    pub(crate) highlighted: bool,

    /// `UserForceFunctor::seen` membership stamp.
    pub(crate) force_seen: u32,

    /// The user's name as a log string id.
    pub(crate) log_id: StrId,
    /// Attribution set of the credited action on screen, kept while the
    /// envelope fades out; 0 = none.
    pub(crate) attr_set: u32,
    /// Assist envelope, 0..=1.
    pub(crate) attr_env: f32,
    /// Seconds of hold left after the last credited action.
    pub(crate) attr_hold: f32,
    /// Flutter base phase in `[0, 1)`.
    pub(crate) deco_phase0: f64,
    /// `(style epoch, attr_set)` the cached `style` was resolved for.
    pub(crate) style_key: Option<(u32, u32)>,
    pub(crate) style: ResolvedStyle,
}

/// Flutter base phase of the user with log string id `id`, in `[0, 1)`.
fn phase_of(id: StrId) -> f64 {
    let v = keyed_u64(DEFAULT_HASH_SEED as u64, DECO_PHASE_KEY, id.0 as u64);
    (v >> 11) as f64 / (1u64 << 53) as f64
}

/// `vec2((rand() % 100) - 50, (rand() % 100) - 50)`, y drawn first.
fn random_direction(rng: &mut RandStream) -> Vec2 {
    let y = (rng.rand() % 100) - 50;
    let x = (rng.rand() % 100) - 50;
    Vec2::new(x as f32, y as f32)
}

impl RUser {
    /// `RUser(name, pos, tagid)` under hash seed `seed`; `log_id` is the
    /// name's string id in the event log.
    pub(crate) fn new(
        name: &str,
        log_id: StrId,
        pos: Vec2,
        tagid: i32,
        settings: &Settings,
        seed: i32,
    ) -> Self {
        let mut pawn = Pawn::new(name, pos, tagid);
        pawn.speed = settings.max_user_speed;
        pawn.size = (20.0f64 * settings.user_scale as f64) as f32;
        pawn.shadow_offset = Vec2::new(2.0, 2.0) * settings.user_scale;
        pawn.shadow = true;

        let mut user = RUser {
            pawn,
            actions: VecDeque::new(),
            active_actions: Vec::new(),
            action_count: 0,
            active_count: 0,
            action_interval: 0.2,
            last_action: 0.0,
            min_units_ps: 100.0,
            usercol: Vec3::ZERO,
            highlighted: false,
            force_seen: 0,
            log_id,
            attr_set: 0,
            attr_env: 0.0,
            attr_hold: 0.0,
            deco_phase0: phase_of(log_id),
            style_key: None,
            style: ResolvedStyle::default(),
        };

        user.assign_user_image(seed);
        user.set_selected(false);

        user.pawn.name_interval = 5.0;
        user
    }

    pub(crate) fn name(&self) -> &str {
        &self.pawn.name
    }

    pub(crate) fn pos(&self) -> Vec2 {
        self.pawn.pos
    }

    /// `assignUserImage()` without image directories: `user.png`, colour
    /// mixed with white.
    fn assign_user_image(&mut self, seed: i32) {
        self.colourize(seed);
        self.pawn.set_graphic(Some(USER_GRAPHIC_SIZE));
        self.usercol = self.usercol * 0.6 + Vec3::ONE * 0.4;
        self.usercol *= 0.9;
    }

    /// `colourize()`.
    pub(crate) fn colourize(&mut self, seed: i32) {
        self.usercol = colour_hash(self.pawn.name.as_bytes(), seed);
    }

    /// The size-dependent part of the constructor, for a live `--user-scale`
    /// change.
    pub(crate) fn set_scale(&mut self, user_scale: f32) {
        self.pawn.size = (20.0f64 * user_scale as f64) as f32;
        self.pawn.shadow_offset = Vec2::new(2.0, 2.0) * user_scale;
        self.pawn.set_graphic(Some(USER_GRAPHIC_SIZE));
    }

    /// `addAction(action)`.
    pub(crate) fn add_action(&mut self, action: RAction) {
        if self.is_idle() {
            self.pawn.show_name();
        }
        self.actions.push_back(action);
        self.action_count += 1;
    }

    /// `fileRemoved(f)`: drop every action targeting `f`.
    pub(crate) fn file_removed(&mut self, f: FileId) {
        let before = self.actions.len();
        self.actions.retain(|a| a.target != f);
        self.action_count = self.action_count.wrapping_sub(before - self.actions.len());

        // activeCount is not decremented here (C++ quirk).
        self.active_actions.retain(|a| a.target != f);
    }

    /// `getActionCount()`.
    pub(crate) fn action_count_total(&self) -> usize {
        self.action_count.wrapping_add(self.active_count)
    }

    /// `getPendingActionCount()`.
    pub(crate) fn pending_action_count(&self) -> usize {
        self.action_count
    }

    /// `isIdle()`.
    pub(crate) fn is_idle(&self) -> bool {
        self.active_actions.is_empty() && self.actions.is_empty()
    }

    /// `isFading()`.
    pub(crate) fn is_fading(&self, settings: &Settings) -> bool {
        self.is_idle() && (self.pawn.elapsed - self.last_action) > settings.user_idle_time
    }

    /// `isInactive()`.
    pub(crate) fn is_inactive(&self) -> bool {
        self.is_idle()
            && ((self.pawn.elapsed - self.last_action) as f64) > USER_INACTIVE_SECONDS as f64
    }

    pub(crate) fn set_selected(&mut self, selected: bool) {
        self.pawn.selected = selected;
    }

    pub(crate) fn set_highlighted(&mut self, highlighted: bool) {
        self.highlighted = highlighted;
    }

    pub(crate) fn set_mouse_over(&mut self, over: bool) {
        self.pawn.mouseover = over;
    }

    /// `getColour()`.
    pub(crate) fn colour(&self) -> Vec3 {
        if self.pawn.selected {
            return Vec3::ONE;
        }
        self.usercol
    }

    /// `getNameColour()`.
    pub(crate) fn name_colour(&self, settings: &Settings) -> Vec3 {
        if self.pawn.selected {
            settings.selection_colour
        } else if self.highlighted {
            settings.highlight_colour
        } else {
            self.pawn.namecol
        }
    }

    /// `getAlpha()`: Pawn fade-in, then a 1 s fade out once idle for
    /// `user_idle_time`.
    pub(crate) fn alpha(&self, settings: &Settings) -> f32 {
        let mut alpha = self.pawn.base_alpha();
        let idle = self.pawn.elapsed - self.last_action;
        if idle > settings.user_idle_time {
            alpha = (1.0 - cpp_min(idle - settings.user_idle_time, 1.0) as f64) as f32;
        }
        alpha
    }

    /// `RUser::nameVisible()`.
    pub(crate) fn name_visible(&self, settings: &Settings) -> bool {
        self.pawn.name_visible() || settings.highlight_all_users || self.highlighted
    }

    /// `drawName()` + `RUser::drawNameText`: the label colour (RGB, alpha),
    /// or `None` when nothing is drawn.
    pub(crate) fn name_label(&self, settings: &Settings) -> Option<(Vec3, f32)> {
        if !self.name_visible(settings) {
            return None;
        }
        let alpha = self.pawn.draw_name_alpha();
        let user_alpha = self.alpha(settings);
        let emphasised = settings.highlight_all_users || self.highlighted || self.pawn.selected;
        if emphasised || alpha > 0.0 {
            let name_alpha = if emphasised { user_alpha } else { alpha };
            Some((self.name_colour(settings), name_alpha))
        } else {
            None
        }
    }

    /// World point the name label is anchored above (`calcScreenPos` before
    /// projection): the top centre of the graphic.
    pub(crate) fn label_anchor(&self) -> Vec2 {
        let mut text_pos = self.pawn.pos;
        text_pos.y -= self.pawn.dims.y * 0.5;
        text_pos
    }

    /// `applyForceUser(u)` for a user at `u_pos`: personal space depends on
    /// how busy this user is.
    pub(crate) fn apply_force_user(&mut self, u_pos: Vec2, rng: &mut RandStream) {
        let dir = u_pos - self.pawn.pos;
        let dist = length(dir);

        let desired_dist = if self.action_count_total() == 0 {
            PERSONAL_SPACE_DIST
        } else if !self.actions.is_empty() && self.active_actions.is_empty() {
            (PERSONAL_SPACE_DIST as f64 * 0.1) as f32
        } else {
            (PERSONAL_SPACE_DIST as f64 * 0.5) as f32
        };

        // resolve overlap
        if (dist as f64) < 0.001 {
            self.pawn.accel += 1.0 * normalise2(random_direction(rng));
            return;
        }

        // repelling force
        if dist < desired_dist {
            self.pawn.accel -= (desired_dist - dist) * normalise2(dir);
        }
    }

    /// `applyForceAction(action)` for a target file at `f_pos`.
    pub(crate) fn apply_force_action(&mut self, f_pos: Vec2, rng: &mut RandStream) {
        let dir = f_pos - self.pawn.pos;
        let dist = length(dir);

        let desired_dist = ACTION_DIST;

        // resolve overlap
        if (dist as f64) < 0.001 {
            self.pawn.accel += normalise2(random_direction(rng));
            return;
        }

        // repelling force
        if dist < desired_dist {
            self.pawn.accel -= (desired_dist - dist) * normalise2(dir);
            return;
        }

        if dist > BEAM_DIST {
            self.pawn.accel += (dist - BEAM_DIST) * normalise2(dir);
        }
    }

    /// `applyForceToActions()`: towards up to three active actions, or the
    /// first pending one when none is active.
    pub(crate) fn apply_force_to_actions(&mut self, tree: &DirTree, rng: &mut RandStream) {
        if self.active_actions.is_empty() && self.actions.is_empty() {
            return;
        }

        self.last_action = self.pawn.elapsed;

        let max_influence = 3;

        // move towards actions being worked on
        for i in 0..self.active_actions.len().min(max_influence) {
            let target = self.active_actions[i].target;
            if tree.files.contains(target) {
                let f_pos = tree.file_abs_pos(target);
                self.apply_force_action(f_pos, rng);
            }
        }

        if !self.active_actions.is_empty() {
            return;
        }

        // if no actions being worked on, move towards one pending action
        if let Some(action) = self.actions.front() {
            let target = action.target;
            if tree.files.contains(target) {
                let f_pos = tree.file_abs_pos(target);
                self.apply_force_action(f_pos, rng);
            }
        }
    }

    /// `logic(t, dt)`: activate pending actions (all older than
    /// `max_file_lag`, then the first one within beam distance), advance the
    /// active ones, then move with the speed cap and friction.
    pub(crate) fn logic(&mut self, t: f32, dt: f32, settings: &Settings, tree: &mut DirTree) {
        self.pawn.logic(dt);

        self.action_interval -= dt;

        let find_nearby_action = !self.actions.is_empty() && self.action_interval <= 0.0;

        // add next active action, if it is in range
        let mut i = 0;
        while i < self.actions.len() {
            let action = &self.actions[i];

            // add all files which are too old
            if settings.max_file_lag as f64 >= 0.0 && action.t < t - settings.max_file_lag {
                let mut action = self.actions.remove(i).expect("index in range");
                self.action_count = self.action_count.wrapping_sub(1);
                action.rate = 2.0;
                self.active_actions.push(action);
                self.active_count += 1;
                continue;
            }

            if !find_nearby_action {
                break;
            }

            let target = action.target;
            let action_dist = if tree.files.contains(target) {
                length(tree.file_abs_pos(target) - self.pawn.pos)
            } else {
                f32::INFINITY
            };

            // queue first action in range
            if action_dist < BEAM_DIST {
                let action = self.actions.remove(i).expect("index in range");
                self.active_actions.push(action);
                self.action_count = self.action_count.wrapping_sub(1);
                self.active_count += 1;
                break;
            }

            i += 1;
        }

        // reset action interval
        if self.action_interval <= 0.0 {
            let total_actions = self.action_count.wrapping_add(self.active_count);
            self.action_interval = if total_actions != 0 {
                (1.0f64 / (total_actions as f32) as f64) as f32
            } else {
                1.0
            };
        }

        // update actions
        let pending = self.pending_action_count();
        let mut removed = 0;
        let mut credited = 0;
        self.active_actions.retain_mut(|action| {
            action.logic(dt, pending, tree);
            if action.is_finished() {
                removed += 1;
                false
            } else {
                if credited == 0 {
                    credited = action.attr;
                }
                true
            }
        });
        self.active_count = self.active_count.wrapping_sub(removed);
        if credited == 0 {
            credited = self.actions.front().map_or(0, |a| a.attr);
        }
        self.assist_logic(credited, dt);

        let speed = self.pawn.speed;
        if length2(self.pawn.accel) > speed * speed {
            self.pawn.accel = normalise2(self.pawn.accel) * speed;
        }

        self.pawn.pos += self.pawn.accel * dt;

        self.pawn.accel *= cpp_max(0.0, 1.0 - settings.user_friction * dt);
    }

    /// `updateQuadItemBounds()`.
    pub(crate) fn update_quad_item_bounds(&mut self) {
        self.pawn.update_quad_item_bounds();
    }

    /// Advance the assist state by `dt`; `credited` is the set id of a
    /// credited action on screen (0 = none).
    pub(crate) fn assist_logic(&mut self, credited: u32, dt: f32) {
        if credited != 0 {
            self.attr_set = credited;
            self.attr_hold = ASSIST_HOLD;
        } else {
            self.attr_hold = (self.attr_hold - dt).max(0.0);
        }
        let target = if credited != 0 || self.attr_hold > 0.0 {
            1.0
        } else {
            0.0
        };
        self.attr_env += (target - self.attr_env).clamp(-dt / ASSIST_FADE_OUT, dt / ASSIST_FADE_IN);
        if self.attr_env == 0.0 && credited == 0 {
            self.attr_set = 0;
        }
    }

    /// The flutter phase at the user's current age, in `[0, 1)`.
    pub(crate) fn deco_phase(&self) -> f32 {
        let p = self.deco_phase0 + 1.5 * self.pawn.elapsed as f64;
        (p - libm::floor(p)) as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> Settings {
        Settings::default()
    }

    #[test]
    fn constructor_matches_cpp() {
        let u = RUser::new("alice", StrId(0), Vec2::new(1.0, 2.0), 3, &s(), 31);
        assert_eq!(u.pawn.size, 20.0);
        assert_eq!(u.pawn.graphic_ratio, 512.0 / 384.0);
        assert_eq!(u.pawn.dims, Vec2::new(20.0, 20.0 * (512.0f32 / 384.0)));
        assert_eq!(u.pawn.speed, 500.0);
        assert_eq!(u.pawn.name_interval, 5.0);
        assert_eq!(u.pawn.nametime, 5.0);
        assert_eq!(u.action_interval, 0.2);
        let mixed = (colour_hash(b"alice", 31) * 0.6 + Vec3::ONE * 0.4) * 0.9;
        assert_eq!(u.usercol, mixed);
        let mut v = u.clone();
        v.colourize(31);
        assert_eq!(v.usercol, colour_hash(b"alice", 31));
    }

    #[test]
    fn idle_fading_and_inactive_thresholds() {
        let settings = s();
        let mut u = RUser::new("a", StrId(0), Vec2::ZERO, 1, &settings, 31);
        assert!(u.is_idle());
        u.pawn.elapsed = 3.5;
        assert!(u.is_fading(&settings));
        assert!((u.alpha(&settings) - 0.5).abs() < 1e-6);
        assert!(!u.is_inactive());
        u.pawn.elapsed = 10.5;
        assert!(u.is_inactive());
        assert_eq!(u.alpha(&settings), 0.0);
    }

    #[test]
    fn name_label_rules() {
        let mut settings = s();
        let mut u = RUser::new("a", StrId(0), Vec2::ZERO, 1, &settings, 31);
        u.pawn.elapsed = 1.0;
        // name_interval 5 and nametime 5: done 0, alpha 0, nothing drawn.
        assert_eq!(u.name_label(&settings), None);
        u.pawn.name_interval = 4.5;
        assert_eq!(u.name_label(&settings), Some((Vec3::ONE, 0.5)));
        u.pawn.name_interval = -1.0;
        assert_eq!(u.name_label(&settings), None);
        settings.highlight_all_users = true;
        // Highlighted names use the user's alpha.
        assert_eq!(u.name_label(&settings), Some((Vec3::ONE, 1.0)));
        settings.highlight_all_users = false;
        u.set_highlighted(true);
        settings.highlight_colour = Vec3::new(1.0, 0.0, 0.0);
        assert_eq!(
            u.name_label(&settings),
            Some((Vec3::new(1.0, 0.0, 0.0), 1.0))
        );
        u.set_selected(true);
        assert_eq!(
            u.name_label(&settings).unwrap().0,
            settings.selection_colour
        );
        assert_eq!(u.colour(), Vec3::ONE);
    }

    #[test]
    fn personal_space_depends_on_activity() {
        let mut rng = RandStream::new(31, USER_FORCE_RNG_KEY, None);
        let mut u = RUser::new("a", StrId(0), Vec2::ZERO, 1, &s(), 31);
        u.apply_force_user(Vec2::new(60.0, 0.0), &mut rng);
        // idle: 100 - 60 = 40 away from the other user
        assert_eq!(u.pawn.accel, Vec2::new(-40.0, 0.0));
        u.pawn.accel = Vec2::ZERO;
        u.apply_force_user(Vec2::ZERO, &mut rng);
        assert!((u.pawn.accel.length() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn speed_cap_and_friction() {
        let settings = s();
        let mut tree = DirTree::new(&settings);
        let mut u = RUser::new("a", StrId(0), Vec2::ZERO, 1, &settings, 31);
        u.pawn.accel = Vec2::new(1000.0, 0.0);
        u.logic(0.0, 0.1, &settings, &mut tree);
        // capped at 500, moved 50, then damped by (1 - 1 * 0.1)
        assert_eq!(u.pawn.pos, Vec2::new(50.0, 0.0));
        assert_eq!(u.pawn.accel, Vec2::new(450.0, 0.0));
    }

    #[test]
    fn old_actions_activate_at_double_rate_and_counters_follow_cpp() {
        let settings = s();
        let mut tree = DirTree::new(&settings);
        let f = tree.add_file("/x/a.c", Vec3::ONE, 1, &settings).unwrap();
        let g = tree.add_file("/x/b.c", Vec3::ONE, 2, &settings).unwrap();
        let mut u = RUser::new("a", StrId(0), Vec2::new(1000.0, 0.0), 3, &settings, 31);
        u.add_action(RAction::create(f, 1, 0.0));
        u.add_action(RAction::create(g, 1, 0.0));
        assert_eq!(u.action_count, 2);
        // t = 6 > 0 + max_file_lag: both activate with rate 2.
        u.logic(6.0, 0.01, &settings, &mut tree);
        assert_eq!(u.action_count, 0);
        assert_eq!(u.active_count, 2);
        assert!(u.active_actions.iter().all(|a| a.rate == 2.0));
        // fileRemoved leaves activeCount alone.
        u.file_removed(f);
        assert_eq!(u.active_actions.len(), 1);
        assert_eq!(u.active_count, 2);
        assert_eq!(u.action_count_total(), 2);
    }

    /// Steps of `dt` until `f` holds, at most `limit`.
    fn steps_until(u: &mut RUser, credited: u32, dt: f32, f: impl Fn(&RUser) -> bool) -> usize {
        let mut n = 0;
        while !f(u) {
            u.assist_logic(credited, dt);
            n += 1;
            assert!(n < 10_000);
        }
        n
    }

    #[test]
    fn assist_envelope_rises_holds_and_falls() {
        // 1/60 s frames at time-scale 2: the simulation dt is 1/30 s
        let dt = (1.0f32 / 60.0) * 2.0;
        let mut u = RUser::new("a", StrId(0), Vec2::ZERO, 1, &s(), 31);
        assert_eq!((u.attr_set, u.attr_env), (0, 0.0));

        let rise = steps_until(&mut u, 7, dt, |u| u.attr_env == 1.0);
        assert_eq!(u.attr_set, 7);
        assert_eq!(rise, (ASSIST_FADE_IN / dt).ceil() as usize);

        // credited for a while, then nothing: hold 2 s at full strength
        u.assist_logic(7, dt);
        let hold = steps_until(&mut u, 0, dt, |u| u.attr_env < 1.0);
        let hold_secs = hold as f32 * dt;
        assert!((hold_secs - ASSIST_HOLD).abs() <= 1.5 * dt, "{hold_secs}");
        assert_eq!(u.attr_set, 7, "the set stays while fading out");

        let fall = steps_until(&mut u, 0, dt, |u| u.attr_env == 0.0);
        let fall_secs = (fall + 1) as f32 * dt;
        assert!(
            (fall_secs - ASSIST_FADE_OUT).abs() <= 1.5 * dt,
            "{fall_secs}"
        );
        assert_eq!(u.attr_set, 0);
    }

    #[test]
    fn assist_state_follows_the_actions_on_screen() {
        let settings = s();
        let mut tree = DirTree::new(&settings);
        let f = tree.add_file("/x/a.c", Vec3::ONE, 1, &settings).unwrap();
        let g = tree.add_file("/x/b.c", Vec3::ONE, 2, &settings).unwrap();
        let mut u = RUser::new("a", StrId(0), Vec2::new(1000.0, 0.0), 3, &settings, 31);
        // A queued credited action far from the user: the front of the
        // pending queue counts.
        u.add_action(RAction::create(f, 1, 0.0).with_attr(3));
        u.add_action(RAction::create(g, 1, 0.0));
        u.logic(0.0, 0.1, &settings, &mut tree);
        assert_eq!(u.attr_set, 3);
        assert!(u.attr_env > 0.0);
        // Behind an uncredited front action it does not.
        let mut v = RUser::new("b", StrId(1), Vec2::new(1000.0, 0.0), 4, &settings, 31);
        v.add_action(RAction::create(g, 1, 0.0));
        v.add_action(RAction::create(f, 1, 0.0).with_attr(3));
        v.logic(0.0, 0.1, &settings, &mut tree);
        assert_eq!((v.attr_set, v.attr_env), (0, 0.0));
        // Activated by max_file_lag: active, unfinished, credited.
        v.logic(6.0, 0.01, &settings, &mut tree);
        assert_eq!(v.attr_set, 3);
    }

    #[test]
    fn phase_is_keyed_by_log_id_not_by_hash_seed() {
        let a = RUser::new("a", StrId(5), Vec2::ZERO, 1, &s(), 31);
        let b = RUser::new("a", StrId(5), Vec2::ZERO, 1, &s(), 1234);
        let c = RUser::new("a", StrId(6), Vec2::ZERO, 1, &s(), 31);
        assert_eq!(a.deco_phase0, b.deco_phase0);
        assert_ne!(a.deco_phase0, c.deco_phase0);
        assert!((0.0..1.0).contains(&a.deco_phase0));
        let mut d = a.clone();
        d.pawn.elapsed = 1.0 / 1.5;
        assert!((d.deco_phase() as f64 - a.deco_phase0).abs() < 1e-6);
    }
}
