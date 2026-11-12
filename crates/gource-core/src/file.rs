// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! `RFile` (`file.cpp`) without its GL drawing.
//!
//! Everything that only touches the file itself lives here. Operations that
//! reach into its directory or the removed-files queue (`touch`, `setHidden`,
//! `logic`, `getAbsolutePos`, `overlaps`) are methods of
//! [`crate::tree::DirTree`], which owns both.
//!
//! The file graphic (`data/file.png`) is 256 x 256, so `graphic_ratio` is 1
//! and `dims` is `(size, size)`.

use glam::{Vec2, Vec3};

use crate::arena::Id;
use crate::bounds::Bounds2D;
use crate::dirnode::DirId;
use crate::hash::colour_hash;
use crate::pawn::Pawn;
use crate::pull_request::PrKey;
use crate::settings::Settings;
use crate::vectors::glm_clamp;

pub(crate) type FileId = Id<RFile>;

/// `gGourceFileDiameter`.
pub(crate) const FILE_DIAMETER: f32 = 8.0;

/// Pixel size of `data/file.png`.
pub(crate) const FILE_GRAPHIC_SIZE: (u32, u32) = (256, 256);

/// Screen offset of a file label from the file centre (`calcScreenPos`).
pub(crate) const FILE_LABEL_OFFSET: Vec2 = Vec2::new(5.5, -1.0);
/// Label offset while the file is selected.
pub(crate) const FILE_LABEL_OFFSET_SELECTED: Vec2 = Vec2::new(5.5, -2.0);

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RFile {
    pub(crate) pawn: Pawn,

    pub(crate) file_colour: Vec3,
    pub(crate) touch_colour: Vec3,

    pub(crate) dir: Option<DirId>,

    pub(crate) removed_timestamp: i64,
    pub(crate) forced_removal: bool,
    pub(crate) expired: bool,
    pub(crate) removing: bool,
    pub(crate) pr_key: Option<PrKey>,
    pub(crate) pr_generation: u64,
    pub(crate) pr_pinned: bool,

    pub(crate) fade_start: f32,
    pub(crate) last_action: f32,
    pub(crate) radius: f32,

    /// Ring slot direction (relative to the dir), from `calcFileDest`.
    pub(crate) dest: Vec2,
    /// Ring radius for `dest`.
    pub(crate) distance: f32,

    /// Directory part of `fullpath`, with the trailing `/`.
    pub(crate) path: String,
    pub(crate) fullpath: String,
    pub(crate) ext: String,
}

impl RFile {
    /// `RFile(name, colour, pos, tagid)`. The file starts hidden.
    pub(crate) fn new(
        name: &str,
        colour: Vec3,
        pos: Vec2,
        tagid: i32,
        settings: &Settings,
    ) -> Self {
        let mut pawn = Pawn::new(name, pos, tagid);
        pawn.hidden = true;
        pawn.size = (FILE_DIAMETER as f64 * 1.05) as f32;
        pawn.set_graphic(Some(FILE_GRAPHIC_SIZE));
        pawn.speed = 5.0;
        pawn.nametime = settings.filename_time;
        pawn.name_interval = pawn.nametime;
        pawn.namecol = Vec3::ONE;
        pawn.shadow = true;
        pawn.selected = false;

        let radius = pawn.size * 0.5;

        let mut file = RFile {
            pawn,
            file_colour: colour,
            touch_colour: Vec3::ZERO,
            dir: None,
            removed_timestamp: 0,
            forced_removal: false,
            expired: false,
            removing: false,
            pr_key: None,
            pr_generation: 0,
            pr_pinned: false,
            fade_start: -1.0,
            last_action: 0.0,
            radius,
            dest: Vec2::ZERO,
            distance: 0.0,
            path: String::new(),
            fullpath: String::new(),
            ext: String::new(),
        };
        file.set_filename(name, settings.file_extension_fallback);
        file
    }

    /// `setFilename(abs_file_path)`: split into `path` (up to and including
    /// the last `/`), `name` and `ext` (after the last `.`, unless the dot
    /// ends the name; the whole name with `--file-extension-fallback`).
    fn set_filename(&mut self, abs_file_path: &str, extension_fallback: bool) {
        self.fullpath = abs_file_path.to_string();

        match self.fullpath.rfind('/') {
            Some(pos) => {
                self.path = self.fullpath[..=pos].to_string();
                self.pawn.name = self.fullpath[pos + 1..].to_string();
            }
            None => {
                self.path = String::new();
                self.pawn.name = abs_file_path.to_string();
            }
        }

        let name = &self.pawn.name;
        match name.rfind('.') {
            Some(dotsep) if dotsep != name.len() - 1 => {
                self.ext = name[dotsep + 1..].to_string();
            }
            _ if extension_fallback => {
                self.ext = name.clone();
            }
            _ => {}
        }
    }

    pub(crate) fn name(&self) -> &str {
        &self.pawn.name
    }

    pub(crate) fn is_hidden(&self) -> bool {
        self.pawn.hidden
    }

    /// `colourize()`: extension colour under hash seed `seed`, white without
    /// an extension.
    pub(crate) fn colourize(&mut self, seed: i32) {
        self.file_colour = if !self.ext.is_empty() {
            colour_hash(self.ext.as_bytes(), seed)
        } else {
            Vec3::ONE
        };
    }

    /// `setFileColour` (ModifyAction).
    pub(crate) fn set_file_colour(&mut self, colour: Vec3) {
        self.file_colour = colour;
    }

    pub(crate) fn file_colour(&self) -> Vec3 {
        self.file_colour
    }

    /// `getNameColour()`.
    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn name_colour(&self, settings: &Settings) -> Vec3 {
        if self.pawn.selected {
            settings.selection_colour
        } else {
            self.pawn.namecol
        }
    }

    /// `getColour()`: white when selected; blends from the touch colour to
    /// the file colour over the second after the last action.
    pub(crate) fn colour(&self) -> Vec3 {
        if self.pawn.selected {
            return Vec3::ONE;
        }
        let lc = self.pawn.elapsed - self.last_action;
        if lc < 1.0 {
            return self.touch_colour * (1.0 - lc) + self.file_colour * lc;
        }
        self.file_colour
    }

    /// `getAlpha()`: Pawn fade-in, overridden by the fade-out once
    /// `fade_start` is set.
    pub(crate) fn alpha(&self) -> f32 {
        let mut alpha = self.pawn.base_alpha();
        if self.fade_start > 0.0 {
            alpha = (1.0 - glm_clamp(self.pawn.elapsed - self.fade_start, 0.0, 1.0) as f64) as f32;
        }
        alpha
    }

    /// `remove(removed_timestamp)`: start fading out.
    pub(crate) fn remove(&mut self, removed_timestamp: i64) {
        if self.pr_pinned {
            return;
        }
        self.last_action = self.pawn.elapsed;
        self.fade_start = self.pawn.elapsed;
        self.removing = true;
        self.removed_timestamp = removed_timestamp;
    }

    /// `remove()`: forced removal (the file turned out to be a directory).
    pub(crate) fn remove_forced(&mut self) {
        if self.pr_pinned {
            return;
        }
        self.forced_removal = true;
        self.remove(0);
    }

    pub(crate) fn set_selected(&mut self, selected: bool) {
        self.pawn.selected = selected;
    }

    pub(crate) fn set_pr(&mut self, key: PrKey, generation: u64) {
        self.pr_key = Some(key);
        self.pr_generation = generation;
        self.pr_pinned = true;
        self.fade_start = -1.0;
        self.forced_removal = false;
        self.removing = false;
        self.expired = false;
    }

    pub(crate) fn set_mouse_over(&mut self, over: bool) {
        self.pawn.mouseover = over;
    }

    /// The label alpha `drawName()` + `drawNameText()` draw with, or `None`
    /// when no label is drawn. Selected files always draw at 1.
    pub(crate) fn label_alpha(&self) -> Option<f32> {
        let alpha = self.pawn.name_alpha()?;
        if !self.pawn.selected && (alpha as f64) <= 0.01 {
            return None;
        }
        Some(if self.pawn.selected { 1.0 } else { alpha })
    }

    /// The label text: the extension with `--file-extensions` (unless
    /// selected), otherwise the name.
    pub(crate) fn label_text<'a>(&'a self, settings: &Settings) -> &'a str {
        if !self.pawn.selected && settings.file_extensions {
            &self.ext
        } else {
            &self.pawn.name
        }
    }

    /// World position of the label anchor for a file whose absolute position
    /// is `abs_pos` (`calcScreenPos` before projection).
    pub(crate) fn label_anchor(&self, abs_pos: Vec2) -> Vec2 {
        abs_pos
            + if self.pawn.selected {
                FILE_LABEL_OFFSET_SELECTED
            } else {
                FILE_LABEL_OFFSET
            }
    }

    /// `overlaps(pos)` for a file whose absolute position is `abs_pos`.
    pub(crate) fn overlaps_at(&self, abs_pos: Vec2, pos: Vec2) -> bool {
        let halfsize_x = self.pawn.size * 0.5;
        let halfsize = Vec2::new(halfsize_x, halfsize_x * self.pawn.graphic_ratio);
        Bounds2D::new(abs_pos - halfsize, abs_pos + halfsize).contains(pos)
    }

    /// The movement and fade part of `RFile::logic`. Returns true when the
    /// file has just expired and must be queued in the removed-files list.
    pub(crate) fn logic(&mut self, dt: f32, settings: &Settings) -> bool {
        self.pawn.logic(dt);

        let dest_pos = self.dest * self.distance;

        self.pawn.accel = dest_pos - self.pawn.pos;

        let mut accel2 = self.pawn.accel * self.pawn.speed * dt;
        if accel2.dot(accel2) > self.pawn.accel.dot(self.pawn.accel) {
            accel2 = self.pawn.accel;
        }

        self.pawn.pos += accel2;

        // files have no momentum
        self.pawn.accel = Vec2::ZERO;

        if self.fade_start < 0.0
            && !self.pr_pinned
            && settings.file_idle_time > 0.0
            && (self.pawn.elapsed - self.last_action) > settings.file_idle_time
        {
            self.fade_start = self.pawn.elapsed;
        }

        let mut newly_expired = false;
        if self.fade_start > 0.0
            && !self.expired
            && ((self.pawn.elapsed - self.fade_start) as f64) >= 1.0
        {
            self.expired = true;
            newly_expired = true;
        }

        if self.pawn.hidden && !self.forced_removal {
            self.pawn.elapsed = 0.0;
        }

        newly_expired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> RFile {
        RFile::new(path, Vec3::ONE, Vec2::ZERO, 1, &Settings::default())
    }

    #[test]
    fn splits_path_name_and_extension() {
        let f = file("/src/core/main.cpp");
        assert_eq!(f.path, "/src/core/");
        assert_eq!(f.name(), "main.cpp");
        assert_eq!(f.fullpath, "/src/core/main.cpp");
        assert_eq!(f.ext, "cpp");
        assert!(f.is_hidden());
        assert_eq!(f.pawn.size, (8.0f64 * 1.05) as f32);
        assert_eq!(f.pawn.dims, Vec2::splat(f.pawn.size));

        assert_eq!(file("/Makefile").ext, "");
        assert_eq!(file("/trailing.").ext, "");
        assert_eq!(file("/.gitignore").ext, "gitignore");
        assert_eq!(file("/a.tar.gz").ext, "gz");

        let s = Settings {
            file_extension_fallback: true,
            ..Settings::default()
        };
        let f = RFile::new("/Makefile", Vec3::ONE, Vec2::ZERO, 1, &s);
        assert_eq!(f.ext, "Makefile");
    }

    #[test]
    fn colourize_uses_extension_hash() {
        let mut f = file("/x/a.rs");
        f.colourize(31);
        assert_eq!(f.file_colour, colour_hash(b"rs", 31));
        let mut g = file("/x/README");
        g.colourize(31);
        assert_eq!(g.file_colour, Vec3::ONE);
    }

    #[test]
    fn colour_blends_from_touch_colour() {
        let mut f = file("/a.c");
        f.file_colour = Vec3::new(0.0, 0.0, 1.0);
        f.touch_colour = Vec3::new(1.0, 0.0, 0.0);
        f.pawn.hidden = false;
        f.last_action = 0.0;
        f.pawn.elapsed = 0.25;
        assert_eq!(f.colour(), Vec3::new(0.75, 0.0, 0.25));
        f.pawn.elapsed = 1.0;
        assert_eq!(f.colour(), Vec3::new(0.0, 0.0, 1.0));
        f.set_selected(true);
        assert_eq!(f.colour(), Vec3::ONE);
    }

    #[test]
    fn approaches_destination_without_overshoot() {
        let s = Settings::default();
        let mut f = file("/a.c");
        f.pawn.hidden = false;
        f.dest = Vec2::new(1.0, 0.0);
        f.distance = 8.0;
        f.logic(0.1, &s);
        // accel * speed(5) * dt(0.1) = half the way.
        assert_eq!(f.pawn.pos, Vec2::new(4.0, 0.0));
        f.logic(1.0, &s);
        // speed * dt > 1 would overshoot: clamped to the remaining distance.
        assert_eq!(f.pawn.pos, Vec2::new(8.0, 0.0));
    }

    #[test]
    fn idle_fade_and_expiry_timing() {
        let s = Settings {
            file_idle_time: 2.0,
            ..Settings::default()
        };
        let mut f = file("/a.c");
        f.pawn.hidden = false;
        let dt = 0.25;
        let mut expired_at = None;
        let mut fade_at = None;
        for frame in 1..=40 {
            let expired = f.logic(dt, &s);
            if fade_at.is_none() && f.fade_start > 0.0 {
                fade_at = Some(frame);
            }
            if expired {
                assert!(expired_at.is_none(), "expires once");
                expired_at = Some(frame);
            }
        }
        // elapsed - last_action > 2.0 first holds at elapsed 2.25 (frame 9);
        // the fade completes 1 s later (frame 13).
        assert_eq!(fade_at, Some(9));
        assert_eq!(expired_at, Some(13));
        assert_eq!(f.alpha(), 0.0);
    }

    #[test]
    fn hidden_file_keeps_elapsed_at_zero() {
        let s = Settings::default();
        let mut f = file("/a.c");
        f.logic(0.5, &s);
        assert_eq!(f.pawn.elapsed, 0.0);
        // A forced removal keeps ticking while hidden.
        f.remove_forced();
        f.logic(0.5, &s);
        assert_eq!(f.pawn.elapsed, 0.5);
    }

    #[test]
    fn label_alpha_follows_name_timer() {
        let s = Settings::default(); // filename_time = 4
        let mut f = file("/a.c");
        f.pawn.hidden = false;
        let mut alphas = Vec::new();
        for _ in 0..10 {
            alphas.push(f.label_alpha());
            f.logic(0.5, &s);
        }
        assert_eq!(
            alphas,
            vec![
                None, // done = 0: alpha 0 <= 0.01 is not drawn
                Some(0.5),
                Some(3.0),
                Some(1.0),
                Some(1.0),
                Some(1.0),
                Some(1.0),
                Some(0.5),
                None,
                None,
            ]
        );
        f.set_selected(true);
        assert_eq!(f.label_alpha(), Some(1.0));
        assert_eq!(f.label_text(&s), "a.c");
    }
}
