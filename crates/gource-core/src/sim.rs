// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! The simulation facade: `Gource::update`/`Gource::logic` plus the read side
//! of `Gource::draw` (mouse trace, frustum test, scene snapshot).
//!
//! The host owns an [`EventLog`] and a [`Simulation`] side by side and passes
//! the log into every call, so commits can keep arriving while playing.
//!
//! # Mapping to `gource.cpp`
//!
//! - [`Simulation::update`] is `Gource::update` without the draw: the tick
//!   rate clamp (`max_tick_rate` 1/60 s), `time_scale`, `runtime` and
//!   `--stop-at-time`, then `Gource::logic` in its exact order (message and
//!   splash timers, keyboard camera move, pending rotation of dirs and users,
//!   recolour, the paused branch, `readLog`, loop handling, first commit
//!   time, `currtime`/`subseconds` arithmetic, deleting removed files,
//!   popping commits with the auto-skip and time-travel rules, then
//!   `updateBounds`, `interactUsers`, `updateUsers`, `interactDirs`,
//!   `updateDirs`, `updateCamera` and `updateTime`).
//! - [`Simulation::extract_view`] is the first half of `Gource::draw`:
//!   `mousetrace`, the frustum test, and everything `updateVBOs`,
//!   `drawScene` and the text pass read (see `sim/extract.rs`).
//! - [`Simulation::handle_input`] is `mouseMove`, `mouseClick`,
//!   `mouseWheel`, `zoom` and `keyPress` (see `sim/input.rs`).
//!
//! # The event log as Gource's `RCommitLog`
//!
//! The simulation reads the log through a cursor into the commit queue
//! (`commitqueue`, at most 100 commits ahead). A complete log behaves like
//! Gource's seekable `SeekLog`; an incomplete one like the non-seekable
//! `StreamLog`: running out of commits means "wait for more", not
//! "finished", and seeking is unavailable. An empty, incomplete log is
//! Gource's "Reading Log" state (`commitlog == 0`): nothing advances until
//! the first commit arrives. `--start-date` skips to the first valid commit
//! at or after the date the way `RLogMill` does, then `--start-position`
//! seeks (it wins when both are set). On a log that is still streaming in,
//! `--start-position` waits until the log is complete and then seeks once
//! (Gource seeks once the log file is open; a non-seekable stdin stream
//! never applies it). Seeking maps a fraction to a commit with
//! [`EventLog::index_at_fraction`].
//!
//! # Deterministic replacements and deviations
//!
//! - `rand()` in `applyForceUser`, `applyForceAction` and `changeColours`
//!   draws from [`KeyedRng`] streams seeded with `Settings::hash_seed`
//!   (keys `"user.applyForceUser"`, `"user.applyForceAction"`,
//!   `"gource.changeColours"`), rewound by every reset so a restarted or
//!   re-seeked simulation repeats itself. The tree's overlap stream is
//!   described in `tree.rs`. With the hidden option `rand-mode glibc`
//!   every call site instead draws from one emulation of glibc's unseeded
//!   `rand()` shared by the whole simulation and never rewound, as in the
//!   C++ build (parity runs against reference renders; see `rand.rs`).
//! - `std::set<QuadItem*>` iteration in `mousetrace` (pointer order) is id
//!   order.
//! - A commit rejected by the filters is skipped and reading continues; the
//!   C++ stops reading for that frame on a non-seekable stream.
//! - Colours of files without an explicit colour come from
//!   `RCommit::fileColour` evaluated when the commit is processed, with the
//!   hash seed of that moment (Gource evaluates it when the line is parsed,
//!   up to 100 commits earlier).
//! - `Gource::reset` (seeking, looping) zeroes every file key count. Gource's
//!   `FileKey::clear` only zeroes the entries currently on screen, so with
//!   the key hidden its counts double up after a seek.
//! - `FileKey` erases an entry once it has faded out (`count <= 0`, alpha
//!   0) at a recalculation while the key is shown. The simulation does not
//!   see the renderer's alpha; it drops an entry that has been at zero for
//!   2 seconds (`KEY_ENTRY_LINGER`) while the key is shown, and lists only
//!   positive counts in the view.
//! - "no commits found" (an exception in Gource) is the `finished` status.
//!   `--stop-at-end`, `--stop-position`, `--stop-date` and `--stop-at-time`
//!   stop reading as in Gource; where Gource would quit (`appFinished`) the
//!   simulation only reports `finished`.
//! - Not ported: captions, the logo, background images, avatars, video
//!   export frame skipping, the debug overlays and screenshots.
//! - The cursor's 3 s idle timeout advances by the real dt of each
//!   `update`, applied after the next mouse trace (Gource runs
//!   `cursor.logic(dt)` after `draw`).
//!
//! # AI attribution (port addition)
//!
//! Every action carries its commit's attribution set; users keep an assist
//! state from the actions on screen (`user.rs`), and the style rules
//! (`style.rs`: built-in layer from `ai-style`, then `user-style`) turn it
//! into decorations at extraction. The display tables, played counts and
//! `attr_stats` are in `sim/attr.rs`. With no attributions and no rule for
//! plain contributors nothing changes: no decoration, no extra text.

use std::collections::{BTreeMap, VecDeque};

use glam::{Vec2, Vec3};

use crate::action::{PR_CLOSED_COLOUR, PR_MERGED_COLOUR, RAction};
use crate::arena::Arena;
use crate::bounds::Bounds2D;
use crate::camera::ZoomCamera;
use crate::datefmt::format_time;
use crate::events::{AttrSetId, ChangeEvent, ChangeKind, EventLog, FileChange, PrPayload, StrId};
use crate::filters::Filters;
use crate::hash::{colour_hash, stream_key};
use crate::input::{InputEvent, buttons};
use crate::pull_request::{PrFlowStats, PrKey, PrNode, PrReceipts, PrTable, canonical_id};
use crate::quadtree::QuadTree;
use crate::rand::{RandStream, SharedGlibcRand, shared_for};
use crate::settings::{CameraMode, OptionEffect, OptionError, RandMode, Settings};
use crate::style::StyleLayers;
use crate::tree::{DirId, DirTree, FileId};
use crate::user::{ACTION_FORCE_RNG_KEY, RUser, USER_FORCE_RNG_KEY, UserId};
use crate::vectors::{cpp_max, cpp_min, rotate_vec2};
use crate::view::{SceneView, TextRange};

mod attr;
mod extract;
mod input;
#[cfg(test)]
mod pr_tests;

pub use attr::{AgentStats, AttrStats, AttrTotals, ModelStats, STATS_MAX_AGENTS, STATS_MAX_MODELS};

/// Snapshot of playback state for the host UI.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimStatus {
    /// Simulation clock (`currtime`), unix seconds; 0 before the first commit.
    pub current_time: i64,
    /// Formatted date as shown on screen (`displaydate`).
    pub display_date: String,
    /// Playback position in the log, 0..=1: the byte fraction of the next
    /// commit to be played (1 once a complete log has been played); 0 while
    /// the log is still streaming in.
    pub fraction: f32,
    /// Seeking allowed: the log is complete and the progress bar is not
    /// hidden (Gource's `canSeek()`).
    pub seekable: bool,
    pub paused: bool,
    /// Nothing left to play: every commit of a complete log has been
    /// played, a stop condition was reached, or the log has no commits.
    pub finished: bool,
    /// Log index of the next commit to be played (commits consumed so far,
    /// including those the filters skipped).
    pub commits_processed: usize,
    pub files: usize,
    pub users: usize,
    pub dirs: usize,
    pub days_per_second: f32,
}

/// Inner-loop counters of the last simulated frame (Gource's debug
/// overlay): `gGourceDirNodeInnerLoops`, `gGourceUserInnerLoops`,
/// `gGourceFileInnerLoops` (never incremented by Gource 0.57).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SimCounters {
    pub dir_inner_loops: u32,
    pub user_inner_loops: u32,
    pub file_inner_loops: u32,
}

/// `starting_z`: initial camera distance.
const STARTING_Z: f32 = -300.0;
/// `commitqueue_max_size`.
const COMMIT_QUEUE_MAX_SIZE: usize = 100;
/// `MouseCursor::timeout`.
const CURSOR_TIMEOUT: f32 = 3.0;
/// Seconds a message set with [`Simulation::set_message`] stays.
const MESSAGE_SECONDS: f32 = 5.0;
/// Splash duration (`showSplash`, key `c`).
const SPLASH_SECONDS: f32 = 15.0;
/// Format of [`Simulation::date_at`] (`dateAtPosition`).
const SLIDER_DATE_FORMAT: &str = "%A, %d %B, %Y";

const RECOLOUR_RNG_KEY: u64 = stream_key(b"gource.changeColours");

/// One `FileKey` entry: extension colour and file count.
#[derive(Copy, Clone, Debug, PartialEq)]
struct KeyEntry {
    colour: Vec3,
    count: i32,
    /// Seconds the count has been at or below zero.
    zero_time: f32,
}

/// Seconds after its count reaches zero that a key entry is dropped. The
/// renderer fades an entry out at 1/s and `FileKey::logic` erases finished
/// entries at its next 1 s recalculation, so Gource has erased it by then.
const KEY_ENTRY_LINGER: f32 = 2.0;

/// `MouseCursor` plus the SDL mouse state Gource polls.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Cursor {
    hidden: bool,
    idle: f32,
    /// `hasFocus()`: the pointer is over the canvas.
    focus: bool,
    /// Pressed buttons (`input::buttons` bits).
    buttons: u32,
    /// Last pointer position seen, for relative motion.
    last_event_pos: Option<Vec2>,
}

impl Default for Cursor {
    fn default() -> Self {
        Cursor {
            hidden: false,
            idle: CURSOR_TIMEOUT,
            focus: false,
            buttons: 0,
            last_event_pos: None,
        }
    }
}

impl Cursor {
    /// `showCursor(show)`.
    fn show(&mut self, show: bool) {
        if self.hidden == !show {
            return;
        }
        self.hidden = !show;
        if show {
            self.idle = 0.0;
        }
    }

    /// `isVisible()`.
    fn is_visible(&self) -> bool {
        !self.hidden && self.idle < CURSOR_TIMEOUT && self.focus
    }

    /// `updatePos(pos)`: resets the idle timer.
    fn touch(&mut self) {
        self.idle = 0.0;
    }

    fn left_pressed(&self) -> bool {
        self.buttons & buttons::LEFT != 0
    }

    fn right_pressed(&self) -> bool {
        self.buttons & buttons::RIGHT != 0
    }
}

/// Arrow keys held (Gource polls `SDL_GetKeyboardState`).
#[derive(Copy, Clone, Debug, Default, PartialEq)]
struct ArrowKeys {
    left: bool,
    right: bool,
    up: bool,
    down: bool,
}

/// The Gource simulation.
#[derive(Debug)]
pub struct Simulation {
    settings: Settings,
    viewport: Vec2,
    ui_scale: f32,
    paused: bool,

    // --- entities ---
    tree: DirTree,
    users: Arena<RUser>,
    /// `users` (by name).
    user_map: BTreeMap<String, UserId>,
    /// `tagusermap`.
    tag_user_map: BTreeMap<i32, UserId>,
    /// `userTree`.
    user_tree: QuadTree<UserId>,
    user_force_stamp: u32,
    user_bounds: Bounds2D,
    active_user_bounds: Bounds2D,
    /// `file_key`'s extension counts (`FileKey::inc` / `dec`).
    file_key: BTreeMap<String, KeyEntry>,
    filters: Filters,
    pr: PrTable,
    pr_files: BTreeMap<PrKey, FileId>,
    pr_receipts: PrReceipts,
    camera: ZoomCamera,

    // --- log reading ---
    /// `commitlog != 0`.
    initialised: bool,
    /// `--start-position` still to apply: the log was incomplete (not
    /// seekable) when the first commit arrived.
    pending_start_position: bool,
    /// Next commit index to read.
    cursor: usize,
    /// `commitqueue` (commit indices).
    commit_queue: VecDeque<usize>,
    commitqueue_max_size: usize,
    stop_position_reached: bool,
    last_percent: f32,
    /// `appFinished`.
    app_finished: bool,
    /// `file_idle_time` before `--file-idle-time-at-end` replaced it.
    base_file_idle_time: Option<f32>,
    seen_strings: usize,

    // --- time ---
    max_tick_rate: f32,
    runtime: f32,
    currtime: i64,
    lasttime: i64,
    subseconds: f32,
    idle_time: f32,
    tag_seq: i32,
    commit_seq: i32,
    display_date: String,

    // --- camera and input ---
    track_users: bool,
    /// Port-only viewer state, not an option: the dir the overview camera
    /// frames instead of the whole tree (see [`Simulation::set_camera_focus`]).
    camera_focus: String,
    manual_zoom: bool,
    manual_rotate: bool,
    manual_camera: bool,
    rotation_remaining_angle: f32,
    rotate_angle: f32,
    cursor_move: Vec2,
    use_selection_bounds: bool,
    selection_bounds: Bounds2D,
    recolour: bool,
    message: String,
    message_timer: f32,
    splash: f32,
    mousepos: Vec2,
    grab_mouse: bool,
    mousemoved: bool,
    mouseclicked: bool,
    mousedragged: bool,
    /// `cursor` (`MouseCursor`) and the polled mouse buttons.
    pointer: Cursor,
    cursor_dt: f32,
    keys: ArrowKeys,
    hover_file: Option<FileId>,
    selected_file: Option<FileId>,
    hover_user: Option<UserId>,
    selected_user: Option<UserId>,

    // --- rand() replacements ---
    /// The rand mode the streams below (and the tree's) were built for.
    rand_mode: RandMode,
    user_force_rng: RandStream,
    action_force_rng: RandStream,
    recolour_rng: RandStream,

    // --- counters ---
    user_inner_loops: i32,

    // --- AI attribution and styles (sim/attr.rs) ---
    /// The compiled style layers (`ai-style`, `user-style`).
    style: StyleLayers,
    /// Bumped whenever `style` or `hide` change; users re-resolve their
    /// cached style when it differs.
    style_epoch: u32,
    /// Display data per played set id.
    attr_infos: Vec<attr::SetInfo>,
    /// Agent ids and model texts `attr_infos` refers to.
    attr_names: BTreeMap<StrId, Box<str>>,
    /// Played commits per set id.
    played_set_counts: Vec<u32>,
    /// Played commits crediting each agent.
    agent_counts: BTreeMap<StrId, u32>,
    /// Agent key rows (agent, count), rebuilt when `agent_rows_dirty`.
    agent_rows: Vec<(StrId, u32)>,
    agents_more: u32,
    agent_rows_dirty: bool,

    // --- scratch (reused every frame) ---
    scratch_users: Vec<UserId>,
    scratch_inactive: Vec<UserId>,
    scratch_removed: Vec<FileId>,
    scratch_pr_retired: Vec<FileId>,
    scratch_dirs: Vec<DirId>,
    scratch_files: Vec<FileId>,
    scratch_pick_users: Vec<UserId>,
    scratch_pick_dirs: Vec<DirId>,
    scratch_stack: Vec<(DirId, Option<u32>, bool)>,
    spare_textbox_lines: Vec<TextRange>,
}

impl Simulation {
    /// `Gource::Gource()`: camera at z = -300 with Gource's distance limits
    /// and padding, camera mode from the settings, then `reset()`.
    pub fn new(settings: Settings) -> Self {
        let seed = settings.hash_seed as u64;
        let mut camera = ZoomCamera::new(
            Vec3::new(0.0, 0.0, STARTING_Z),
            Vec3::ZERO,
            settings.camera_zoom_default,
            settings.camera_zoom_max,
        );
        camera.set_padding(settings.padding);

        let mut filters = Filters::default();
        filters.sync(&settings);

        let mut sim = Simulation {
            tree: DirTree::new(&settings),
            viewport: Vec2::new(1280.0, 720.0),
            ui_scale: 1.0,
            paused: false,
            users: Arena::default(),
            user_map: BTreeMap::new(),
            tag_user_map: BTreeMap::new(),
            user_tree: QuadTree::default(),
            user_force_stamp: 0,
            user_bounds: Bounds2D::default(),
            active_user_bounds: Bounds2D::default(),
            file_key: BTreeMap::new(),
            filters,
            pr: PrTable::default(),
            pr_files: BTreeMap::new(),
            pr_receipts: PrReceipts::default(),
            camera,
            initialised: false,
            pending_start_position: false,
            cursor: 0,
            commit_queue: VecDeque::with_capacity(COMMIT_QUEUE_MAX_SIZE + 1),
            commitqueue_max_size: COMMIT_QUEUE_MAX_SIZE,
            stop_position_reached: false,
            last_percent: 0.0,
            app_finished: false,
            base_file_idle_time: None,
            seen_strings: 0,
            max_tick_rate: (1.0f64 / 60.0) as f32,
            runtime: 0.0,
            currtime: 0,
            lasttime: 0,
            subseconds: 0.0,
            idle_time: 0.0,
            tag_seq: 1,
            commit_seq: 1,
            display_date: String::new(),
            track_users: false,
            camera_focus: String::new(),
            manual_zoom: false,
            manual_rotate: false,
            manual_camera: false,
            rotation_remaining_angle: 0.0,
            rotate_angle: 0.0,
            cursor_move: Vec2::ZERO,
            use_selection_bounds: false,
            selection_bounds: Bounds2D::default(),
            recolour: false,
            message: String::new(),
            message_timer: 0.0,
            splash: -1.0,
            mousepos: Vec2::ZERO,
            grab_mouse: false,
            mousemoved: false,
            mouseclicked: false,
            mousedragged: false,
            pointer: Cursor::default(),
            cursor_dt: 0.0,
            keys: ArrowKeys::default(),
            hover_file: None,
            selected_file: None,
            hover_user: None,
            selected_user: None,
            rand_mode: RandMode::Keyed,
            user_force_rng: RandStream::new(seed, USER_FORCE_RNG_KEY, None),
            action_force_rng: RandStream::new(seed, ACTION_FORCE_RNG_KEY, None),
            recolour_rng: RandStream::new(seed, RECOLOUR_RNG_KEY, None),
            user_inner_loops: 0,
            style: StyleLayers::new(settings.ai_style, &settings.user_style),
            style_epoch: 0,
            attr_infos: Vec::new(),
            attr_names: BTreeMap::new(),
            played_set_counts: Vec::new(),
            agent_counts: BTreeMap::new(),
            // one more than the key shows: rows are inserted, then truncated
            agent_rows: Vec::with_capacity(crate::view::AGENT_KEY_ROWS + 1),
            agents_more: 0,
            agent_rows_dirty: false,
            scratch_users: Vec::new(),
            scratch_inactive: Vec::new(),
            scratch_removed: Vec::new(),
            scratch_pr_retired: Vec::new(),
            scratch_dirs: Vec::new(),
            scratch_files: Vec::new(),
            scratch_pick_users: Vec::new(),
            scratch_pick_dirs: Vec::new(),
            scratch_stack: Vec::new(),
            // a text box has at most two lines (name and path)
            spare_textbox_lines: Vec::with_capacity(2),
            settings,
        };

        if sim.settings.hide_mouse {
            sim.pointer.show(false);
        }

        sim.install_rand_mode();

        let track = sim.settings.camera_mode == CameraMode::Track;
        sim.set_camera_mode(track);
        sim.reset();
        sim
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn pr_nodes(&self) -> impl ExactSizeIterator<Item = &PrNode> {
        self.pr.nodes()
    }

    pub fn pr_node_by_path(&self, path: StrId) -> Option<&PrNode> {
        self.pr.by_path(path)
    }

    /// Immutable rich hover context for an escaped host tooltip.
    pub fn hovered_pr(&self) -> Option<&PrNode> {
        if self.settings.hide_pull_requests {
            return None;
        }
        self.pr_node_for_file(self.hover_file?)
    }

    pub(crate) fn pr_node_for_file(&self, file: FileId) -> Option<&PrNode> {
        self.pr.node(self.tree.files.get(file)?.pr_key?)
    }

    pub fn pr_flow_stats(&self, log: &EventLog) -> Option<PrFlowStats> {
        if self.settings.hide_pull_requests {
            return None;
        }
        self.pr.flow_stats(log, self.currtime)
    }

    pub(crate) fn pr_flash_age_for_file(&self, file: FileId) -> Option<f32> {
        if self.settings.hide_pull_requests {
            return None;
        }
        let value = self.tree.files.get(file)?;
        if value.pr_key.is_some()
            || value.is_hidden()
            || value.removing
            || value.expired
            || value.forced_removal
        {
            return None;
        }
        self.pr_receipts.age(file, &self.pr, self.runtime)
    }

    fn register_pr_association(
        &mut self,
        log: &EventLog,
        path: StrId,
        value: crate::events::PrAssociation,
    ) {
        if self.settings.hide_pull_requests {
            return;
        }
        let key = (canonical_id(log, value.pr_node), value.number);
        if self.pr.node(key).is_none() {
            return;
        }
        let Some(file) = self.tree.find_file(log.str(path)) else {
            return;
        };
        let target = &self.tree.files[file];
        if target.pr_key.is_some()
            || target.is_hidden()
            || target.removing
            || target.expired
            || target.forced_removal
        {
            return;
        }
        self.pr_receipts.register(
            file,
            key,
            canonical_id(log, value.repository),
            canonical_id(log, value.source_sha),
        );
    }

    fn has_lifecycle(&self, log: &EventLog, index: usize) -> bool {
        if self.settings.hide_pull_requests {
            return false;
        }
        let first = log.records()[index].first_change as usize;
        log.commit(index)
            .changes
            .iter()
            .enumerate()
            .any(|(offset, _)| {
                matches!(log.pr_change(first + offset), Some(PrPayload::Lifecycle(_)))
            })
    }

    fn visible_commit(&mut self, log: &EventLog, index: usize) -> bool {
        if !self.filters.commit_is_valid(log, index) {
            return false;
        }
        if !log.has_pr() {
            return true;
        }
        let first = log.records()[index].first_change as usize;
        log.commit(index)
            .changes
            .iter()
            .enumerate()
            .any(|(offset, change)| {
                self.filters.file_passes(log, change.path)
                    && match log.pr_change(first + offset) {
                        Some(PrPayload::Lifecycle(payload)) => {
                            !self.settings.hide_pull_requests && !payload.initial
                        }
                        _ => true,
                    }
            })
    }

    fn materialize_pr(&mut self, log: &EventLog, key: PrKey) -> Option<FileId> {
        let node = *self.pr.node(key)?;
        if self.settings.hide_pull_requests
            || !node.visible(self.runtime)
            || !self.filters.file_passes(log, node.path)
        {
            return None;
        }
        let file = match self
            .pr_files
            .get(&key)
            .copied()
            .filter(|&file| self.tree.files.contains(file))
        {
            Some(file) => file,
            None => {
                // Never convert an ordinary file into protected lifecycle geometry.
                if self.tree.find_file(log.str(node.path)).is_some() {
                    return None;
                }
                let change = FileChange {
                    path: node.path,
                    kind: ChangeKind::Add,
                    event: ChangeEvent::PrOpened,
                    colour: None,
                };
                let file = self.add_file(log, &change)?;
                self.pr_files.insert(key, file);
                file
            }
        };
        self.tree.files[file].set_pr(key, node.generation);
        self.tree.set_file_hidden(file, false);
        Some(file)
    }

    fn retire_pr(&mut self) {
        let mut retired = std::mem::take(&mut self.scratch_pr_retired);
        retired.clear();
        for (&key, &file) in &self.pr_files {
            if !self
                .pr
                .node(key)
                .is_some_and(|node| node.visible(self.runtime))
            {
                retired.push(file);
            }
        }
        for &file in &retired {
            self.delete_file(file);
        }
        retired.clear();
        self.scratch_pr_retired = retired;
    }

    fn fold_pr_prefix(&mut self, log: &EventLog, cursor: usize) {
        if self.settings.hide_pull_requests {
            return;
        }
        self.pr.sync_code_paths(log);
        for index in 0..cursor.min(log.commit_count()) {
            let commit = log.commit(index);
            let first = log.records()[index].first_change as usize;
            for (offset, change) in commit.changes.iter().enumerate() {
                if let Some(PrPayload::Lifecycle(payload)) = log.pr_change(first + offset) {
                    self.pr.consume(
                        log,
                        change.path,
                        change.event,
                        *payload,
                        commit.user,
                        commit.timestamp,
                        None,
                    );
                }
            }
        }
        let keys: Vec<_> = self
            .pr
            .nodes()
            .filter(|node| node.is_open())
            .map(|node| (node.path, node.number))
            .collect();
        for key in keys {
            self.materialize_pr(log, key);
        }
    }

    /// Apply a Gource option (see [`Settings::apply_option`]). Live options
    /// take effect immediately, including their side effects on existing
    /// entities (camera mode, padding, hash seed recolouring, highlighted
    /// users, user speed and scale, file name time, mouse visibility); for
    /// [`OptionEffect::Restart`] the caller should call
    /// [`Simulation::restart`] (possibly after batching several option
    /// changes).
    pub fn set_option(&mut self, name: &str, value: &str) -> Result<OptionEffect, OptionError> {
        let effect = self.settings.apply_option(name, value)?;
        match crate::options::canonical_name(name) {
            "camera-mode" => {
                let track = self.settings.camera_mode == CameraMode::Track;
                self.set_camera_mode(track);
            }
            "hash-seed" => {
                let seed = self.settings.hash_seed as i32;
                self.apply_hash_seed(seed);
            }
            "highlight-user" | "highlight-users" => self.refresh_highlights(),
            "padding" => self.camera.set_padding(self.settings.padding),
            "max-user-speed" => {
                let speed = self.settings.max_user_speed;
                for (_, user) in self.users.iter_mut() {
                    user.pawn.speed = speed;
                }
            }
            "user-scale" => {
                let scale = self.settings.user_scale;
                for (_, user) in self.users.iter_mut() {
                    user.set_scale(scale);
                }
            }
            "filename-time" => {
                let t = self.settings.filename_time;
                for (_, file) in self.tree.files.iter_mut() {
                    file.pawn.nametime = t;
                }
            }
            "hide" | "hide-mouse" => {
                // the cursor stays hidden while the mouse is grabbed
                let show = !self.settings.hide_mouse && !self.grab_mouse;
                self.pointer.show(show);
                if !show {
                    self.clear_hover();
                }
            }
            "file-idle-time" => self.base_file_idle_time = None,
            "font-scale" => self.apply_default_font_scale(),
            _ => {}
        }
        if matches!(
            crate::options::canonical_name(name),
            "user-style" | "ai-style" | "hide" | "hide-decorations"
        ) {
            self.style = StyleLayers::new(self.settings.ai_style, &self.settings.user_style);
            self.style_epoch = self.style_epoch.wrapping_add(1);
        }
        Ok(effect)
    }

    pub fn option_value(&self, name: &str) -> Option<String> {
        self.settings.option_value(name)
    }

    /// Replace the local time zone ([`Settings::utc_offset_minutes`] and
    /// [`Settings::utc_offset_changes`]) without a restart. `start-date` and
    /// `stop-date` are read again with the new offsets: a date in the other
    /// DST regime moves by the difference (`stop-date` takes effect at
    /// once, `start-date` on the next restart, as when it is set).
    pub fn set_utc_offsets(&mut self, minutes: i32, changes: Vec<(i64, i32)>) {
        self.settings.utc_offset_minutes = minutes;
        self.settings.utc_offset_changes = changes;
        for name in ["start-date", "stop-date"] {
            let value = match name {
                "start-date" => self.settings.start_date.clone(),
                _ => self.settings.stop_date.clone(),
            };
            if !value.is_empty() {
                // it parsed before; with other offsets it still parses
                let _ = self.settings.apply_option(name, &value);
            }
        }
    }

    /// Drawable size in device pixels and device pixels per CSS pixel.
    ///
    /// Without an explicit `font-scale` ([`Settings::default_font_scale`])
    /// this also sets `font_scale` the way Gource's constructor does: on a
    /// HiDPI display the DPI ratio (here already applied through
    /// `ui_scale`, so `font_scale` is 1), otherwise `1 + height / 1600`
    /// (integer division: 2 from 1600 px, 3 from 3200 px).
    pub fn set_viewport(&mut self, width: u32, height: u32, ui_scale: f32) {
        self.viewport = Vec2::new(width as f32, height as f32);
        self.ui_scale = ui_scale;
        self.apply_default_font_scale();
    }

    /// `if(gGourceSettings.default_font_scale)` in `Gource::Gource`.
    fn apply_default_font_scale(&mut self) {
        if !self.settings.default_font_scale {
            return;
        }
        self.settings.font_scale = if self.ui_scale > 1.0 {
            1.0
        } else {
            let threshold = 1600;
            (1 + 0.max(self.viewport.y as i32 / threshold)) as f32
        };
    }

    /// Mouse, wheel and keyboard input (see `sim/input.rs` for the mapping).
    pub fn handle_input(&mut self, event: InputEvent) {
        self.input(event);
    }

    /// Advance one display frame of `dt` real seconds (`Gource::update`
    /// without the draw: tick-rate clamp, time scale, `logic`).
    pub fn update(&mut self, log: &EventLog, dt: f32) {
        if self.log_replaced(log) {
            self.restart(log);
        }
        self.seen_strings = log.string_count();

        let mut scaled_dt = cpp_min(dt, self.max_tick_rate);

        // apply time scaling
        scaled_dt *= self.settings.time_scale;

        // have to manage runtime internally as we're messing with dt
        if !self.paused {
            self.runtime += scaled_dt;
        }

        if self.settings.stop_at_time > 0.0 && self.runtime >= self.settings.stop_at_time {
            self.stop_position_reached = true;
        }

        let t = self.runtime;
        self.logic(log, t, scaled_dt);

        // note: cursor uses real dt (applied after the next mouse trace)
        if !self.settings.hide_mouse {
            self.cursor_dt += dt;
        }
    }

    /// Mouse trace and frustum test against the current camera, then fill
    /// `view` for rendering. Reuses `view`'s allocations.
    pub fn extract_view(&mut self, log: &EventLog, view: &mut SceneView) {
        self.extract(log, view);
    }

    /// Clear every entity and play the log again from the start (or from
    /// `start_position` / `start_date`), applying the current filters. Call
    /// it after options that returned [`OptionEffect::Restart`], and after
    /// replacing the log's contents ([`EventLog::clear`]). The viewport and
    /// pointer state are kept; a pause ends, as every `Gource::seekTo` reset
    /// ends it (a paused frame never reads the log, so the scene would stay
    /// empty).
    pub fn restart(&mut self, log: &EventLog) {
        let mut settings = self.settings.clone();
        if let Some(t) = self.base_file_idle_time {
            settings.file_idle_time = t;
        }
        let mut fresh = Simulation::new(settings);
        fresh.viewport = self.viewport;
        fresh.ui_scale = self.ui_scale;
        fresh.pointer = self.pointer;
        fresh.keys = self.keys;
        fresh.mousepos = self.mousepos;
        fresh.camera_focus = std::mem::take(&mut self.camera_focus);
        fresh.seen_strings = log.string_count();
        *self = fresh;
    }

    /// `Gource::seekTo`: reset and continue from `fraction` of the log.
    /// Only a complete (seekable) log can be seeked; unpauses.
    pub fn seek(&mut self, log: &EventLog, fraction: f32) {
        if !self.initialised && !self.try_init(log) {
            return;
        }
        self.seek_to(log, fraction);
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    /// The settings a fresh simulation needs to replay this one from the
    /// start (the recorder's snapshot): a copy with the adjustment
    /// [`Simulation::restart`] applies, so a `file_idle_time` that
    /// `file-idle-time-at-end` replaced at the end of the log is back at
    /// its base value.
    pub fn settings_snapshot(&self) -> Settings {
        let mut settings = self.settings.clone();
        if let Some(t) = self.base_file_idle_time {
            settings.file_idle_time = t;
        }
        settings
    }

    /// [`SimStatus::finished`] without building a status.
    pub fn finished(&self, log: &EventLog) -> bool {
        self.is_finished(log)
    }

    /// `appFinished`: a stop condition latched and every user is gone
    /// (where Gource would quit).
    pub fn app_finished(&self) -> bool {
        self.app_finished
    }

    /// [`SimStatus::commits_processed`] without building a status: the log
    /// index of the next commit to be played.
    pub fn commit_cursor(&self) -> usize {
        self.cursor.saturating_sub(self.commit_queue.len())
    }

    /// `setMessage`: show `message` for 5 seconds.
    pub fn set_message(&mut self, message: &str) {
        self.message.clear();
        self.message.push_str(message);
        self.message_timer = MESSAGE_SECONDS;
    }

    /// `showSplash()`.
    pub fn show_splash(&mut self) {
        self.splash = SPLASH_SECONDS;
    }

    pub fn status(&self, log: &EventLog) -> SimStatus {
        let next = self.cursor.saturating_sub(self.commit_queue.len());
        // Only a complete log has a fixed denominator: while it streams in,
        // `total_bytes` grows with every batch and the position would jump
        // backwards. Gource shows no position for a log it cannot seek.
        let fraction = if self.initialised && log.is_complete() {
            log.fraction_at_index(next)
        } else {
            0.0
        };
        SimStatus {
            current_time: self.currtime,
            display_date: self.display_date.clone(),
            fraction,
            seekable: self.can_seek(log),
            paused: self.paused,
            finished: self.is_finished(log),
            commits_processed: next,
            files: self.tree.file_count(),
            users: self.user_map.len(),
            dirs: self.tree.dir_count(),
            days_per_second: self.settings.days_per_second,
        }
    }

    pub fn counters(&self) -> SimCounters {
        SimCounters {
            dir_inner_loops: self.tree.dir_inner_loops.max(0) as u32,
            user_inner_loops: self.user_inner_loops.max(0) as u32,
            file_inner_loops: 0,
        }
    }

    /// Date of the commit at `fraction` of the log, formatted like the
    /// slider caption (`Gource::dateAtPosition`: `"%A, %d %B, %Y"` of the
    /// first valid commit within 500 of the position). `None` for
    /// `fraction >= 1`, an incomplete log, or no valid commit.
    pub fn date_at(&self, log: &EventLog, fraction: f32) -> Option<String> {
        // `percent < 1.0` in the C++ (false for NaN too)
        if fraction.partial_cmp(&1.0) != Some(std::cmp::Ordering::Less)
            || !log.is_complete()
            || log.is_empty()
        {
            return None;
        }
        let start = log.index_at_fraction(fraction);
        let end = (start + 500).min(log.commit_count());
        let index = (start..end).find(|&i| self.filters.commit_is_valid_cached(log, i))?;
        let mut date = String::new();
        format_time(
            &mut date,
            SLIDER_DATE_FORMAT,
            log.commit(index).timestamp,
            self.settings.utc_offset_at(log.commit(index).timestamp),
        );
        Some(date)
    }
}

// ---------------------------------------------------------------------------
// Gource::logic and helpers
// ---------------------------------------------------------------------------

impl Simulation {
    /// The log was cleared or replaced under the simulation.
    fn log_replaced(&self, log: &EventLog) -> bool {
        log.string_count() < self.seen_strings || log.commit_count() < self.cursor
    }

    fn can_seek(&self, log: &EventLog) -> bool {
        !self.settings.hide_progress && log.is_complete()
    }

    fn is_finished(&self, log: &EventLog) -> bool {
        if self.app_finished {
            return true;
        }
        if !self.initialised {
            return log.is_complete() && log.is_empty();
        }
        self.commit_queue.is_empty()
            && (self.stop_position_reached
                || (log.is_complete() && self.cursor >= log.commit_count()))
    }

    /// The `commitlog == 0` branch of `Gource::logic` plus `RLogMill`'s
    /// start-date scan. Returns false while still waiting for commits.
    fn try_init(&mut self, log: &EventLog) -> bool {
        if log.is_empty() {
            return false;
        }

        // find first commit after start_timestamp if specified
        if self.settings.start_timestamp != 0 {
            loop {
                if self.cursor >= log.commit_count() {
                    if log.is_complete() {
                        break;
                    }
                    return false;
                }
                let index = self.cursor;
                if (self.has_lifecycle(log, index) || self.visible_commit(log, index))
                    && log.commit(index).timestamp >= self.settings.start_timestamp
                {
                    break;
                }
                self.cursor += 1;
            }
        }

        self.initialised = true;
        if self.cursor > 0 && log.has_pr() {
            self.fold_pr_prefix(log, self.cursor);
            // Seed context uses the selected replay boundary even if no later
            // record can enter the queue (including rows beyond stop-date).
            // A genuine queued action still initializes its normal replay time.
            if self.settings.start_timestamp != 0 && self.pr.nodes().any(|node| node.is_open()) {
                self.currtime = self.settings.start_timestamp;
                self.lasttime = self.currtime;
                self.subseconds = 0.0;
            }
        }

        if self.settings.start_position > 0.0 {
            if log.is_complete() {
                let p = self.settings.start_position;
                self.seek_to(log, p);
            } else {
                self.pending_start_position = true;
            }
        }
        true
    }

    /// `Gource::seekTo`.
    ///
    /// Port difference: a seek (and so a loop) also clears the stop latch
    /// and `appFinished`. Gource quits once they are set, so its `seekTo`
    /// never needs to; the page keeps running with a live slider, and a
    /// latched `stop_position_reached` would stop `read_log` for good. Each
    /// stop condition latches again when playback reaches it.
    fn seek_to(&mut self, log: &EventLog, fraction: f32) {
        if !log.is_complete() {
            return;
        }

        // end pause
        if self.paused {
            self.paused = false;
        }

        self.pending_start_position = false;
        self.stop_position_reached = false;
        self.app_finished = false;

        self.reset();

        self.cursor = log.index_at_fraction(fraction);
        if log.has_pr() {
            self.fold_pr_prefix(log, self.cursor);
            for index in 0..self.cursor {
                if self.visible_commit(log, index) {
                    let attrs = log.commit(index).attrs;
                    self.note_set(log, attrs);
                    self.count_played(attrs);
                }
            }
        } else {
            self.seed_played(log, self.cursor);
        }
    }

    /// `Gource::reset`.
    fn reset(&mut self) {
        self.camera.reset();
        self.user_bounds.reset();
        self.active_user_bounds.reset();
        self.commit_queue.clear();
        self.tag_user_map.clear();

        self.user_tree.clear();

        self.recolour = false;

        self.selected_file = None;
        self.hover_file = None;

        self.use_selection_bounds = false;
        self.selection_bounds.reset();

        self.manual_rotate = false;
        self.manual_zoom = false;
        self.rotation_remaining_angle = 0.0;

        self.message_timer = 0.0;

        self.cursor_move = Vec2::ZERO;

        self.selected_user = None;
        self.hover_user = None;

        self.manual_camera = false;

        self.grab_mouse = false;

        self.mouseclicked = false;
        self.mousemoved = false;
        self.mousedragged = false;

        self.commitqueue_max_size = COMMIT_QUEUE_MAX_SIZE;

        self.rotate_angle = 0.0;

        // root, dir map, files, removed files, dir quadtree and dir_bounds
        self.tree.reset(&self.settings);
        self.pr.clear();
        self.pr_files.clear();
        self.pr_receipts.clear();

        self.users.clear();
        self.user_map.clear();

        self.last_percent = 0.0;

        // file_key.clear(): counts drop to zero, entries fade out
        for entry in self.file_key.values_mut() {
            entry.count = 0;
        }

        self.idle_time = 0.0;
        self.currtime = 0;
        self.lasttime = 0;
        self.subseconds = 0.0;
        self.tag_seq = 1;
        self.commit_seq = 1;

        let seed = self.settings.hash_seed as u64;
        self.user_force_rng.rewind(seed, USER_FORCE_RNG_KEY);
        self.action_force_rng.rewind(seed, ACTION_FORCE_RNG_KEY);

        self.reset_played();
    }

    /// Rebuild every `rand()` stream for `settings.rand_mode`: fresh keyed
    /// streams, or one new glibc generator (seed 1) shared by all sites.
    /// A no-op while the mode is unchanged. `Simulation::new` (and so
    /// [`Simulation::restart`]) calls it: `rand-mode` takes effect when the
    /// simulation is rebuilt.
    fn install_rand_mode(&mut self) {
        let mode = self.settings.rand_mode;
        if mode == self.rand_mode {
            return;
        }
        self.rand_mode = mode;
        let seed = self.settings.hash_seed as u64;
        let shared: Option<SharedGlibcRand> = shared_for(mode);
        let shared = shared.as_ref();
        self.user_force_rng = RandStream::new(seed, USER_FORCE_RNG_KEY, shared);
        self.action_force_rng = RandStream::new(seed, ACTION_FORCE_RNG_KEY, shared);
        self.recolour_rng = RandStream::new(seed, RECOLOUR_RNG_KEY, shared);
        self.tree.overlap_rng = RandStream::new(seed, crate::tree::OVERLAP_RNG_KEY, shared);
    }

    /// `Gource::logic(t, dt)`.
    fn logic(&mut self, log: &EventLog, t: f32, dt: f32) {
        if log.has_pr() {
            self.pr.sync_code_paths(log);
        }
        if self.message_timer > 0.0 {
            self.message_timer -= dt;
        }
        if self.splash > 0.0 {
            self.splash -= dt;
        }

        // init log file
        if !self.initialised && !self.try_init(log) {
            return;
        }

        // a streamed log became seekable: apply --start-position now, as
        // Gource does once the whole log file is open
        if self.pending_start_position && log.is_complete() {
            let p = self.settings.start_position;
            self.seek_to(log, p);
        }

        self.file_key_logic(dt);

        if self.keys.right {
            self.cursor_move.x = 10.0;
            self.manual_camera = true;
        }
        if self.keys.left {
            self.cursor_move.x = -10.0;
            self.manual_camera = true;
        }
        if self.keys.up {
            self.cursor_move.y = -10.0;
            self.manual_camera = true;
        }
        if self.keys.down {
            self.cursor_move.y = 10.0;
            self.manual_camera = true;
        }

        // apply rotation
        if self.rotate_angle != 0.0 {
            let s = libm::sinf(self.rotate_angle);
            let c = libm::cosf(self.rotate_angle);

            if self.manual_rotate {
                // rotate around camera position if manual
                let centre = self.camera.pos.truncate();

                self.tree.rotate_about(s, c, centre);

                for &u in self.user_map.values() {
                    let user = &mut self.users[u];
                    user.pawn.pos = rotate_vec2(user.pawn.pos - centre, s, c) + centre;
                }
            } else {
                self.tree.rotate(s, c);

                for &u in self.user_map.values() {
                    let user = &mut self.users[u];
                    user.pawn.pos = rotate_vec2(user.pawn.pos, s, c);
                }
            }

            self.rotate_angle = 0.0;
        }

        if self.recolour {
            self.change_colours();
            self.recolour = false;
        }

        // still want to update camera while paused
        if self.paused {
            self.update_bounds();
            self.interact_users();
            self.tree.interact_dirs();
            self.update_camera(dt);
            return;
        }

        // get more entries
        if self.commit_queue.is_empty() {
            self.read_log(log);
        }

        // loop in attempt to find commits
        if self.settings.loop_playback
            && self.commit_queue.is_empty()
            && log.is_complete()
            && self.idle_time >= self.settings.loop_delay_seconds
        {
            self.seek_to(log, 0.0);
            self.read_log(log);
        }

        if self.currtime == 0
            && let Some(&first) = self.commit_queue.front()
        {
            let ts = log.commit(first).timestamp;
            self.currtime = ts;
            self.lasttime = ts;
            self.subseconds = 0.0;
        }

        // set current time
        let time_inc = (dt as f64 * 86400.0 * self.settings.days_per_second as f64) as f32;
        let seconds = time_inc as i32;

        self.subseconds += time_inc - seconds as f32;

        if (self.subseconds as f64) >= 1.0 {
            let whole = self.subseconds as i32;
            self.currtime = self.currtime.saturating_add(whole as i64);
            self.subseconds -= whole as f32;
        }

        // saturating: a log can hold timestamps up to i64::MAX (`atoll`
        // semantics); Gource's time_t would wrap, this clock stops there
        self.currtime = self.currtime.saturating_add(seconds as i64);

        // delete files
        let mut removed = std::mem::take(&mut self.scratch_removed);
        self.tree.take_removed_files(&mut removed);
        for &f in &removed {
            self.delete_file(f);
        }
        removed.clear();
        self.scratch_removed = removed;
        self.retire_pr();

        // add commits up until the current time
        while let Some(&index) = self.commit_queue.front() {
            let timestamp = log.commit(index).timestamp;

            // auto skip ahead, unless stop_position_reached
            if self.settings.auto_skip_seconds as f64 >= 0.0
                && self.idle_time >= self.settings.auto_skip_seconds
                && !self.stop_position_reached
            {
                self.currtime = timestamp;
                self.lasttime = timestamp;
                self.idle_time = 0.0;
            }

            if timestamp > self.currtime {
                break;
            }

            self.process_commit(log, index, t);

            if self.settings.no_time_travel {
                if timestamp > self.lasttime {
                    self.lasttime = timestamp;
                }
            } else {
                // allow for non linear time lines
                if self.lasttime > timestamp {
                    self.currtime = timestamp;
                }
                self.lasttime = timestamp;
            }

            self.subseconds = 0.0;

            self.commit_queue.pop_front();
        }

        // reset loop counters
        self.user_inner_loops = 0;
        self.tree.dir_inner_loops = 0;

        self.update_bounds();

        self.interact_users();
        self.update_users(t, dt);

        self.tree.interact_dirs();
        self.tree.update_dirs(dt, &self.settings);

        self.update_camera(dt);

        let display_time = if !self.commit_queue.is_empty() {
            self.currtime
        } else {
            self.lasttime
        };
        self.update_time(display_time);
    }

    /// The part of `FileKey::logic` that owns entries: an entry whose count
    /// stayed at zero until it has faded out is erased (only while the key
    /// is shown, as Gource recalculates the key only then), so a returning
    /// extension gets a fresh entry with its current file's colour.
    fn file_key_logic(&mut self, dt: f32) {
        for entry in self.file_key.values_mut() {
            if entry.count <= 0 {
                entry.zero_time += dt;
            } else {
                entry.zero_time = 0.0;
            }
        }
        if self.settings.show_key {
            self.file_key
                .retain(|_, e| e.count > 0 || e.zero_time <= KEY_ENTRY_LINGER);
        }
    }

    /// `Gource::readLog`: fill the queue until it holds a commit ahead of
    /// `currtime` or 100 commits.
    fn read_log(&mut self, log: &EventLog) {
        if self.stop_position_reached {
            return;
        }

        let count = log.commit_count();
        let seekable = log.is_complete();

        // read commits until either we are ahead of currtime
        loop {
            let finished = seekable && self.cursor >= count;
            if finished {
                break;
            }
            let room = match self.commit_queue.back() {
                None => true,
                Some(&back) => {
                    log.commit(back).timestamp <= self.currtime
                        && self.commit_queue.len() < self.commitqueue_max_size
                }
            };
            if !room {
                break;
            }

            // a stream that ran dry: wait for more
            if self.cursor >= count {
                break;
            }

            let index = self.cursor;
            self.cursor += 1;

            if !self.has_lifecycle(log, index) && !self.visible_commit(log, index) {
                continue;
            }

            if self.settings.stop_timestamp != 0
                && log.commit(index).timestamp > self.settings.stop_timestamp
            {
                self.stop_position_reached = true;
                break;
            }

            self.commit_queue.push_back(index);
        }

        let is_finished = seekable && self.cursor >= count;

        if !is_finished && seekable {
            self.last_percent = log.fraction_at_index(self.cursor);
        }

        if (self.settings.stop_at_end && is_finished)
            || (self.settings.stop_position > 0.0
                && seekable
                && (is_finished || self.last_percent >= self.settings.stop_position))
        {
            self.stop_position_reached = true;
        }

        if (is_finished || self.stop_position_reached) && self.settings.file_idle_time_at_end > 0.0
        {
            if self.base_file_idle_time.is_none() {
                self.base_file_idle_time = Some(self.settings.file_idle_time);
            }
            self.settings.file_idle_time = self.settings.file_idle_time_at_end;
        }
    }

    /// `RCommit::fileColour` (or the change's explicit colour).
    fn change_colour(&self, log: &EventLog, change: &FileChange) -> Vec3 {
        if let Some([r, g, b]) = change.colour {
            let mut colour = Vec3::new(r as f32, g as f32, b as f32);
            colour /= 255.0;
            return colour;
        }
        let filename = log.str(change.path);
        let slash = filename.rfind('/');
        match filename.rfind('.') {
            Some(dot) if dot + 1 < filename.len() && slash.is_none_or(|s| s < dot) => {
                colour_hash(&filename.as_bytes()[dot + 1..], self.tree.hash_seed)
            }
            _ => Vec3::ONE,
        }
    }

    /// `Gource::processCommit`.
    fn process_commit(&mut self, log: &EventLog, index: usize, t: f32) {
        let commit = log.commit(index);
        let visible = self.visible_commit(log, index);
        if visible {
            self.note_set(log, commit.attrs);
            self.count_played(commit.attrs);
        }
        let attr = commit.attrs;
        let mut dirs = std::mem::take(&mut self.scratch_dirs);
        let mut dir_files = std::mem::take(&mut self.scratch_files);

        // find files of this commit or create it
        let first = log.records()[index].first_change as usize;
        for (offset, change) in commit.changes.iter().enumerate() {
            if let Some(PrPayload::Lifecycle(payload)) = log.pr_change(first + offset) {
                if self.settings.hide_pull_requests {
                    continue;
                }
                let old_generation = self
                    .pr
                    .by_path(canonical_id(log, change.path))
                    .map(|node| node.generation);
                if let Some(key) = self.pr.consume(
                    log,
                    change.path,
                    change.event,
                    *payload,
                    commit.user,
                    commit.timestamp,
                    Some(t),
                ) && let Some(file) = self.materialize_pr(log, key)
                    && visible
                    && !payload.initial
                    && self
                        .pr
                        .node(key)
                        .is_some_and(|node| Some(node.generation) != old_generation)
                {
                    self.add_file_action(log, commit.user, commit.timestamp, attr, change, file, t);
                }
                continue;
            }
            if !visible || !self.filters.file_passes(log, change.path) {
                continue;
            }
            if change.kind != ChangeKind::Delete
                && let Some(PrPayload::Association(value)) = log.pr_change(first + offset)
            {
                // Capture before the genuine commit mutation. Missing/new files
                // cannot acquire a synthetic merge flash later in this commit.
                self.register_pr_association(log, change.path, *value);
            }
            let filename = log.str(change.path);

            // deleting a directory: every file beneath it gets an action
            if filename.ends_with('/') {
                // we cannot 'add' or 'modify' a directory
                if change.kind != ChangeKind::Delete {
                    continue;
                }

                dirs.clear();
                let root = self.tree.root();
                self.tree.find_dirs(root, filename, &mut dirs);

                for &dir in &dirs {
                    self.tree.get_files_recursive(dir, &mut dir_files);
                    for &file in &dir_files {
                        if self.tree.files[file].pr_key.is_some() {
                            continue;
                        }
                        self.add_file_action(
                            log,
                            commit.user,
                            commit.timestamp,
                            attr,
                            change,
                            file,
                            t,
                        );
                    }
                }
                continue;
            }

            let file = match self.tree.find_file(filename) {
                Some(f) => f,
                None => match self.add_file(log, change) {
                    Some(f) => f,
                    None => continue,
                },
            };

            self.add_file_action(log, commit.user, commit.timestamp, attr, change, file, t);
        }

        dirs.clear();
        dir_files.clear();
        self.scratch_dirs = dirs;
        self.scratch_files = dir_files;
    }

    /// `Gource::addFile`: the tree part plus `tag_seq` and `file_key.inc`.
    fn add_file(&mut self, log: &EventLog, change: &FileChange) -> Option<FileId> {
        let colour = self.change_colour(log, change);
        let filename = log.str(change.path);
        let file = self
            .tree
            .add_file(filename, colour, self.tag_seq, &self.settings)?;
        self.tag_seq += 1;

        let f = &self.tree.files[file];
        match self.file_key.get_mut(f.ext.as_str()) {
            Some(entry) => {
                entry.count += 1;
                entry.zero_time = 0.0;
            }
            None => {
                self.file_key.insert(
                    f.ext.clone(),
                    KeyEntry {
                        colour: f.file_colour(),
                        count: 1,
                        zero_time: 0.0,
                    },
                );
            }
        }
        Some(file)
    }

    /// `Gource::addFileAction`; the action carries the commit's
    /// attribution set `attr`.
    #[allow(clippy::too_many_arguments)]
    fn add_file_action(
        &mut self,
        log: &EventLog,
        user: StrId,
        timestamp: i64,
        attr: AttrSetId,
        change: &FileChange,
        file: FileId,
        t: f32,
    ) {
        let log_id = user;
        let username = log.str(user);
        let user = match self.user_map.get(username) {
            Some(&u) => u,
            None => {
                let u = self.add_user(username, log_id);
                let highlighted = self.settings.highlight_all_users
                    || self
                        .settings
                        .highlight_users
                        .iter()
                        .any(|h| !h.is_empty() && h == username);
                if highlighted {
                    self.users[u].set_highlighted(true);
                }
                u
            }
        };

        self.commit_seq += 1;

        let action = match change.kind {
            ChangeKind::Delete => RAction::remove(file, timestamp, t),
            ChangeKind::Add => RAction::create(file, timestamp, t),
            ChangeKind::Modify => {
                let colour = self.change_colour(log, change);
                RAction::modify(file, timestamp, t, colour)
            }
        };
        // Pull request events: a merge or a close leaves with its own beam
        // (an open is a create, green as in Gource).
        let action = match change.event {
            ChangeEvent::PrMerged => action.with_beam(PR_MERGED_COLOUR),
            ChangeEvent::PrClosed => action.with_beam(PR_CLOSED_COLOUR),
            ChangeEvent::Commit
            | ChangeEvent::PrOpened
            | ChangeEvent::ReviewApproved
            | ChangeEvent::ReviewChangesRequested
            | ChangeEvent::ReviewCommented
            | ChangeEvent::PrReopened
            | ChangeEvent::PrReady
            | ChangeEvent::PrReviewRequested => action,
        };

        let action = match self.tree.files[file].pr_key {
            Some(key) => action.with_pr(key, self.tree.files[file].pr_generation),
            None => action,
        };
        self.users[user].add_action(action.with_attr(attr.0));
    }

    /// `Gource::addUser`.
    fn add_user(&mut self, username: &str, log_id: StrId) -> UserId {
        let bounds = self.tree.dir_bounds();
        let pos = if bounds.area() > 0.0 {
            bounds.centre()
        } else {
            Vec2::ZERO
        };

        let tagid = self.tag_seq;
        self.tag_seq += 1;

        let user = RUser::new(
            username,
            log_id,
            pos,
            tagid,
            &self.settings,
            self.tree.hash_seed,
        );
        let id = self.users.insert(user);

        self.user_map.insert(username.to_string(), id);
        self.tag_user_map.insert(tagid, id);
        id
    }

    /// `Gource::deleteUser`.
    fn delete_user(&mut self, user: UserId) {
        if self.hover_user == Some(user) {
            self.hover_user = None;
        }
        if self.selected_user == Some(user) {
            self.select_user(None);
        }
        if let Some(u) = self.users.remove(user) {
            self.user_map.remove(u.name());
            self.tag_user_map.remove(&u.pawn.tagid);
        }
    }

    /// `Gource::deleteFile`.
    fn delete_file(&mut self, file: FileId) {
        if !self.tree.files.contains(file) {
            return;
        }

        if let Some(key) = self.tree.files[file].pr_key {
            self.pr_files.remove(&key);
        }
        self.pr_receipts.remove_file(file);

        if self.hover_file == Some(file) {
            self.hover_file = None;
        }
        if self.selected_file == Some(file) {
            self.select_file(None);
        }

        // remove from any users with actions against this file
        for &u in self.user_map.values() {
            self.users[u].file_removed(file);
        }

        if let Some(entry) = self.file_key.get_mut(self.tree.files[file].ext.as_str()) {
            entry.count -= 1;
        }

        self.tree.delete_file(file);
    }

    /// `Gource::updateBounds`.
    fn update_bounds(&mut self) {
        self.user_bounds.reset();
        self.active_user_bounds.reset();

        for &u in self.user_map.values() {
            let user = &mut self.users[u];
            user.update_quad_item_bounds();
            let b = user.pawn.quad_item_bounds;
            self.user_bounds.update_bounds(&b);
            if !user.is_idle() {
                self.active_user_bounds.update_bounds(&b);
            }
        }

        self.tree.update_dir_bounds();
    }

    /// `Gource::interactUsers`: rebuild the user quadtree, then apply user
    /// and action forces in name order.
    fn interact_users(&mut self) {
        let mut quadtreebounds = self.user_bounds;
        quadtreebounds.min -= Vec2::ONE;
        quadtreebounds.max += Vec2::ONE;

        // dont use deep quad tree initially when all the nodes are in one place
        let max_depth = if (self.tree.dir_bounds().area() as f64) > 10000.0 {
            self.tree.config.max_quadtree_depth
        } else {
            1
        };

        self.user_tree.reset(quadtreebounds, max_depth, 1);

        for &u in self.user_map.values() {
            self.user_tree
                .add_item(u, self.users[u].pawn.quad_item_bounds);
        }

        // move users - interact with other users and files
        for &a in self.user_map.values() {
            self.user_force_stamp = self.user_force_stamp.wrapping_add(1);
            if self.user_force_stamp == 0 {
                for (_, user) in self.users.iter_mut() {
                    user.force_seen = 0;
                }
                self.user_force_stamp = 1;
            }
            let stamp = self.user_force_stamp;

            // UserForceFunctor
            let bounds = self.users[a].pawn.quad_item_bounds;
            let users = &mut self.users;
            let rng = &mut self.user_force_rng;
            let mut loop_count = 0;
            self.user_tree
                .visit_items_in_bounds(&bounds, &mut |b, node_count| {
                    if b == a {
                        return;
                    }
                    if node_count != 1 {
                        if users[b].force_seen == stamp {
                            return;
                        }
                        users[b].force_seen = stamp;
                    }
                    let b_pos = users[b].pawn.pos;
                    users[a].apply_force_user(b_pos, rng);
                    loop_count += 1;
                });
            self.user_inner_loops += loop_count;

            self.users[a].apply_force_to_actions(&self.tree, &mut self.action_force_rng);
        }
    }

    /// `Gource::updateUsers`.
    fn update_users(&mut self, t: f32, dt: f32) {
        let mut order = std::mem::take(&mut self.scratch_users);
        order.clear();
        order.extend(self.user_map.values().copied());
        let mut inactive = std::mem::take(&mut self.scratch_inactive);
        inactive.clear();

        let mut idle_users = 0usize;

        // move users
        for &u in &order {
            self.users[u].logic(t, dt, &self.settings, &mut self.tree);

            let user = &self.users[u];

            // deselect user if fading out from inactivity
            if user.is_fading(&self.settings) && self.selected_user == Some(u) {
                self.select_user(None);
            }

            let user = &self.users[u];
            if user.is_inactive() {
                inactive.push(u);
            }

            if user.is_idle() {
                idle_users += 1;
            } else if self.selected_user.is_none() && self.selected_file.is_none() {
                // if nothing is selected, and this user is active and this
                // user is the specified user to follow, select them
                let follow = self
                    .settings
                    .follow_users
                    .iter()
                    .any(|f| !f.is_empty() && f == user.name());
                if follow {
                    self.select_user(Some(u));
                }
            }
        }

        if self.user_map.is_empty() && self.stop_position_reached {
            self.app_finished = true;
        }

        // nothing is moving so increment idle
        if idle_users == self.user_map.len() {
            self.idle_time += dt;
        } else {
            self.idle_time = 0.0;
        }

        // delete inactive users
        for &u in &inactive {
            self.delete_user(u);
        }

        order.clear();
        inactive.clear();
        self.scratch_users = order;
        self.scratch_inactive = inactive;
    }

    fn aspect_ratio(&self) -> f32 {
        if self.viewport.y > 0.0 {
            self.viewport.x / self.viewport.y
        } else {
            1.0
        }
    }

    /// `Gource::updateCamera`.
    fn update_camera(&mut self, dt: f32) {
        // camera tracking
        let mut auto_rotate = !self.manual_rotate && !self.settings.disable_auto_rotate;

        if self.manual_camera {
            if self.cursor_move.dot(self.cursor_move) > 0.0 {
                let cam_rate = (-self.camera.pos.z) / 5000.0;

                let mut cam_pos = self.camera.pos;

                let cursor_delta = self.cursor_move * cam_rate * 10.0;

                cam_pos.x += cursor_delta.x;
                cam_pos.y += cursor_delta.y;

                self.camera.set_pos(cam_pos, true);
                self.camera.stop();

                auto_rotate = false;

                self.cursor_move = Vec2::ZERO;
            }
        } else {
            let cambounds = if self.track_users
                && (self.selected_file.is_some() || self.selected_user.is_some())
            {
                let mut focusbounds = Bounds2D::default();
                if let Some(u) = self.selected_user {
                    focusbounds.update(self.users[u].pos());
                }
                if let Some(f) = self.selected_file {
                    focusbounds.update(self.tree.file_abs_pos(f));
                }
                focusbounds
            } else if self.track_users && self.idle_time == 0.0 {
                self.active_user_bounds
            } else {
                let focus = (!self.camera_focus.is_empty())
                    .then(|| self.tree.dir_bounds_under(&self.camera_focus))
                    .filter(|b| !b.first);
                focus.unwrap_or(*self.tree.dir_bounds())
            };

            let aspect = self.aspect_ratio();
            self.camera
                .adjust(&cambounds, !self.manual_zoom, aspect, &self.settings);
        }

        self.camera.logic(dt);

        // automatically rotate camera
        if auto_rotate {
            if self.rotation_remaining_angle > 0.0 {
                // rotation through 90 degrees, speed peaks at half way
                let shape = (1.0f32 as f64
                    - libm::fabs((self.rotation_remaining_angle / 90.0) as f64 - 0.5) * 2.0)
                    as f32;
                let angle_rate = cpp_max(dt, shape) * dt;

                self.rotate_angle = cpp_min(self.rotation_remaining_angle, 90.0 * angle_rate);
                self.rotation_remaining_angle -= self.rotate_angle;

                self.rotate_angle =
                    (self.rotate_angle as f64 * crate::frustum::DEGREES_TO_RADIANS) as f32;
            } else if !self.pointer.right_pressed() && self.tree.dir_bounds().area() > 10000.0 {
                let aspect_ratio = self.aspect_ratio();
                let b = self.tree.dir_bounds();

                let bounds_ratio = if aspect_ratio > 1.0 {
                    b.width() / b.height()
                } else {
                    b.height() / b.width()
                };

                if bounds_ratio < 0.67 {
                    self.rotation_remaining_angle = 90.0;
                }
            }
        } else {
            self.rotation_remaining_angle = 0.0;
        }
    }

    /// `Gource::updateTime`.
    fn update_time(&mut self, display_time: i64) {
        self.display_date.clear();
        if display_time == 0 {
            return;
        }
        format_time(
            &mut self.display_date,
            &self.settings.date_format,
            display_time,
            self.settings.utc_offset_at(display_time),
        );
    }

    /// `Gource::changeColours`: a new random hash seed.
    fn change_colours(&mut self) {
        let seed = (self.recolour_rng.rand() % 10000) + 1;
        self.settings.hash_seed = seed as u32;
        self.apply_hash_seed(seed);
    }

    /// The recolouring half of `changeColours` for seed `seed`: users,
    /// files (dirs follow their files) and the file key.
    fn apply_hash_seed(&mut self, seed: i32) {
        self.tree.set_hash_seed(seed);

        for &u in self.user_map.values() {
            self.users[u].colourize(seed);
        }

        self.tree.colourize_files();

        for (ext, entry) in self.file_key.iter_mut() {
            entry.colour = if ext.is_empty() {
                Vec3::ONE
            } else {
                colour_hash(ext.as_bytes(), seed)
            };
        }
    }

    /// Re-evaluate `highlighted` for every user after the highlight options
    /// changed (Gource only sets it when a user is created).
    fn refresh_highlights(&mut self) {
        for &u in self.user_map.values() {
            let user = &mut self.users[u];
            let name = user.name();
            let highlighted = self.settings.highlight_all_users
                || self
                    .settings
                    .highlight_users
                    .iter()
                    .any(|h| !h.is_empty() && h == name);
            user.set_highlighted(highlighted);
        }
    }

    /// Port-only: frame the visible dirs under `dir` (for example
    /// `/owner:repo/`; several dirs one per line) instead of the whole tree, while no user or file is
    /// selected for tracking. The rest of the tree keeps playing. An empty
    /// `dir`, or one with nothing visible yet, frames the whole tree. Kept
    /// across seek and restart; not an option, so recipes never carry it.
    /// Precedence: manual camera control; then, in track mode, a selected
    /// user or file, then active users while any are active; then this focus.
    /// My work plays in overview mode, so only manual control overrides it.
    pub fn set_camera_focus(&mut self, dir: &str) {
        self.camera_focus = dir.to_string();
    }

    pub fn camera_focus(&self) -> &str {
        &self.camera_focus
    }

    /// `Gource::setCameraMode(track_users)`.
    fn set_camera_mode(&mut self, track_users: bool) {
        self.manual_rotate = false;
        self.manual_zoom = false;

        self.track_users = track_users;
        if self.selected_user.is_some() {
            self.camera.lock_on(track_users);
        }

        self.manual_camera = false;

        self.settings.camera_mode = if track_users {
            CameraMode::Track
        } else {
            CameraMode::Overview
        };
    }

    /// `Gource::toggleCameraMode`.
    fn toggle_camera_mode(&mut self) {
        let track = !self.track_users;
        self.set_camera_mode(track);
    }

    /// `Gource::selectUser`: select a user, deselect the current file/user.
    fn select_user(&mut self, user: Option<UserId>) {
        // already selected do nothing
        if user.is_some() && self.selected_user == user {
            return;
        }

        if let Some(f) = self.selected_file.take()
            && let Some(file) = self.tree.files.get_mut(f)
        {
            file.set_selected(false);
        }

        // deselect current user
        if let Some(u) = self.selected_user.take()
            && let Some(old) = self.users.get_mut(u)
        {
            old.set_selected(false);
        }

        // if no user return
        let Some(u) = user else {
            self.camera.lock_on(false);
            return;
        };

        self.selected_user = Some(u);

        // select user, lock on camera
        self.users[u].set_selected(true);

        if self.track_users {
            self.camera.lock_on(true);
        }
    }

    /// `Gource::selectFile`: select a file, deselect the current file/user.
    fn select_file(&mut self, file: Option<FileId>) {
        // already selected do nothing
        if file.is_some() && self.selected_file == file {
            return;
        }

        if let Some(u) = self.selected_user.take()
            && let Some(user) = self.users.get_mut(u)
        {
            user.set_selected(false);
        }

        // deselect current file
        if let Some(f) = self.selected_file.take()
            && let Some(old) = self.tree.files.get_mut(f)
        {
            old.set_selected(false);
        }

        // if no file return
        let Some(f) = file else {
            return;
        };

        self.selected_file = Some(f);
        self.tree.files[f].set_selected(true);
    }

    /// `Gource::selectNextUser`: the next fully visible active user in tag
    /// order, wrapping around.
    fn select_next_user(&mut self) {
        let curr_tag_id = self
            .selected_user
            .map(|u| self.users[u].pawn.tagid)
            .unwrap_or(-1);

        let settings = &self.settings;
        let candidate = |user: &RUser| !user.is_inactive() && user.alpha(settings) >= 1.0;

        // find next user after this user
        let next = self
            .tag_user_map
            .values()
            .copied()
            .find(|&u| {
                let user = &self.users[u];
                candidate(user) && user.pawn.tagid > curr_tag_id
            })
            // just get first user
            .or_else(|| {
                self.tag_user_map
                    .values()
                    .copied()
                    .find(|&u| candidate(&self.users[u]))
            });

        self.select_user(next);
    }

    /// `Gource::zoom`.
    fn zoom(&mut self, zoomin: bool) {
        self.manual_zoom = true;

        let zoom_multi: f32 = 1.1;

        let mut distance = -self.camera.dest.z;

        if zoomin {
            distance /= zoom_multi;
            if distance < self.settings.camera_zoom_min {
                distance = self.settings.camera_zoom_min;
            }
        } else {
            distance *= zoom_multi;
            if distance > self.settings.camera_zoom_max {
                distance = self.settings.camera_zoom_max;
            }
        }

        self.camera.set_distance(distance);
    }

    /// `Gource::grabMouse`.
    fn grab_mouse(&mut self, grab: bool) {
        self.grab_mouse = grab;
        self.pointer.show(!grab);
    }

    /// `Gource::selectBackground`: a click on empty space starts dragging
    /// the camera.
    fn select_background(&mut self) {
        // is the left mouse button down?
        if !self.pointer.left_pressed() {
            return;
        }

        self.select_user(None);

        self.manual_camera = true;

        self.mousedragged = true;

        self.grab_mouse(true);
    }
}

#[cfg(test)]
mod tests;
