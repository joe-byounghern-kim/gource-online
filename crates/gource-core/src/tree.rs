// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! [`DirTree`]: owner of every directory node and file, standing in for
//! Gource's globals `gGourceDirMap` and `gGourceRemovedFiles`, the `root`,
//! `files`, `dirNodeTree` and `dir_bounds` members of `Gource`, and the
//! tuning globals of `dirnode.cpp` / `file.cpp` ([`TreeConfig`]).
//!
//! # Internal API for the simulation (`Gource::logic` / `Gource::draw`)
//!
//! Entities are addressed by generation-checked ids ([`DirId`], [`FileId`]);
//! `tree.dirs[id]` / `tree.files[id]` give the state, `get` returns `None` for
//! a deleted entity. Containers keep the C++ orders: [`DirTree::dir_map`]
//! iterates like `std::map<std::string, RDirNode*>` (byte order of the path),
//! [`DirTree::file_map`] like `Gource::files`, `children` / `files` of a node
//! like its `std::list`s.
//!
//! Per frame, `Gource::logic` maps onto the tree like this (part 2 owns the
//! user side and the ordering around it):
//!
//! 1. Rotation: [`DirTree::rotate`] / [`DirTree::rotate_about`] (`root->rotate`).
//! 2. Removed files: [`DirTree::take_removed_files`] into a scratch `Vec`, then
//!    for each id: `RUser::fileRemoved` on every user, clear hover/selection,
//!    `file_key.dec(&file.ext)`, and [`DirTree::delete_file`]
//!    (the tree part of `Gource::deleteFile`).
//! 3. `processCommit`: [`DirTree::find_file`] for an existing file, otherwise
//!    [`DirTree::add_file`] (`Gource::addFile`, incl. `max_files` and the
//!    is-a-directory check; the caller passes `tag_seq++` and does
//!    `file_key.inc`). Directory deletes (`D` of a path ending in `/`):
//!    [`DirTree::find_dirs`] from [`DirTree::root`], then
//!    [`DirTree::get_files_recursive`] for each found dir.
//! 4. Actions apply to files through [`DirTree::touch_file`] (all actions),
//!    [`DirTree::files`]`[id].set_file_colour` (modify) and
//!    [`DirTree::remove_file_at`] (remove).
//! 5. `updateBounds` / `interactUsers` dir half: [`DirTree::update_dir_bounds`]
//!    (both places it runs), then [`DirTree::dir_bounds`] for the user tree
//!    depth, the camera and `addUser`'s start position.
//! 6. `interactDirs`: [`DirTree::interact_dirs`] rebuilds the dir quadtree.
//! 7. `updateDirs`: [`DirTree::update_dirs`] (`applyForces` then `logic`).
//!
//! The paused path runs 5 and 6 only. `Gource::reset` is [`DirTree::reset`];
//! `changeColours` is [`DirTree::set_hash_seed`] + [`DirTree::colourize_files`].
//!
//! For drawing: [`DirTree::check_frustum`], [`DirTree::is_visible`] (it
//! caches, hence `&mut`), [`DirTree::file_abs_pos`], [`RFile::label_alpha`] /
//! [`RFile::label_text`] / [`RFile::label_anchor`], and the node fields
//! (`pos`, `spos`, `dir_radius`, `col`, `path_token`, `depth`,
//! `since_last_node_change`, `in_frustum`). Mouse picking uses
//! [`DirTree::dir_quadtree`]`.collect_items_at` (id order replaces the
//! pointer order of the C++ `std::set`) and [`DirTree::file_overlaps`].
//!
//! Deterministic replacements: `applyForceDir`'s `rand()` draws from
//! `overlap_rng`, a [`crate::hash::KeyedRng`] seeded with `Settings::hash_seed` and keyed
//! `"dirnode.applyForceDir"`, rewound by [`DirTree::reset`] (or, with
//! `rand-mode glibc`, from the simulation's shared glibc `rand()`, which
//! the simulation installs and no reset rewinds; see `rand.rs`).
//!
//! Port guards (cases where the C++ is undefined or loops forever): a file
//! name that is empty or does not start with `/` is rejected by
//! [`DirTree::add_file`] (Gource's sources always root paths).

use std::collections::BTreeMap;

use glam::{Vec2, Vec3};

use crate::arena::Arena;
use crate::bounds::Bounds2D;
use crate::file::FILE_DIAMETER;
use crate::frustum::Frustum;
use crate::hash::stream_key;
use crate::quadtree::QuadTree;
use crate::rand::RandStream;
use crate::settings::Settings;

pub(crate) use crate::dirnode::{DirId, DirNode, TourEntry};
pub(crate) use crate::file::{FileId, RFile};

/// `gGourceMaxQuadTreeDepth`.
pub(crate) const MAX_QUADTREE_DEPTH: i32 = 6;

/// Stream key of the `rand()` replacement in `applyForceDir`.
pub(crate) const OVERLAP_RNG_KEY: u64 = stream_key(b"dirnode.applyForceDir");

/// Gource's tree globals.
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct TreeConfig {
    /// `gGourceForceGravity` (adjustable at runtime with `[` / `]`).
    pub(crate) force_gravity: f32,
    /// `gGourceGravity`: toggled by a key in Gource but read nowhere.
    pub(crate) gravity: bool,
    /// `gGourceDirPadding`.
    pub(crate) dir_padding: f32,
    /// `gGourceMinDirSize` (declared, unused by Gource).
    pub(crate) min_dir_size: f32,
    /// `gGourceFileDiameter`.
    pub(crate) file_diameter: f32,
    /// `gGourceNodeDebug`.
    pub(crate) node_debug: bool,
    /// `gGourceMaxQuadTreeDepth`.
    pub(crate) max_quadtree_depth: i32,
}

impl Default for TreeConfig {
    fn default() -> Self {
        TreeConfig {
            force_gravity: 10.0,
            gravity: true,
            dir_padding: 1.5,
            min_dir_size: 15.0,
            file_diameter: FILE_DIAMETER,
            node_debug: false,
            max_quadtree_depth: MAX_QUADTREE_DEPTH,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct DirTree {
    pub(crate) config: TreeConfig,
    pub(crate) dirs: Arena<DirNode>,
    pub(crate) files: Arena<RFile>,
    /// `gGourceDirMap`.
    pub(crate) dir_map: BTreeMap<String, DirId>,
    /// `Gource::files`.
    pub(crate) file_map: BTreeMap<String, FileId>,
    pub(crate) root: DirId,
    /// `gGourceRemovedFiles`.
    pub(crate) removed_files: Vec<FileId>,
    /// `gStringHashSeed` as seen by `vec2Hash` and `colourize`.
    pub(crate) hash_seed: i32,
    pub(crate) overlap_rng: RandStream,
    /// `dirNodeTree`.
    pub(crate) dir_quadtree: QuadTree<DirId>,
    /// `Gource::dir_bounds`.
    pub(crate) dir_bounds: Bounds2D,
    /// `gGourceDirNodeInnerLoops` (reset by the caller each frame).
    pub(crate) dir_inner_loops: i32,
    pub(crate) force_stamp: u32,
    /// [`DirTree::number_tree`]'s stamp, and whether this frame's numbering
    /// covers the tree consistently.
    pub(crate) tour_stamp: u32,
    pub(crate) tour_valid: bool,
    pub(crate) tour: Vec<TourEntry>,
    pub(crate) scratch_tour: Vec<(DirId, usize)>,
    scratch_dirs: Vec<DirId>,
}

impl DirTree {
    pub(crate) fn new(settings: &Settings) -> Self {
        let mut tree = DirTree {
            config: TreeConfig::default(),
            dirs: Arena::default(),
            files: Arena::default(),
            dir_map: BTreeMap::new(),
            file_map: BTreeMap::new(),
            root: DirId::new(u32::MAX, u32::MAX),
            removed_files: Vec::new(),
            hash_seed: settings.hash_seed as i32,
            overlap_rng: RandStream::new(settings.hash_seed as u64, OVERLAP_RNG_KEY, None),
            dir_quadtree: QuadTree::default(),
            dir_bounds: Bounds2D::default(),
            dir_inner_loops: 0,
            force_stamp: 0,
            tour_stamp: 0,
            tour_valid: false,
            tour: Vec::new(),
            scratch_tour: Vec::new(),
            scratch_dirs: Vec::new(),
        };
        tree.root = tree.new_dir("/");
        tree
    }

    /// The file and directory part of `Gource::reset`: delete every node
    /// and file, clear the removed-files queue, the dir quadtree and
    /// `dir_bounds`, and start again from a root at `/`. The hash seed and
    /// the overlap RNG restart from `settings.hash_seed`.
    pub(crate) fn reset(&mut self, settings: &Settings) {
        self.dir_bounds.reset();
        self.removed_files.clear();
        self.dir_quadtree.clear();
        self.dirs.clear();
        self.dir_map.clear();
        self.files.clear();
        self.file_map.clear();
        self.hash_seed = settings.hash_seed as i32;
        self.overlap_rng
            .rewind(settings.hash_seed as u64, OVERLAP_RNG_KEY);
        self.dir_inner_loops = 0;
        self.root = self.new_dir("/");
    }

    pub(crate) fn root(&self) -> DirId {
        self.root
    }

    /// `gGourceDirMap.size()`.
    pub(crate) fn dir_count(&self) -> usize {
        self.dir_map.len()
    }

    /// `files.size()`.
    pub(crate) fn file_count(&self) -> usize {
        self.file_map.len()
    }

    pub(crate) fn find_file(&self, path: &str) -> Option<FileId> {
        self.file_map.get(path).copied()
    }

    pub(crate) fn dir_bounds(&self) -> &Bounds2D {
        &self.dir_bounds
    }

    /// Port-only: the bounds of the visible dirs at or under any of
    /// `prefixes` (dir paths such as `/owner:repo/`, one per line), from the
    /// item bounds [`DirTree::update_dir_bounds`] last computed. Empty
    /// (`first`) when none.
    ///
    /// Runs every frame while a focus is set, so it visits only the dirs under
    /// each prefix: `dir_map` is ordered by path, and they sort together.
    pub(crate) fn dir_bounds_under(&self, prefixes: &str) -> Bounds2D {
        use std::ops::Bound;
        let mut bounds = Bounds2D::default();
        for prefix in prefixes.lines().filter(|p| !p.is_empty()) {
            let under = self
                .dir_map
                .range::<str, _>((Bound::Included(prefix), Bound::Unbounded))
                .take_while(|(path, _)| path.starts_with(prefix));
            for (path, &d) in under {
                if let Some(node) = self.dirs.get(d)
                    && node.visible
                    && crate::dirnode::prefixed_by(path, prefix)
                {
                    bounds.update_bounds(&node.quad_item_bounds);
                }
            }
        }
        bounds
    }

    pub(crate) fn dir_quadtree(&self) -> &QuadTree<DirId> {
        &self.dir_quadtree
    }

    /// `Gource::addFile(cf)`: `None` when `max_files` is reached, when the
    /// path is an existing directory, or (port guard) when it is not rooted.
    pub(crate) fn add_file(
        &mut self,
        filename: &str,
        colour: Vec3,
        tagid: i32,
        settings: &Settings,
    ) -> Option<FileId> {
        // if we already have max files in circulation we cant add any more
        if settings.max_files > 0 && self.file_map.len() >= settings.max_files as usize {
            return None;
        }

        if !filename.starts_with('/') {
            return None;
        }

        // see if this is a directory
        let is_dir = if filename.ends_with('/') {
            self.is_dir(self.root, filename)
        } else {
            let mut file_as_dir = String::with_capacity(filename.len() + 1);
            file_as_dir.push_str(filename);
            file_as_dir.push('/');
            self.is_dir(self.root, &file_as_dir)
        };
        if is_dir {
            return None;
        }

        let file = RFile::new(filename, colour, Vec2::ZERO, tagid, settings);
        let id = self.files.insert(file);

        self.file_map.insert(filename.to_string(), id);

        self.dir_add_file(self.root, id);

        self.root = self.get_root(self.root);

        Some(id)
    }

    /// The tree part of `Gource::deleteFile(file)`: detach it from its dir
    /// (reaping empty nodes), erase it from the file map and free it. The
    /// caller handles users, hover/selection and the file key first.
    pub(crate) fn delete_file(&mut self, f: FileId) -> Option<RFile> {
        if !self.files.contains(f) {
            return None;
        }
        self.dir_remove_file(self.root, f);
        let fullpath = &self.files[f].fullpath;
        self.file_map.remove(fullpath);
        self.files.remove(f)
    }

    /// Move the removed-files queue into `out` (cleared first) and leave the
    /// queue empty, reusing `out`'s allocation for it. Equivalent to
    /// iterating `gGourceRemovedFiles` and clearing it afterwards.
    pub(crate) fn take_removed_files(&mut self, out: &mut Vec<FileId>) {
        out.clear();
        std::mem::swap(out, &mut self.removed_files);
    }

    /// Delete every queued file with no other bookkeeping (no users).
    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn process_removed_files(&mut self, scratch: &mut Vec<FileId>) {
        self.take_removed_files(scratch);
        for &f in scratch.iter() {
            self.delete_file(f);
        }
        scratch.clear();
    }

    /// `RFile::touch(timestamp, colour)`.
    pub(crate) fn touch_file(&mut self, f: FileId, touched_timestamp: i64, colour: Vec3) {
        let file = &mut self.files[f];
        if file.forced_removal || (file.removing && touched_timestamp < file.removed_timestamp) {
            return;
        }

        file.fade_start = -1.0;
        file.removing = false;
        file.removed_timestamp = 0;
        file.last_action = file.pawn.elapsed;
        file.touch_colour = colour;

        // un expire file if touched after being removed
        if file.expired {
            if let Some(pos) = self.removed_files.iter().position(|&x| x == f) {
                self.removed_files.remove(pos);
            }
            self.files[f].expired = false;
        }

        self.files[f].pawn.show_name();
        self.set_file_hidden(f, false);
        if let Some(d) = self.files[f].dir {
            self.file_updated(d, true);
        }
    }

    /// `RFile::remove(timestamp)` (RemoveAction).
    pub(crate) fn remove_file_at(&mut self, f: FileId, removed_timestamp: i64) {
        self.files[f].remove(removed_timestamp);
    }

    /// `RFile::setHidden(hidden)`.
    pub(crate) fn set_file_hidden(&mut self, f: FileId, hidden: bool) {
        let file = &self.files[f];
        if file.pawn.hidden
            && !hidden
            && let Some(d) = file.dir
        {
            self.add_visible(d);
        }
        self.files[f].pawn.hidden = hidden;
    }

    /// `RFile::getAbsolutePos()`.
    pub(crate) fn file_abs_pos(&self, f: FileId) -> Vec2 {
        let file = &self.files[f];
        match file.dir {
            Some(d) => file.pawn.pos + self.dirs[d].pos,
            None => file.pawn.pos,
        }
    }

    /// `RFile::overlaps(pos)`.
    pub(crate) fn file_overlaps(&self, f: FileId, pos: Vec2) -> bool {
        self.files[f].overlaps_at(self.file_abs_pos(f), pos)
    }

    /// `gStringHashSeed = seed` as far as the tree is concerned.
    pub(crate) fn set_hash_seed(&mut self, seed: i32) {
        self.hash_seed = seed;
    }

    /// The file half of `Gource::changeColours`: recolour every file in
    /// `files` map order.
    pub(crate) fn colourize_files(&mut self) {
        let seed = self.hash_seed;
        for &f in self.file_map.values() {
            self.files[f].colourize(seed);
        }
    }

    /// The dir half of `Gource::updateBounds` (also run by `interactUsers`):
    /// refresh the quad bounds of visible dirs in map order and accumulate
    /// `dir_bounds`.
    pub(crate) fn update_dir_bounds(&mut self) {
        self.dir_bounds.reset();

        let mut order = std::mem::take(&mut self.scratch_dirs);
        order.clear();
        order.extend(self.dir_map.values().copied());
        for &d in &order {
            if !self.dirs.contains(d) {
                continue;
            }
            if self.is_visible(d) {
                self.update_quad_item_bounds(d);
                let b = self.dirs[d].quad_item_bounds;
                self.dir_bounds.update_bounds(&b);
            }
        }
        self.scratch_dirs = order;
    }

    /// `Gource::interactDirs`: rebuild the dir quadtree from `dir_bounds`
    /// (grown by 1 on each side; depth 1 until the area exceeds 10000) with
    /// every non-empty dir in map order.
    pub(crate) fn interact_dirs(&mut self) {
        let mut bounds = self.dir_bounds;
        bounds.min -= Vec2::ONE;
        bounds.max += Vec2::ONE;

        let max_depth = if (self.dir_bounds.area() as f64) > 10000.0 {
            self.config.max_quadtree_depth
        } else {
            1
        };

        self.dir_quadtree.reset(bounds, max_depth, 1);

        for &d in self.dir_map.values() {
            let Some(node) = self.dirs.get(d) else {
                continue;
            };
            if !node.is_empty() {
                self.dir_quadtree.add_item(d, node.quad_item_bounds);
            }
        }
    }

    /// `Gource::updateDirs`: `root->applyForces(*dirNodeTree)` then
    /// `root->logic(dt)`.
    pub(crate) fn update_dirs(&mut self, dt: f32, settings: &Settings) {
        let quadtree = std::mem::take(&mut self.dir_quadtree);
        self.number_tree();
        self.apply_forces(self.root, &quadtree);
        self.tour_valid = false;
        self.dir_quadtree = quadtree;
        self.dir_logic(self.root, dt, settings);
    }

    /// `root->rotate(s, c)`.
    pub(crate) fn rotate(&mut self, s: f32, c: f32) {
        self.rotate_dir(self.root, s, c);
    }

    /// `root->rotate(s, c, centre)`.
    pub(crate) fn rotate_about(&mut self, s: f32, c: f32, centre: Vec2) {
        self.rotate_dir_about(self.root, s, c, centre);
    }

    /// `root->checkFrustum(frustum)`.
    pub(crate) fn check_frustum(&mut self, frustum: &Frustum) {
        self.check_frustum_dir(self.root, frustum);
    }
}

#[cfg(test)]
#[path = "tree_tests.rs"]
mod tests;
