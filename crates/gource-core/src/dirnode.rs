// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! `RDirNode` (`dirnode.cpp`): the directory tree, its layout forces and the
//! file ring placement, without the GL drawing.
//!
//! [`DirNode`] holds one node's state. Every `RDirNode` method that follows
//! parent/child/file pointers is a method of [`DirTree`] taking the node's
//! [`DirId`] (the arenas live there), named after the C++ method in snake
//! case: `addFile` is [`DirTree::dir_add_file`], `applyForces` is
//! [`DirTree::apply_forces`], and so on.
//!
//! Faithfulness notes:
//! - `std::list` children and files are `Vec`s; erasing keeps order.
//! - The constructor ignores its `parent` argument (the C++ shadows it and
//!   always passes 0 to `setParent`), so a new node has depth 1 and an empty
//!   path token until `addNode` adopts it.
//! - `path_token_offset` is only refreshed by `adjustPath`, so after the root
//!   renames itself (`modifying root path`) its children keep tokens computed
//!   from the old length, as in Gource.
//! - `isVisible()` caches `visible = true` when a child is visible; it is a
//!   `&mut` method here for that reason.
//! - `DirForceFunctor` dedupes items spanning several quadtree leaves with a
//!   per-node stamp instead of a `std::set` (same membership semantics, no
//!   allocation). Visit order is the quadtree's, which is deterministic.
//! - `applyForceDir` resolves exact overlaps with `rand()`; the port draws
//!   from the tree's [`crate::hash::KeyedRng`] stream instead, y before x:
//!   g++ evaluates the two `rand()` arguments of `vec2(...)` right to left,
//!   and the golden harness (which swaps in the same stream) relies on it.
//! - Mixed float/double expressions (`PI` is a double literal) are evaluated
//!   in `f64` and rounded to `f32` exactly where the C++ assigns to a float.
//! - `calcFileDest` uses `libm::sinf` / `libm::cosf` (identical on native
//!   and wasm32). They are not bit-identical to glibc's: about 0.2% of ring
//!   slots differ by up to 2 ULP in the file offset. Nothing feeds back into
//!   the layout (dir positions, radii, colours, quadtree and frustum results
//!   match), so parity checks against a C++ build must allow ~2 ULP on file
//!   positions.

use glam::{Vec2, Vec3, Vec4};

use crate::arena::Id;
use crate::bounds::Bounds2D;
use crate::file::FileId;
use crate::frustum::Frustum;
use crate::hash::vec2_hash;
use crate::quadtree::QuadTree;
use crate::settings::Settings;
use crate::tree::DirTree;
use crate::vectors::{PI, cpp_max, cpp_min, length, length2, normalise2, rotate_vec2};

pub(crate) type DirId = Id<DirNode>;

/// One node's numbers in [`DirTree::number_tree`]'s depth-first walk,
/// indexed by arena slot.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub(crate) struct TourEntry {
    stamp: u32,
    generation: u32,
    enter: u32,
    exit: u32,
}

/// Largest elasticity filter gain `dt * elasticity` (see
/// [`DirTree::move_dir`]).
const MAX_ELASTIC_GAIN: f32 = 0.15;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DirNode {
    /// Absolute path, always ending in `/`.
    pub(crate) abspath: String,
    /// Display name: the path below the parent's token offset.
    pub(crate) path_token: String,
    pub(crate) path_token_offset: usize,

    pub(crate) parent: Option<DirId>,
    pub(crate) children: Vec<DirId>,
    pub(crate) files: Vec<FileId>,

    /// `getColour()`.
    pub(crate) col: Vec4,

    /// Spline control point of the edge to the parent.
    pub(crate) spos: Vec2,
    pub(crate) pos: Vec2,
    pub(crate) vel: Vec2,
    pub(crate) accel: Vec2,
    pub(crate) prev_accel: Vec2,

    pub(crate) dir_area: f32,

    pub(crate) visible: bool,
    pub(crate) in_frustum: bool,
    pub(crate) position_initialized: bool,

    pub(crate) since_node_visible: f32,
    pub(crate) since_last_file_change: f32,
    pub(crate) since_last_node_change: f32,

    pub(crate) file_area: f32,
    pub(crate) dir_radius: f32,
    pub(crate) parent_radius: f32,

    pub(crate) depth: i32,

    pub(crate) visible_count: i32,

    pub(crate) node_normal: Vec2,

    /// `QuadItem::quadItemBounds`, refreshed by `updateQuadItemBounds` only
    /// while the node is visible.
    pub(crate) quad_item_bounds: Bounds2D,

    /// `DirForceFunctor::seen` membership stamp.
    pub(crate) force_seen: u32,
}

impl DirNode {
    pub(crate) fn no_dirs(&self) -> bool {
        self.children.is_empty()
    }

    pub(crate) fn no_files(&self) -> bool {
        self.files.is_empty()
    }

    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn dir_count(&self) -> usize {
        self.children.len()
    }

    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn file_count(&self) -> usize {
        self.files.len()
    }

    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn visible_file_count(&self) -> i32 {
        self.visible_count
    }

    /// `empty()`: no visible files and no child dirs.
    pub(crate) fn is_empty(&self) -> bool {
        self.visible_count == 0 && self.no_dirs()
    }

    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn radius(&self) -> f32 {
        self.dir_radius
    }

    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn path(&self) -> &str {
        &self.abspath
    }

    /// `prefixedBy(path)`: `path` (with a `/` appended if missing) is a
    /// prefix of this node's path.
    pub(crate) fn prefixed_by(&self, path: &str) -> bool {
        prefixed_by(&self.abspath, path)
    }

    /// `commonPathPrefix(str)`: the longest prefix of `s` shared with this
    /// node's path that ends in `/` (empty if none).
    pub(crate) fn common_path_prefix<'a>(&self, s: &'a str) -> &'a str {
        common_path_prefix(&self.abspath, s)
    }

    /// The alpha `drawDirName` draws `path_token` with, or `None` when it
    /// draws nothing: not for the root, with `--hide dirnames`, below
    /// `--dir-name-depth`, or more than 5 s after the last change (unless
    /// `--highlight-dirs`). `drawNames` additionally requires `isVisible()`
    /// (and skips the call with `hide_dirnames`); the caller checks that.
    pub(crate) fn name_alpha(&self, settings: &Settings) -> Option<f32> {
        // the root has no name
        self.parent?;
        if settings.hide_dirnames {
            return None;
        }
        if settings.dir_name_depth > 0 && settings.dir_name_depth < (self.depth - 1) {
            return None;
        }
        if !settings.highlight_dirs && (self.since_last_node_change as f64) > 5.0 {
            return None;
        }
        Some(if settings.highlight_dirs {
            1.0
        } else {
            cpp_max(0.0, 5.0 - self.since_last_node_change) / 5.0
        })
    }
}

pub(crate) fn prefixed_by(abspath: &str, path: &str) -> bool {
    if path.is_empty() {
        return false;
    }
    if !path.ends_with('/') {
        abspath.starts_with(path) && abspath.as_bytes().get(path.len()) == Some(&b'/')
    } else {
        abspath.starts_with(path)
    }
}

pub(crate) fn common_path_prefix<'a>(abspath: &str, s: &'a str) -> &'a str {
    let a = abspath.as_bytes();
    let b = s.as_bytes();
    let mut c = 0;
    let mut slash: Option<usize> = None;
    while c < a.len() && c < b.len() && a[c] == b[c] {
        if a[c] == b'/' {
            slash = Some(c);
        }
        c += 1;
    }
    match slash {
        None => "",
        Some(slash) => &s[..slash + 1],
    }
}

/// `calcColour()` brightness: 1 right after a change, down to 0.6 over 3 s.
fn colour_brightness(since_last_node_change: f32) -> f32 {
    cpp_max(0.6, 1.0 - cpp_min(1.0, since_last_node_change / 3.0))
}

/// `calcFileDest(max_files, file_no)`: unit direction of slot `file_no` on
/// a ring of `max_files` slots.
pub(crate) fn calc_file_dest(max_files: i32, file_no: i32) -> Vec2 {
    let arc = (1.0 / (max_files as f32) as f64) as f32;
    let frac = (arc as f64 * 0.5 + (arc * file_no as f32) as f64) as f32;
    let angle = (frac as f64 * PI * 2.0) as f32;
    Vec2::new(libm::sinf(angle), libm::cosf(angle))
}

/// `std::string::substr(pos, count)` on bytes; `pos` past the end gives an
/// empty string where the C++ would throw.
fn substr(s: &str, pos: usize, count: usize) -> String {
    let bytes = s.as_bytes();
    if pos > bytes.len() {
        return String::new();
    }
    let end = pos + count.min(bytes.len() - pos);
    String::from_utf8_lossy(&bytes[pos..end]).into_owned()
}

impl DirTree {
    /// `new RDirNode(parent, abspath)`. The parent argument is ignored by the
    /// C++ constructor, so it is not taken here; `add_node` adopts the node.
    pub(crate) fn new_dir(&mut self, abspath: &str) -> DirId {
        let padded_file_radius = (self.config.file_diameter as f64 * 0.5) as f32;
        let file_area = ((padded_file_radius * padded_file_radius) as f64 * PI) as f32;

        let id = self.dirs.insert(DirNode {
            abspath: String::new(),
            path_token: String::new(),
            path_token_offset: 0,
            parent: None,
            children: Vec::new(),
            files: Vec::new(),
            col: Vec4::ZERO,
            spos: Vec2::ZERO,
            pos: Vec2::ZERO,
            vel: Vec2::ZERO,
            accel: Vec2::ZERO,
            prev_accel: Vec2::ZERO,
            dir_area: 0.0,
            visible: false,
            in_frustum: false,
            position_initialized: false,
            since_node_visible: 0.0,
            since_last_file_change: 0.0,
            since_last_node_change: 0.0,
            file_area,
            dir_radius: 0.0,
            parent_radius: 0.0,
            depth: 0,
            visible_count: 0,
            node_normal: Vec2::ZERO,
            quad_item_bounds: Bounds2D::default(),
            force_seen: 0,
        });

        self.change_path(id, abspath);
        self.set_parent(id, None);
        self.calc_radius(id);
        self.calc_colour(id);
        id
    }

    /// `changePath(abspath)`: re-key the node in the dir map.
    pub(crate) fn change_path(&mut self, d: DirId, abspath: &str) {
        let old = std::mem::take(&mut self.dirs[d].abspath);
        self.dir_map.remove(&old);

        let mut path = old;
        path.clear();
        path.push_str(abspath);
        if path.is_empty() || !path.ends_with('/') {
            path.push('/');
        }
        self.dir_map.insert(path.clone(), d);
        self.dirs[d].abspath = path;
    }

    /// `~RDirNode()`: delete the subtree and erase each path from the map.
    pub(crate) fn delete_dir(&mut self, d: DirId) {
        let Some(node) = self.dirs.remove(d) else {
            return;
        };
        for child in node.children {
            self.delete_dir(child);
        }
        self.dir_map.remove(&node.abspath);
    }

    /// `fileUpdated(userInitiated)`.
    pub(crate) fn file_updated(&mut self, d: DirId, user_initiated: bool) {
        self.calc_radius(d);
        self.dirs[d].since_last_file_change = 0.0;
        self.node_updated(d, user_initiated);
    }

    /// `nodeUpdated(userInitiated)`; the parent chain is updated with
    /// `userInitiated = true`.
    pub(crate) fn node_updated(&mut self, d: DirId, user_initiated: bool) {
        let mut d = d;
        let mut user_initiated = user_initiated;
        loop {
            if user_initiated {
                self.dirs[d].since_last_node_change = 0.0;
            }
            self.calc_radius(d);
            self.update_file_positions(d);
            let node = &mut self.dirs[d];
            if node.visible && node.no_dirs() && node.no_files() {
                node.visible = false;
            }
            match node.parent {
                Some(p) => {
                    d = p;
                    user_initiated = true;
                }
                None => break,
            }
        }
    }

    /// `rotate(s, c)` about the origin, recursively.
    pub(crate) fn rotate_dir(&mut self, d: DirId, s: f32, c: f32) {
        let node = &mut self.dirs[d];
        node.pos = rotate_vec2(node.pos, s, c);
        node.spos = rotate_vec2(node.spos, s, c);
        for i in 0..self.dirs[d].children.len() {
            let child = self.dirs[d].children[i];
            self.rotate_dir(child, s, c);
        }
    }

    /// `rotate(s, c, centre)`, recursively.
    pub(crate) fn rotate_dir_about(&mut self, d: DirId, s: f32, c: f32, centre: Vec2) {
        let node = &mut self.dirs[d];
        node.pos = rotate_vec2(node.pos - centre, s, c) + centre;
        node.spos = rotate_vec2(node.spos - centre, s, c) + centre;
        for i in 0..self.dirs[d].children.len() {
            let child = self.dirs[d].children[i];
            self.rotate_dir_about(child, s, c, centre);
        }
    }

    /// `isDir(path)`.
    pub(crate) fn is_dir(&self, d: DirId, path: &str) -> bool {
        let node = &self.dirs[d];
        if node.prefixed_by(path) {
            return true;
        }
        if !path.starts_with(node.abspath.as_str()) {
            return false;
        }
        node.children.iter().any(|&child| self.is_dir(child, path))
    }

    /// `findDirs(path, dirs)`: the nodes closest to `d` prefixed by `path`,
    /// appended to `out` in tree order.
    pub(crate) fn find_dirs(&self, d: DirId, path: &str, out: &mut Vec<DirId>) {
        let node = &self.dirs[d];
        if node.prefixed_by(path) {
            out.push(d);
            return;
        }
        for &child in &node.children {
            self.find_dirs(child, path, out);
        }
    }

    /// `getFilesRecursive(files)` into a cleared `out`, in the C++ order:
    /// every node's files are inserted at the front of the list, so the
    /// result lists the last-visited node's files first.
    pub(crate) fn get_files_recursive(&self, d: DirId, out: &mut Vec<FileId>) {
        out.clear();
        self.push_files_reversed(d, out);
        out.reverse();
    }

    fn push_files_reversed(&self, d: DirId, out: &mut Vec<FileId>) {
        let node = &self.dirs[d];
        out.extend(node.files.iter().rev().copied());
        for &child in &node.children {
            self.push_files_reversed(child, out);
        }
    }

    /// `adjustPath()`.
    pub(crate) fn adjust_path(&mut self, d: DirId) {
        let parent_token_offset = self.dirs[d].parent.map(|p| self.dirs[p].path_token_offset);
        let node = &mut self.dirs[d];
        node.path_token_offset = node.abspath.len();
        if let Some(offset) = parent_token_offset {
            let count = node.abspath.len().wrapping_sub(offset).wrapping_sub(1);
            node.path_token = substr(&node.abspath, offset, count);
            node.path_token_offset = node.abspath.len();
        }
    }

    /// `setParent(parent)`.
    pub(crate) fn set_parent(&mut self, d: DirId, parent: Option<DirId>) {
        if parent.is_some() && self.dirs[d].parent == parent {
            return;
        }
        self.dirs[d].parent = parent;
        self.adjust_path(d);
        self.adjust_depth(d);
    }

    /// `adjustDepth()`, recursively.
    pub(crate) fn adjust_depth(&mut self, d: DirId) {
        let depth = match self.dirs[d].parent {
            None => 1,
            Some(p) => self.dirs[p].depth + 1,
        };
        self.dirs[d].depth = depth;
        for i in 0..self.dirs[d].children.len() {
            let child = self.dirs[d].children[i];
            self.adjust_depth(child);
        }
    }

    /// `addNode(node)`: adopt `node`, first moving every child it prefixes
    /// under it.
    pub(crate) fn add_node(&mut self, d: DirId, node: DirId) {
        let mut i = 0;
        while i < self.dirs[d].children.len() {
            let child = self.dirs[d].children[i];
            let matches = prefixed_by(&self.dirs[child].abspath, &self.dirs[node].abspath);
            if matches {
                self.dirs[d].children.remove(i);
                self.add_node(node, child);
            } else {
                i += 1;
            }
        }

        self.dirs[d].children.push(node);
        self.set_parent(node, Some(d));

        self.node_updated(d, false);
    }

    /// `getRoot()`.
    pub(crate) fn get_root(&self, d: DirId) -> DirId {
        let mut d = d;
        while let Some(p) = self.dirs[d].parent {
            d = p;
        }
        d
    }

    /// `removeFile(f)`: detach the file from the node holding it and reap
    /// child nodes left with no files and no dirs. The file itself is not
    /// deleted.
    pub(crate) fn dir_remove_file(&mut self, d: DirId, f: FileId) -> bool {
        let (matches, is_this_dir) = {
            let fpath = &self.files[f].path;
            let abspath = &self.dirs[d].abspath;
            (fpath.starts_with(abspath.as_str()), fpath == abspath)
        };
        if !matches {
            return false;
        }

        if is_this_dir {
            let hidden = self.files[f].is_hidden();
            let node = &mut self.dirs[d];
            if let Some(pos) = node.files.iter().position(|&x| x == f) {
                node.files.remove(pos);
                if !hidden {
                    node.visible_count -= 1;
                }
                self.file_updated(d, false);
                return true;
            }
            return false;
        }

        for i in 0..self.dirs[d].children.len() {
            let node = self.dirs[d].children[i];
            if self.dir_remove_file(node, f) {
                let child = &self.dirs[node];
                if child.no_files() && child.no_dirs() {
                    self.dirs[d].children.remove(i);
                    self.delete_dir(node);
                    self.node_updated(d, false);
                }
                return true;
            }
        }

        false
    }

    /// `addVisible()`.
    pub(crate) fn add_visible(&mut self, d: DirId) {
        let node = &mut self.dirs[d];
        node.visible_count += 1;
        node.visible = true;
    }

    /// `isVisible()`: caches `visible = true` when any child is visible.
    pub(crate) fn is_visible(&mut self, d: DirId) -> bool {
        if self.dirs[d].visible {
            return true;
        }
        for i in 0..self.dirs[d].children.len() {
            let child = self.dirs[d].children[i];
            if self.is_visible(child) {
                self.dirs[d].visible = true;
                return true;
            }
        }
        false
    }

    /// `addFile(f)`: place the file in the node matching its path, creating,
    /// splitting and re-rooting nodes as needed. When `d` is the root and the
    /// file lies outside it, a new parent is forked; the caller walks the
    /// root pointer upward afterwards (`Gource::addFile`).
    pub(crate) fn dir_add_file(&mut self, d: DirId, f: FileId) -> bool {
        // Owned copy: the tree is restructured below while the path is used.
        let fpath = self.files[f].path.clone();

        // doesnt match this path at all
        if !fpath.starts_with(self.dirs[d].abspath.as_str()) {
            if self.dirs[d].parent.is_some() {
                return false;
            }

            // this is the root node: fork it
            let mut common = self.dirs[d].common_path_prefix(&fpath).to_string();
            if common.is_empty() {
                common.push('/');
            }

            let newparent = self.new_dir(&common);
            self.add_node(newparent, d);
            return self.dir_add_file(newparent, f);
        }

        // simply change path of node and add this to it
        {
            let node = &self.dirs[d];
            if node.parent.is_none()
                && node.abspath == "/"
                && fpath != node.abspath
                && node.no_files()
                && node.no_dirs()
            {
                self.change_path(d, &fpath);
            }
        }

        // is this dir - add to this node
        if fpath == self.dirs[d].abspath {
            let hidden = self.files[f].is_hidden();
            let node = &mut self.dirs[d];
            node.files.push(f);
            if !hidden {
                node.visible_count += 1;
            }
            self.files[f].dir = Some(d);
            self.file_updated(d, false);
            return true;
        }

        let mut added = false;

        // does this belong to one of the children ?
        for i in 0..self.dirs[d].children.len() {
            let child = self.dirs[d].children[i];
            added = self.dir_add_file(child, f);
            if added {
                break;
            }
        }

        if added && self.dirs[d].parent.is_some() {
            return true;
        }

        // a file in this directory whose fullpath prefixes this file's path is
        // actually a directory: force-remove it (the root does this even when
        // the file went to a child).
        for i in 0..self.dirs[d].files.len() {
            let other = self.dirs[d].files[i];
            if fpath.starts_with(self.files[other].fullpath.as_str()) {
                self.files[other].remove_forced();
                break;
            }
        }

        if added {
            return true;
        }

        // add new child, add it to that
        let node = self.new_dir(&fpath);
        self.dir_add_file(node, f);
        self.add_node(d, node);

        // do we have dir nodes with a common path element greater than
        // abspath? if so create another node and move those nodes there
        let mut commonpath = String::new();
        let mut common_pos = Vec2::ZERO;
        {
            let abspath_len = self.dirs[d].abspath.len();
            for &child in &self.dirs[d].children {
                let common = self.dirs[child].common_path_prefix(&fpath);
                if common.len() > abspath_len && common != fpath {
                    commonpath = common.to_string();
                    common_pos = self.dirs[child].pos;
                    break;
                }
            }
        }

        // redistribute to new common node
        if commonpath.len() > self.dirs[d].abspath.len() {
            let cnode = self.new_dir(&commonpath);
            self.dirs[cnode].pos = common_pos;

            let mut i = 0;
            while i < self.dirs[d].children.len() {
                let child = self.dirs[d].children[i];
                if self.dirs[child].prefixed_by(&commonpath) {
                    self.dirs[d].children.remove(i);
                    self.add_node(cnode, child);
                    continue;
                }
                i += 1;
            }

            self.add_node(d, cnode);
        }

        true
    }

    /// `averageFileColour()`.
    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn average_file_colour(&self, d: DirId) -> Vec3 {
        let node = &self.dirs[d];
        let mut av = Vec3::ZERO;
        let mut count = 0;

        for &f in &node.files {
            let file = &self.files[f];
            if file.is_hidden() {
                continue;
            }
            av += file.colour();
            count += 1;
        }

        if count > 0 {
            av *= 1.0 / count as f32;
        }

        count = 0;

        for &child in &node.children {
            av += self.average_file_colour(child);
            count += 1;
        }

        if count > 0 {
            av *= 1.0 / count as f32;
        }

        av
    }

    /// `calcColour()`: brighter for 3 s after a change, mixed with the
    /// visible files' colours and alphas.
    pub(crate) fn calc_colour(&mut self, d: DirId) {
        let node = &self.dirs[d];

        let brightness = colour_brightness(node.since_last_node_change);

        let mut col = Vec4::new(brightness, brightness, brightness, 1.0);

        let mut fcount = 0;

        for &f in &node.files {
            let file = &self.files[f];
            if file.is_hidden() {
                continue;
            }
            let filecol = file.colour() * brightness;
            let a = file.alpha();
            col += Vec4::new(filecol.x, filecol.y, filecol.z, a);
            fcount += 1;
        }

        col /= (fcount as f64 + 1.0) as f32;

        self.dirs[d].col = col;
    }

    /// `calcRadius()`.
    pub(crate) fn calc_radius(&mut self, d: DirId) {
        let node = &self.dirs[d];
        let total_file_area = node.file_area * node.visible_count as f32;

        let mut dir_area = total_file_area;
        for &child in &node.children {
            dir_area += self.dirs[child].dir_area;
        }

        let padding = self.config.dir_padding;
        let node = &mut self.dirs[d];
        node.dir_area = dir_area;
        node.dir_radius = cpp_max(1.0, dir_area.sqrt()) * padding;
        node.parent_radius = cpp_max(1.0, total_file_area.sqrt() * padding);
    }

    /// `distanceToParent()`. Requires a parent.
    pub(crate) fn distance_to_parent(&self, d: DirId) -> f32 {
        let node = &self.dirs[d];
        let parent = &self.dirs[node.parent.expect("distance_to_parent of the root")];
        let posd = length(parent.pos - node.pos);
        posd - (node.dir_radius + parent.parent_radius)
    }

    /// `applyForceDir(node)`: push `d` out of an overlapping `node`.
    pub(crate) fn apply_force_dir(&mut self, d: DirId, node: DirId) {
        if node == d {
            return;
        }

        let this = &self.dirs[d];
        let other = &self.dirs[node];

        let dir = other.pos - this.pos;

        let posd2 = length2(dir);
        let myradius = this.dir_radius;
        let your_radius = other.dir_radius;

        let sumradius = myradius + your_radius;

        let distance2 = posd2 - sumradius * sumradius;

        if distance2 > 0.0 {
            return;
        }

        let posd = posd2.sqrt();

        let distance = posd - myradius - your_radius;

        // resolve overlap (g++ evaluates the y argument first)
        if (posd as f64) < 0.00001 {
            let y = (self.overlap_rng.rand() % 100) - 50;
            let x = (self.overlap_rng.rand() % 100) - 50;
            self.dirs[d].accel += normalise2(Vec2::new(x as f32, y as f32));
            return;
        }

        self.dirs[d].accel += distance * normalise2(dir);
    }

    /// `isParent(node)`: `node` is an ancestor of `d`.
    pub(crate) fn is_parent(&self, d: DirId, node: DirId) -> bool {
        let mut cur = self.dirs[d].parent;
        while let Some(p) = cur {
            if p == node {
                return true;
            }
            cur = self.dirs[p].parent;
        }
        false
    }

    /// Number every node reachable from the root in depth-first order, so
    /// that [`DirTree::is_parent`] is two comparisons instead of a walk up
    /// the parent chain (the walk per quadtree pair made a chain of nested
    /// directories cost O(depth³) per frame). If the children lists and
    /// parent links ever disagree, the numbering is dropped and every test
    /// walks the chain as before.
    pub(crate) fn number_tree(&mut self) {
        self.tour_stamp = self.tour_stamp.wrapping_add(1);
        if self.tour_stamp == 0 {
            self.tour.fill(TourEntry::default());
            self.tour_stamp = 1;
        }
        let stamp = self.tour_stamp;
        self.tour_valid = false;
        let root = self.root;
        if !self.dirs.contains(root) {
            return;
        }
        let mut counter = 0u32;
        self.enter(root, stamp, &mut counter);
        let mut stack = std::mem::take(&mut self.scratch_tour);
        stack.clear();
        stack.push((root, 0));
        let mut valid = true;
        while let Some(top) = stack.last_mut() {
            let (n, i) = *top;
            if let Some(&c) = self.dirs[n].children.get(i) {
                top.1 += 1;
                let fresh = self
                    .dirs
                    .get(c)
                    .is_some_and(|child| child.parent == Some(n))
                    && self.tour_entry(c).is_none();
                if !fresh {
                    valid = false;
                    break;
                }
                self.enter(c, stamp, &mut counter);
                stack.push((c, 0));
            } else {
                self.tour[n.index as usize].exit = counter;
                counter += 1;
                stack.pop();
            }
        }
        stack.clear();
        self.scratch_tour = stack;
        self.tour_valid = valid;
    }

    fn enter(&mut self, d: DirId, stamp: u32, counter: &mut u32) {
        let i = d.index as usize;
        if self.tour.len() <= i {
            self.tour.resize(i + 1, TourEntry::default());
        }
        self.tour[i] = TourEntry {
            stamp,
            generation: d.generation,
            enter: *counter,
            exit: u32::MAX,
        };
        *counter += 1;
    }

    /// `d`'s numbering in the current walk.
    fn tour_entry(&self, d: DirId) -> Option<&TourEntry> {
        self.tour
            .get(d.index as usize)
            .filter(|t| t.stamp == self.tour_stamp && t.generation == d.generation)
    }

    /// [`DirTree::is_parent`] through this frame's numbering when both nodes
    /// have one (same answer, constant time).
    pub(crate) fn is_parent_numbered(&self, d: DirId, node: DirId) -> bool {
        if self.tour_valid
            && let (Some(a), Some(b)) = (self.tour_entry(d), self.tour_entry(node))
        {
            return b.enter < a.enter && a.exit < b.exit;
        }
        self.is_parent(d, node)
    }

    /// `applyForces(quadtree)`: children first, then overlap repulsion from
    /// non-related nodes (`DirForceFunctor`), the parent overlap, gravity
    /// toward the parent's rim, the push along the grandparent edge and the
    /// sibling repulsion.
    pub(crate) fn apply_forces(&mut self, d: DirId, quadtree: &QuadTree<DirId>) {
        for i in 0..self.dirs[d].children.len() {
            let child = self.dirs[d].children[i];
            self.apply_forces(child, quadtree);
        }

        let Some(parent) = self.dirs[d].parent else {
            return;
        };

        // DirForceFunctor
        self.force_stamp = self.force_stamp.wrapping_add(1);
        if self.force_stamp == 0 {
            for (_, node) in self.dirs.iter_mut() {
                node.force_seen = 0;
            }
            self.force_stamp = 1;
        }
        let stamp = self.force_stamp;
        let bounds = self.dirs[d].quad_item_bounds;
        let mut loop_count = 0;
        quadtree.visit_items_in_bounds(&bounds, &mut |other, node_count| {
            if other == d {
                return;
            }
            let Some(o) = self.dirs.get_mut(other) else {
                return;
            };
            // An item in several leaves is visited once per leaf. The
            // relation tests below depend only on (d, other), so judging it
            // on the first visit and skipping the repeats gives the C++'s
            // result (which tests first and dedupes after) without walking
            // the ancestor chains again for every leaf.
            if node_count != 1 {
                if o.force_seen == stamp {
                    return;
                }
                o.force_seen = stamp;
            }
            if other == parent {
                return;
            }
            if o.parent == Some(d) {
                return;
            }
            if self.is_parent_numbered(d, other) {
                return;
            }
            if self.is_parent_numbered(other, d) {
                return;
            }
            self.apply_force_dir(d, other);
            loop_count += 1;
        });
        self.dir_inner_loops += loop_count;

        // always call on parent no matter how far away
        self.apply_force_dir(d, parent);

        // pull towards parent
        let parent_dist = self.distance_to_parent(d);
        let gravity = self.config.force_gravity;
        {
            let parent_pos = self.dirs[parent].pos;
            let node = &mut self.dirs[d];
            node.accel += gravity * parent_dist * normalise2(parent_pos - node.pos);
        }

        // push along the grandparent -> parent edge
        if let Some(pparent) = self.dirs[parent].parent {
            let parent_pos = self.dirs[parent].pos;
            let parent_radius = self.dirs[parent].dir_radius;
            let parent_edge = parent_pos - self.dirs[pparent].pos;
            let parent_edge_normal = normalise2(parent_edge);
            let node = &mut self.dirs[d];
            let dest =
                (parent_pos + (parent_radius + node.dir_radius) * parent_edge_normal) - node.pos;
            node.accel += dest;
        }

        // repulse from visible siblings (O(siblings²) per frame for a wide
        // directory, as in Gource: the float sum is order dependent, so it
        // stays exact rather than approximated)
        if !self.dirs[parent].children.is_empty() {
            let mut sib_accel = Vec2::ZERO;
            let mut visible = 1;
            let pos = self.dirs[d].pos;

            for i in 0..self.dirs[parent].children.len() {
                let node = self.dirs[parent].children[i];
                if node == d {
                    continue;
                }
                if !self.is_visible(node) {
                    continue;
                }
                visible += 1;
                sib_accel -= normalise2(self.dirs[node].pos - pos);
            }

            // parent circumference divided by the number of visible child nodes
            if visible > 1 {
                let parent_radius = self.dirs[parent].dir_radius;
                let slice_size =
                    ((parent_radius as f64 * PI) / ((visible + 1) as f32) as f64) as f32;
                sib_accel *= slice_size;
                self.dirs[d].accel += sib_accel;
            }
        }
    }

    /// `totalFileCount()`: this node's visible files plus those of its
    /// direct children (not recursive, as in the C++).
    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn total_file_count(&self, d: DirId) -> i32 {
        let node = &self.dirs[d];
        node.visible_count
            + node
                .children
                .iter()
                .map(|&c| self.dirs[c].visible_count)
                .sum::<i32>()
    }

    /// `totalDirCount()`.
    #[allow(
        dead_code,
        reason = "kept for parity with the C++ API; the simulation does not call it"
    )]
    pub(crate) fn total_dir_count(&self, d: DirId) -> i32 {
        1 + self.dirs[d]
            .children
            .iter()
            .map(|&c| self.total_dir_count(c))
            .sum::<i32>()
    }

    /// `updateSplinePoint(dt)`.
    pub(crate) fn update_spline_point(&mut self, d: DirId, dt: f32) {
        let Some(parent) = self.dirs[d].parent else {
            return;
        };
        let parent_pos = self.dirs[parent].pos;
        let node = &mut self.dirs[d];

        let td = (parent_pos - node.pos) * 0.5;

        let mid = node.pos + td;

        let delta = mid - node.spos;

        // dont let spos get more than half the length of the distance behind
        if length2(delta) > length2(td) {
            node.spos += normalise2(delta) * (length(delta) - length(td));
        }

        node.spos += delta * cpp_min(1.0, dt * 2.0);
    }

    /// `setInitialPosition()`: next to the parent, offset by the path hash
    /// (blended with the grandparent edge direction).
    pub(crate) fn set_initial_position(&mut self, d: DirId) {
        let parent = self.dirs[d]
            .parent
            .expect("set_initial_position of the root");
        let parent_p = self.dirs[parent].parent;
        let parent_pos = self.dirs[parent].pos;
        let hash = vec2_hash(self.dirs[d].abspath.as_bytes(), self.hash_seed);

        let mut pos = parent_pos;

        if let Some(pp) = parent_p {
            let pp_pos = self.dirs[pp].pos;
            pos += normalise2(normalise2(parent_pos - pp_pos) * 2.0 + hash);
        } else {
            pos += hash;
        }

        let node = &mut self.dirs[d];
        node.pos = pos;
        node.spos = pos - (parent_pos - pos) * 0.5;
        node.position_initialized = true;
    }

    /// `move(dt)`: integrate the accumulated force (plus the optional
    /// elasticity term) and clear it. The root never moves.
    pub(crate) fn move_dir(&mut self, d: DirId, dt: f32, elasticity: f32) {
        if self.dirs[d].parent.is_none() {
            return;
        }

        if !self.dirs[d].is_empty() && !self.dirs[d].position_initialized {
            self.set_initial_position(d);
        }

        let node = &mut self.dirs[d];
        let old_pos = node.pos;
        node.pos += node.accel * dt;

        if (elasticity as f64) > 0.0 {
            let diff = node.accel - node.prev_accel;
            // Port guard: the filter state feeds back through the layout
            // forces (gravity alone is 10 per unit of distance), and the
            // loop diverges once the gain passes about
            // (4 - 2k dt) / (4 + k - 2k dt) for stiffness k (about 0.17 for
            // k = 12 at the longest tick, 1/15 s). Gource has no upper bound
            // on `--elasticity`; gains up to the cap (elasticity 9 at
            // 1/60 s, 2.25 at time-scale 4, so the panel's whole range) are
            // Gource's.
            let m = cpp_min(dt * elasticity, MAX_ELASTIC_GAIN);
            let accel3 = node.prev_accel * (1.0 - m) + diff * m;
            node.pos += accel3;
            node.prev_accel = accel3;
        }

        // Port guard: a layout that still blew up (huge forces from many
        // overlapping nodes) keeps its last finite position instead of
        // turning NaN for good.
        if !node.pos.is_finite() {
            node.pos = old_pos;
            node.prev_accel = Vec2::ZERO;
        }

        node.accel = Vec2::ZERO;
    }

    /// `updateFilePositions()`: concentric rings, one file in the centre,
    /// ring `k` at `k * file_diameter` holding `floor((k + 1) * PI)` files.
    pub(crate) fn update_file_positions(&mut self, d: DirId) {
        let mut max_files: i32 = 1;
        let mut diameter: i32 = 1;
        let mut file_no: i32 = 0;
        let mut dist: f32 = 0.0;

        let file_diameter = self.config.file_diameter;
        let node = &self.dirs[d];
        let mut files_left = node.visible_count;

        for &f in &node.files {
            let file = &mut self.files[f];

            if file.is_hidden() {
                file.dest = Vec2::ZERO;
                file.distance = 0.0;
                continue;
            }

            file.dest = calc_file_dest(max_files, file_no);
            file.distance = dist;

            files_left -= 1;
            file_no += 1;

            if file_no >= max_files {
                diameter += 1;
                dist += file_diameter;
                max_files = (1.0f64).max(diameter as f64 * PI) as i32;

                if files_left < max_files {
                    max_files = files_left;
                }

                file_no = 0;
            }
        }
    }

    /// `logic(dt)`: move, spline point, node normal, files, children, colour
    /// and tickers, in that order.
    pub(crate) fn dir_logic(&mut self, d: DirId, dt: f32, settings: &Settings) {
        self.move_dir(d, dt, settings.elasticity);
        self.update_spline_point(d, dt);

        if let Some(parent) = self.dirs[d].parent {
            let parent_pos = self.dirs[parent].pos;
            let node = &mut self.dirs[d];
            node.node_normal = normalise2(node.pos - parent_pos);
        }

        // calcColour() runs after the children in the C++; its sums are
        // taken here instead, right after each file's logic. Nothing the
        // children do touches this node's files or its change timer, so the
        // floats and their order are the same, without a second pass over
        // the files once the children's subtrees have evicted them from the
        // cache.
        let brightness = colour_brightness(self.dirs[d].since_last_node_change);
        let mut col = Vec4::new(brightness, brightness, brightness, 1.0);
        let mut fcount = 0;

        for i in 0..self.dirs[d].files.len() {
            let f = self.dirs[d].files[i];
            let file = &mut self.files[f];
            let expired = file.logic(dt, settings);
            if !file.is_hidden() {
                let filecol = file.colour() * brightness;
                col += Vec4::new(filecol.x, filecol.y, filecol.z, file.alpha());
                fcount += 1;
            }
            // Gource scans gGourceRemovedFiles before pushing; the scan can
            // never match: `expired` turns true only here, and touch_file and
            // reset clear the queue entry together with the flag, so each
            // file is queued at most once. Skipping it keeps an expiry burst
            // linear.
            if expired {
                debug_assert!(!self.removed_files.contains(&f));
                self.removed_files.push(f);
            }
        }

        for i in 0..self.dirs[d].children.len() {
            let child = self.dirs[d].children[i];
            self.dir_logic(child, dt, settings);
        }

        // calcColour()
        self.dirs[d].col = col / ((fcount as f64 + 1.0) as f32);

        let node = &mut self.dirs[d];
        if node.visible {
            node.since_node_visible += dt;
        }
        node.since_last_file_change += dt;
        node.since_last_node_change += dt;
    }

    /// `checkFrustum(frustum)`, recursively.
    pub(crate) fn check_frustum_dir(&mut self, d: DirId, frustum: &Frustum) {
        let node = &mut self.dirs[d];
        node.in_frustum = frustum.intersects(&node.quad_item_bounds, 0.0);
        for i in 0..self.dirs[d].children.len() {
            let child = self.dirs[d].children[i];
            self.check_frustum_dir(child, frustum);
        }
    }

    /// `updateQuadItemBounds()`: `pos ± radius`.
    pub(crate) fn update_quad_item_bounds(&mut self, d: DirId) {
        let node = &mut self.dirs[d];
        let radius = node.dir_radius;
        let radoffset = Vec2::new(radius, radius);
        node.quad_item_bounds
            .set(node.pos - radoffset, node.pos + radoffset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefix_helpers_match_cpp() {
        assert!(prefixed_by("/a/b/", "/a"));
        assert!(prefixed_by("/a/b/", "/a/"));
        assert!(prefixed_by("/a/b/", "/a/b"));
        assert!(!prefixed_by("/a/bc/", "/a/b"));
        assert!(!prefixed_by("/a/", ""));
        assert_eq!(common_path_prefix("/a/b/c/", "/a/b/d/"), "/a/b/");
        assert_eq!(common_path_prefix("/a/bc/", "/a/bd/"), "/a/");
        assert_eq!(common_path_prefix("/x/", "y/"), "");
    }

    #[test]
    fn dir_name_alpha_fades_over_five_seconds() {
        let s = Settings::default();
        let mut tree = DirTree::new(&s);
        let f = tree.add_file("/a/b/x", Vec3::ONE, 0, &s).unwrap();
        tree.add_file("/c", Vec3::ONE, 0, &s);
        let d = tree.files[f].dir.unwrap();
        let root = tree.root();
        assert_eq!(tree.dirs[root].name_alpha(&s), None);

        let node = &mut tree.dirs[d];
        node.since_last_node_change = 0.0;
        assert_eq!(node.name_alpha(&s), Some(1.0));
        node.since_last_node_change = 2.5;
        assert_eq!(node.name_alpha(&s), Some(0.5));
        node.since_last_node_change = 5.5;
        assert_eq!(node.name_alpha(&s), None);
        let hl = Settings {
            highlight_dirs: true,
            ..Settings::default()
        };
        assert_eq!(node.name_alpha(&hl), Some(1.0));
        let shallow = Settings {
            dir_name_depth: 1,
            highlight_dirs: true,
            ..Settings::default()
        };
        // depth 2 node: 1 < 2 - 1 is false, so it is still drawn.
        assert_eq!(node.depth, 2);
        assert_eq!(node.name_alpha(&shallow), Some(1.0));
        node.depth = 3;
        assert_eq!(node.name_alpha(&shallow), None);
    }

    #[test]
    fn file_rings_follow_cpp_counts() {
        // Ring k holds floor((k + 1) * PI) slots: 1, 6, 9, 12, ...
        assert_eq!((1.0f64).max(2.0 * PI) as i32, 6);
        assert_eq!((1.0f64).max(3.0 * PI) as i32, 9);
        // Slot 0 of a 1-slot ring points along (sin(pi), cos(pi)).
        let d = calc_file_dest(1, 0);
        assert!((d - Vec2::new(0.0, -1.0)).length() < 1e-6);
    }
}
