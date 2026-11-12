// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! `RAction` and its subclasses (`action.cpp`): `CreateAction` (green),
//! `ModifyAction` (orange, then sets the file colour) and `RemoveAction`
//! (red, starts the file's removal when it completes), without the GL
//! drawing.
//!
//! The C++ virtuals become [`ActionKind`]. An action lives inside its user's
//! pending or active list (`RUser::actions` / `activeActions`), so its
//! `source` is implicit; the one thing `logic` reads from the source,
//! `getPendingActionCount()`, is passed in. The target file is a
//! [`FileId`]; Gource removes every action of a file before deleting it
//! (`RUser::fileRemoved`), so targets are always alive, and the methods here
//! skip a stale id instead of panicking.

use glam::Vec3;

use crate::file::FileId;
use crate::pull_request::PrKey;
use crate::tree::DirTree;
use crate::vectors::{cpp_max, cpp_min};

/// Beam colour of a `CreateAction`.
pub(crate) const CREATE_COLOUR: Vec3 = Vec3::new(0.0, 1.0, 0.0);
/// Beam colour of a `RemoveAction`.
pub(crate) const REMOVE_COLOUR: Vec3 = Vec3::new(1.0, 0.0, 0.0);
/// Beam colour of a `ModifyAction`.
pub(crate) const MODIFY_COLOUR: Vec3 = Vec3::new(1.0, 0.7, 0.3);
/// Beam colour of a merged pull request (a `RemoveAction` of its node;
/// [`crate::events::ChangeEvent::PrMerged`]). Not in Gource.
pub(crate) const PR_MERGED_COLOUR: Vec3 = Vec3::new(0.64, 0.44, 0.97);
/// Beam colour of a pull request closed without merging.
pub(crate) const PR_CLOSED_COLOUR: Vec3 = Vec3::new(0.55, 0.58, 0.62);

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) enum ActionKind {
    Create,
    /// Carries `modify_colour`, the colour the file takes when applied.
    Modify(Vec3),
    Remove,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct RAction {
    pub(crate) kind: ActionKind,
    pub(crate) target: FileId,
    /// Commit timestamp.
    pub(crate) timestamp: i64,
    /// Simulation runtime when the commit was processed.
    pub(crate) t: f32,
    pub(crate) progress: f32,
    pub(crate) rate: f32,
    pub(crate) colour: Vec3,
    /// Attribution set id of the commit (0 = none), AI-30.
    pub(crate) attr: u32,
    /// Rich actions are cosmetic and cannot mutate lifecycle geometry.
    pub(crate) pr: Option<(PrKey, u64)>,
}

impl RAction {
    fn new(kind: ActionKind, target: FileId, timestamp: i64, t: f32, colour: Vec3) -> Self {
        RAction {
            kind,
            target,
            timestamp,
            t,
            progress: 0.0,
            rate: 0.5,
            colour,
            attr: 0,
            pr: None,
        }
    }

    /// `CreateAction(source, target, timestamp, t)`.
    pub(crate) fn create(target: FileId, timestamp: i64, t: f32) -> Self {
        RAction::new(ActionKind::Create, target, timestamp, t, CREATE_COLOUR)
    }

    /// `RemoveAction(source, target, timestamp, t)`.
    pub(crate) fn remove(target: FileId, timestamp: i64, t: f32) -> Self {
        RAction::new(ActionKind::Remove, target, timestamp, t, REMOVE_COLOUR)
    }

    /// `ModifyAction(source, target, timestamp, t, modify_colour)`.
    pub(crate) fn modify(target: FileId, timestamp: i64, t: f32, modify_colour: Vec3) -> Self {
        RAction::new(
            ActionKind::Modify(modify_colour),
            target,
            timestamp,
            t,
            MODIFY_COLOUR,
        )
    }

    /// The action with another beam (and touch) colour: pull request events.
    pub(crate) fn with_beam(mut self, colour: Vec3) -> Self {
        self.colour = colour;
        self
    }

    /// The action credited to the commit's attribution set `attr`.
    pub(crate) fn with_attr(mut self, attr: u32) -> Self {
        self.attr = attr;
        self
    }

    pub(crate) fn with_pr(mut self, key: PrKey, generation: u64) -> Self {
        self.pr = Some((key, generation));
        self
    }

    /// `isFinished()`.
    pub(crate) fn is_finished(&self) -> bool {
        self.progress >= 1.0
    }

    /// `apply()`: touch the file with the beam colour; a modification then
    /// sets the file colour.
    fn apply(&self, tree: &mut DirTree) {
        if self.pr.is_some() || !tree.files.contains(self.target) {
            return;
        }
        tree.touch_file(self.target, self.timestamp, self.colour);
        if let ActionKind::Modify(modify_colour) = self.kind {
            tree.files[self.target].set_file_colour(modify_colour);
        }
    }

    /// `logic(dt)` with the source's `getPendingActionCount()`; a
    /// `RemoveAction` starts removing its file on the tick it completes.
    pub(crate) fn logic(&mut self, dt: f32, pending_count: usize, tree: &mut DirTree) {
        if let Some((key, generation)) = self.pr
            && !tree
                .files
                .get(self.target)
                .is_some_and(|file| file.pr_key == Some(key) && file.pr_generation == generation)
        {
            self.progress = 1.0;
            return;
        }
        let old_progress = self.progress;

        // RAction::logic
        if self.progress < 1.0 {
            if self.progress == 0.0 {
                self.apply(tree);
            }

            let action_rate = cpp_min(10.0, self.rate * cpp_max(1.0, pending_count as f32));

            self.progress = cpp_min(self.progress + action_rate * dt, 1.0);
        }

        if self.kind == ActionKind::Remove
            && self.pr.is_none()
            && old_progress < 1.0
            && self.progress >= 1.0
            && tree.files.contains(self.target)
        {
            tree.remove_file_at(self.target, self.timestamp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    fn tree_with(path: &str) -> (DirTree, FileId) {
        let s = Settings::default();
        let mut tree = DirTree::new(&s);
        let f = tree.add_file(path, Vec3::ONE, 1, &s).unwrap();
        (tree, f)
    }

    #[test]
    fn first_tick_applies_and_progress_follows_the_rate_rule() {
        let (mut tree, f) = tree_with("/a/b.c");
        let mut a = RAction::create(f, 10, 0.0);
        assert!(tree.files[f].is_hidden());
        a.logic(0.5, 0, &mut tree);
        // touched: visible, touch colour green
        assert!(!tree.files[f].is_hidden());
        assert_eq!(tree.files[f].touch_colour, CREATE_COLOUR);
        // rate 0.5 * max(1, 0) * 0.5
        assert_eq!(a.progress, 0.25);
        // pending 8: rate 0.5 * 8 = 4 per second
        a.logic(0.1, 8, &mut tree);
        assert_eq!(a.progress, 0.25 + 4.0 * 0.1);
        // pending 100: capped at 10 per second, progress capped at 1
        a.logic(0.1, 100, &mut tree);
        assert_eq!(a.progress, 1.0);
        assert!(a.is_finished());
    }

    #[test]
    fn modify_sets_the_file_colour_and_remove_starts_the_fade() {
        let (mut tree, f) = tree_with("/a/b.c");
        let red = Vec3::new(1.0, 0.0, 0.0);
        let mut m = RAction::modify(f, 10, 0.0, red);
        m.logic(0.1, 0, &mut tree);
        assert_eq!(tree.files[f].file_colour(), red);
        assert_eq!(tree.files[f].touch_colour, MODIFY_COLOUR);

        let mut r = RAction::remove(f, 20, 0.0);
        r.rate = 2.0;
        r.logic(0.25, 1, &mut tree);
        assert!(!tree.files[f].removing);
        r.logic(0.25, 1, &mut tree);
        assert!(r.is_finished());
        assert!(tree.files[f].removing);
        assert_eq!(tree.files[f].removed_timestamp, 20);
        // Further ticks do nothing.
        tree.files[f].removing = false;
        r.logic(0.25, 1, &mut tree);
        assert!(!tree.files[f].removing);
    }
}
