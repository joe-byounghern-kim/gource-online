// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! The read side of `Gource::draw`: `mousetrace`, the frustum test and the
//! scene snapshot ([`SceneView`]) in Gource's draw order.
//!
//! - Dirs, files and dir/file labels come from one depth-first pre-order
//!   walk, which is the order of `updateEdgeVBO`, `updateBloomVBO`,
//!   `updateFilesVBO` and `drawNames` alike.
//! - Users and their beams follow the `users` map (name order), as in
//!   `updateVBOs`; user labels follow `Gource::draw`'s text pass, then the
//!   selected user and file names are appended last so they draw on top.
//! - `isVisible()` is called wherever the draw code calls it; it caches
//!   `visible = true` on parents of visible children, so the walk needs
//!   `&mut` access to the tree.
//! - Port addition: each user's decorations ([`crate::view::UserDecor`])
//!   from its cached style, the AI line of the hover box, the agent key
//!   rows and the `ai-filter` line (see `sim/attr.rs`).

use std::fmt::Write;

use glam::{Vec2, Vec3};

use super::Simulation;
use crate::events::{EventLog, PrCompleteness, PrLogCoverage};
use crate::frustum::Frustum;
use crate::pr_style;
use crate::pull_request::PrNode;
use crate::view::{
    ActionView, DirView, ExtensionCount, FileView, FontRole, LabelAnchor, LabelView, SceneView,
    TextBoxView, TextRange, UserTexture, UserView,
};

impl Simulation {
    /// `Gource::mousetrace`: Gource's approximate unprojection of the mouse
    /// onto z = 0 (exact only for the 90 degree field of view), then the
    /// user under the mouse, else the last file found among the dirs under
    /// it, then hover and click handling.
    pub(super) fn mousetrace(&mut self) {
        let cam_pos = self.camera.pos;
        let w = self.viewport.x;
        let h = self.viewport.y;

        let mut projected_mouse = Vec2::new(
            -(self.mousepos.x * 2.0 - w) / h,
            1.0 - (2.0 * self.mousepos.y) / h,
        ) * cam_pos.z;
        projected_mouse.x += cam_pos.x;
        projected_mouse.y += cam_pos.y;

        // find user/file under mouse
        let mut file_selection = None;
        let mut user_selection = None;

        if !self.settings.hide_users {
            let mut userset = std::mem::take(&mut self.scratch_pick_users);
            // A leaf holds at most every item once: reserving that keeps the
            // buffer from growing while the entity count is stable.
            userset.clear();
            userset.reserve(self.user_tree.unique_item_count.max(0) as usize);
            self.user_tree
                .collect_items_at(&mut userset, projected_mouse);

            for &u in &userset {
                let Some(user) = self.users.get(u) else {
                    continue;
                };
                if !user.is_fading(&self.settings)
                    && user.pawn.quad_item_bounds.contains(projected_mouse)
                {
                    user_selection = Some(u);
                    break;
                }
            }
            self.scratch_pick_users = userset;
        }

        if user_selection.is_none() && !self.settings.hide_files {
            let mut dirset = std::mem::take(&mut self.scratch_pick_dirs);
            dirset.clear();
            dirset.reserve(self.tree.dir_quadtree().unique_item_count.max(0) as usize);
            self.tree
                .dir_quadtree()
                .collect_items_at(&mut dirset, projected_mouse);

            // The `break` only leaves the file loop: a later dir can still
            // replace the selection, as in the C++.
            for &d in &dirset {
                let Some(dir) = self.tree.dirs.get(d) else {
                    continue;
                };
                for &f in &dir.files {
                    if !self.tree.files[f].is_hidden()
                        && self.tree.file_overlaps(f, projected_mouse)
                    {
                        file_selection = Some(f);
                        break;
                    }
                }
            }
            self.scratch_pick_dirs = dirset;
        }

        if let Some(f) = file_selection {
            // un hover a user
            if let Some(u) = self.hover_user.take()
                && let Some(user) = self.users.get_mut(u)
            {
                user.set_mouse_over(false);
            }

            if Some(f) != self.hover_file {
                // deselect previous selection
                if let Some(old) = self.hover_file
                    && let Some(file) = self.tree.files.get_mut(old)
                {
                    file.set_mouse_over(false);
                }
                // select new
                self.tree.files[f].set_mouse_over(true);
                self.hover_file = Some(f);
            }
        } else if let Some(u) = user_selection {
            // un hover a file
            if let Some(f) = self.hover_file.take()
                && let Some(file) = self.tree.files.get_mut(f)
            {
                file.set_mouse_over(false);
            }

            if Some(u) != self.hover_user {
                if let Some(old) = self.hover_user
                    && let Some(user) = self.users.get_mut(old)
                {
                    user.set_mouse_over(false);
                }
                self.users[u].set_mouse_over(true);
                self.hover_user = Some(u);
            }
        } else {
            self.clear_hover();
        }

        if self.mouseclicked {
            if let Some(u) = self.hover_user {
                self.camera.lock_on(false);
                self.select_user(Some(u));
            } else if let Some(f) = self.hover_file {
                self.camera.lock_on(false);
                self.select_file(Some(f));
            } else {
                self.select_background();
            }
        }
    }

    pub(super) fn clear_hover(&mut self) {
        if let Some(f) = self.hover_file.take()
            && let Some(file) = self.tree.files.get_mut(f)
        {
            file.set_mouse_over(false);
        }
        if let Some(u) = self.hover_user.take()
            && let Some(user) = self.users.get_mut(u)
        {
            user.set_mouse_over(false);
        }
    }

    /// Fill `view` (see [`Simulation::extract_view`]).
    pub(super) fn extract(&mut self, log: &EventLog, view: &mut SceneView) {
        // Keep the text box line buffer across frames without a hover.
        let mut textbox_lines = match view.hud.textbox.take() {
            Some(tb) => tb.lines,
            None => std::mem::take(&mut self.spare_textbox_lines),
        };
        textbox_lines.clear();

        view.clear();
        view.viewport = self.viewport;
        view.ui_scale = self.ui_scale;
        view.background = self.settings.background_colour;
        view.camera = self.camera.view();

        if !self.initialised {
            // loading screen
            self.spare_textbox_lines = textbox_lines;
            return;
        }

        if !self.settings.hide_mouse && self.pointer.is_visible() {
            self.mousetrace();
        } else {
            self.clear_hover();
        }

        let frustum = Frustum::new(
            self.camera.pos,
            self.camera.target,
            self.camera.up,
            self.camera.fov,
            self.camera.znear,
            self.camera.zfar,
            self.aspect_ratio(),
        );

        // check visibility
        self.tree.check_frustum(&frustum);

        self.extract_tree(view);
        self.extract_users(view);
        self.extract_hud(log, view, textbox_lines);

        self.mousemoved = false;
        self.mouseclicked = false;

        // cursor.logic(dt)
        self.pointer.idle += self.cursor_dt;
        self.cursor_dt = 0.0;
    }

    /// Dirs (edges, bloom), files and their labels in tree pre-order.
    fn extract_tree(&mut self, view: &mut SceneView) {
        let s = &self.settings;
        let hide_tree = s.hide_tree;
        let hide_root = s.hide_root;
        let hide_bloom = s.hide_bloom;
        let hide_files = s.hide_files;
        let file_names = !(s.hide_filenames || s.hide_files);
        let dir_names = !s.hide_dirnames;
        let dir_colour = s.dir_colour;
        let filename_colour = s.filename_colour;

        let mut stack = std::mem::take(&mut self.scratch_stack);
        stack.clear();
        stack.push((self.tree.root(), None, true));

        while let Some((d, parent_index, parent_chain)) = stack.pop() {
            let visible = self.tree.is_visible(d);
            let tree = &self.tree;
            let node = &tree.dirs[d];

            let has_parent = node.parent.is_some();
            let has_grandparent = node.parent.is_some_and(|p| tree.dirs[p].parent.is_some());

            // updateEdgeVBO only descends into visible children
            let chain = if has_parent {
                parent_chain && visible
            } else {
                true
            };

            let index = view.dirs.len() as u32;
            view.dirs.push(DirView {
                parent: parent_index,
                depth: node.depth.max(0) as u32,
                pos: node.pos,
                spos: node.spos,
                radius: node.dir_radius,
                colour: node.col,
                in_frustum: node.in_frustum,
                visible,
                draw_edge: !hide_tree && has_parent && (!hide_root || has_grandparent) && chain,
                draw_bloom: node.in_frustum && visible && !hide_bloom,
            });

            // drawNames: the dir name ...
            if dir_names
                && visible
                && let Some(alpha) = node.name_alpha(&self.settings)
            {
                let text = view.push_text(&node.path_token);
                view.labels.push(LabelView {
                    anchor: LabelAnchor::Dir { dir: index },
                    text,
                    font: FontRole::DirName,
                    colour: dir_colour.extend(alpha),
                });
            }

            // ... then the files (updateFilesVBO and the file names)
            if node.in_frustum {
                for &f in &node.files {
                    let file = &tree.files[f];
                    if file.is_hidden() {
                        continue;
                    }
                    let abs_pos = file.pawn.pos + node.pos;
                    let proposal = self.pr_node_for_file(f);

                    if !hide_files {
                        if let Some(proposal) = proposal {
                            let visual =
                                pr_style::appearance(s.pull_request_style, proposal, self.runtime);
                            // Linked discs distinguish proposals using the existing file
                            // texture and pass, without making them committed code files.
                            for (offset, scale) in [
                                (Vec2::ZERO, 0.65),
                                (Vec2::new(-0.3, -0.3), 0.35),
                                (Vec2::new(0.3, 0.3), 0.35),
                            ] {
                                view.files.push(FileView {
                                    pos: abs_pos + offset * file.pawn.dims,
                                    size: file.pawn.dims * scale,
                                    colour: visual.colour,
                                    alpha: file.alpha() * visual.alpha,
                                });
                            }
                        } else {
                            let strength = self
                                .pr_flash_age_for_file(f)
                                .map_or(0.0, pr_style::merge_flash_strength);
                            view.files.push(FileView {
                                pos: abs_pos,
                                size: file.pawn.dims,
                                colour: if strength > 0.0 {
                                    file.colour().lerp(Vec3::ONE, strength)
                                } else {
                                    file.colour()
                                },
                                alpha: file.alpha(),
                            });
                        }
                    }

                    if file_names
                        && !file.pawn.selected
                        && let Some(alpha) = proposal
                            .map(|_| file.alpha())
                            .or_else(|| file.label_alpha())
                    {
                        let text = if let Some(proposal) = proposal {
                            pr_short_label(
                                view,
                                proposal,
                                s.pull_request_style,
                                self.currtime,
                                self.runtime,
                            )
                        } else {
                            view.push_text(file.label_text(&self.settings))
                        };
                        view.labels.push(LabelView {
                            anchor: LabelAnchor::WorldTopLeft {
                                pos: if proposal.is_some() {
                                    pr_label_anchor(
                                        Vec2::new(file.label_anchor(abs_pos).x, abs_pos.y),
                                        s,
                                        self.camera.pos.z,
                                        self.viewport.y,
                                        self.ui_scale,
                                    )
                                } else {
                                    file.label_anchor(abs_pos)
                                },
                            },
                            text,
                            font: FontRole::FileName,
                            colour: filename_colour.extend(alpha),
                        });
                    }
                }
            }

            for &child in node.children.iter().rev() {
                stack.push((child, Some(index), chain));
            }
        }

        stack.clear();
        self.scratch_stack = stack;
    }

    /// Users, action beams and user names (incl. the selected user and file
    /// names drawn last).
    fn extract_users(&mut self, view: &mut SceneView) {
        let decorate = !self.settings.hide_decorations && !self.settings.hide_users;
        if decorate {
            self.update_user_styles();
        }
        let s = &self.settings;

        if !s.hide_users {
            let scale = if s.fixed_user_size {
                -self.camera.pos.z / -super::STARTING_Z
            } else {
                1.0
            };

            for &u in self.user_map.values() {
                let user = &self.users[u];
                view.users.push(UserView {
                    pos: user.pos(),
                    size: user.pawn.dims * scale,
                    colour: user.colour(),
                    alpha: user.alpha(s),
                    texture: UserTexture::Default,
                    decor: if decorate {
                        super::attr::user_decor(user, s.decor_motion)
                    } else {
                        Default::default()
                    },
                });

                for action in &user.active_actions {
                    if action.is_finished() || !self.tree.files.contains(action.target) {
                        continue;
                    }
                    let target = &self.tree.files[action.target];
                    view.actions.push(ActionView {
                        src: user.pos(),
                        dest: self.tree.file_abs_pos(action.target),
                        target_size: target.pawn.size,
                        colour: action.colour,
                        progress: action.progress,
                    });
                }
            }
        }

        if !(s.hide_usernames || s.hide_users) {
            for &u in self.user_map.values() {
                let user = &self.users[u];
                if user.pawn.selected {
                    continue;
                }
                if let Some((colour, alpha)) = user.name_label(s) {
                    let text = view.push_text(user.name());
                    view.labels.push(LabelView {
                        anchor: LabelAnchor::WorldCentredAbove {
                            pos: user.label_anchor(),
                        },
                        text,
                        font: FontRole::UserName,
                        colour: colour.extend(alpha),
                    });
                }
            }
        }

        // draw selected item names again so they are over the top
        if let Some(u) = self.selected_user
            && let Some(user) = self.users.get(u)
            && let Some((colour, alpha)) = user.name_label(s)
        {
            let text = view.push_text(user.name());
            view.labels.push(LabelView {
                anchor: LabelAnchor::WorldCentredAbove {
                    pos: user.label_anchor(),
                },
                text,
                font: FontRole::UserSelected,
                colour: colour.extend(alpha),
            });
        }

        if let Some(f) = self.selected_file
            && let Some(file) = self.tree.files.get(f)
            && (self.pr_node_for_file(f).is_none() || !(s.hide_filenames || s.hide_files))
            && let Some(alpha) = self
                .pr_node_for_file(f)
                .map(|_| file.alpha())
                .or_else(|| file.label_alpha())
        {
            let abs_pos = self.tree.file_abs_pos(f);
            let text = if let Some(proposal) = self.pr_node_for_file(f) {
                pr_short_label(
                    view,
                    proposal,
                    s.pull_request_style,
                    self.currtime,
                    self.runtime,
                )
            } else {
                view.push_text(file.label_text(s))
            };
            view.labels.push(LabelView {
                anchor: LabelAnchor::WorldTopLeft {
                    pos: if self.pr_node_for_file(f).is_some() {
                        pr_label_anchor(
                            Vec2::new(file.label_anchor(abs_pos).x, abs_pos.y),
                            s,
                            self.camera.pos.z,
                            self.viewport.y,
                            self.ui_scale,
                        )
                    } else {
                        file.label_anchor(abs_pos)
                    },
                },
                text,
                font: FontRole::FileSelected,
                colour: s.selection_colour.extend(alpha),
            });
        }
    }

    /// Date, title, message, file key counts, hover text box and splash.
    fn extract_hud(
        &mut self,
        log: &EventLog,
        view: &mut SceneView,
        mut textbox_lines: Vec<crate::view::TextRange>,
    ) {
        self.extract_ai_hud(view);
        self.extract_pr_hud(log, view);
        let s = &self.settings;

        if !s.hide_date && !self.display_date.is_empty() {
            view.hud.date = Some(view.push_text(&self.display_date));
        }

        if !s.title.is_empty() {
            view.hud.title = Some(view.push_text(&s.title));
        }

        if self.message_timer > 0.0 {
            let text = view.push_text(&self.message);
            view.hud.message = Some((text, self.message_timer));
        }

        view.hud.show_key = s.show_key;
        // an unlisted extension has count 0 (fading out in the renderer)
        for (ext, entry) in self.file_key.iter().filter(|(_, e)| e.count > 0) {
            let text = view.push_text(ext);
            view.hud.extensions.push(ExtensionCount {
                ext: text,
                colour: entry.colour,
                count: entry.count as u32,
            });
        }

        // text box
        let hover_file_id = self.hover_file.filter(|&f| Some(f) != self.selected_file);
        let hover_file = hover_file_id.and_then(|f| self.tree.files.get(f));
        let hover_user = self
            .hover_user
            .filter(|&u| Some(u) != self.selected_user)
            .and_then(|u| self.users.get(u));

        if let Some(file) = hover_file {
            textbox_lines.push(view.push_text(file.name()));
            let proposal = hover_file_id.and_then(|f| self.pr_node_for_file(f));
            if let Some(proposal) = proposal {
                let visual = pr_style::appearance(s.pull_request_style, proposal, self.runtime);
                textbox_lines.push(pr_text(
                    view,
                    format_args!("Owner: {}", log.str(proposal.owner)),
                ));
                textbox_lines.push(pr_text(
                    view,
                    format_args!("Review state: {}", visual.label),
                ));
                textbox_lines.push(pr_text(
                    view,
                    format_args!("Last actor: {}", log.str(proposal.actor)),
                ));
                if let Some(target) = proposal.target {
                    textbox_lines.push(pr_text(
                        view,
                        format_args!("Requested: {}", log.str(target)),
                    ));
                }
                textbox_lines.push(pr_text(
                    view,
                    format_args!("Age: {} days", proposal.age_days(self.currtime)),
                ));
            } else {
                let display_path = file.path.get(1..).unwrap_or("");
                if !display_path.is_empty() {
                    textbox_lines.push(view.push_text(display_path));
                }
            }
            view.hud.textbox = Some(TextBoxView {
                lines: textbox_lines,
                colour: proposal.map_or_else(
                    || file.colour(),
                    |p| pr_style::appearance(s.pull_request_style, p, self.runtime).colour,
                ),
                pos: self.mousepos,
            });
        } else if let Some(user) = hover_user {
            textbox_lines.push(view.push_text(user.name()));
            if user.attr_set != 0
                && let Some(line) = self.write_assist_line(user.attr_set, view)
            {
                textbox_lines.push(line);
            }
            view.hud.textbox = Some(TextBoxView {
                lines: textbox_lines,
                colour: user.colour(),
                pos: self.mousepos,
            });
        } else {
            self.spare_textbox_lines = textbox_lines;
        }

        if self.splash > 0.0 {
            view.hud.splash = self.splash;
        }
    }

    fn extract_pr_hud(&self, log: &EventLog, view: &mut SceneView) {
        if self.settings.hide_pull_requests || !log.has_pr_lifecycle() {
            return;
        }
        for line in [
            "PR: P open / D draft / M merged / X closed",
            "Review: A approved / ! changes / C comment",
            "Age: replay days / target in hover",
        ] {
            let text = view.push_text(line);
            view.hud.pr_key.push(text);
        }
        let (lifecycle, reviews, files) = match log.pr_coverage() {
            PrLogCoverage::Known(m) => (m.lifecycle, m.reviews, m.files),
            _ => (
                PrCompleteness::Unknown,
                PrCompleteness::Unknown,
                PrCompleteness::Unknown,
            ),
        };
        for (name, coverage) in [
            ("Lifecycle", lifecycle),
            ("Reviews", reviews),
            ("Files", files),
        ] {
            let state = match coverage {
                PrCompleteness::Complete => "complete",
                PrCompleteness::Partial => "partial",
                PrCompleteness::Unknown => "unknown",
            };
            let text = pr_text(view, format_args!("{name}: {state}"));
            view.hud.pr_coverage.push(text);
        }
        if self.settings.pr_flow_hud
            && let Some(flow) = self.pr_flow_stats(log)
        {
            view.hud.pr_flow = Some(match flow.oldest_days {
                Some(age) => pr_text(
                    view,
                    format_args!("PRs (repository): {} open, oldest {age}d", flow.open),
                ),
                None => pr_text(view, format_args!("PRs (repository): {} open", flow.open)),
            });
        }
    }
}

// The camera uses the fixed 90-degree Gource projection. Convert a logical
// text-row clearance to world units so zoom/DPR do not shrink it back into the
// ordinary directory/file label band. Plain labels retain their original anchor.
fn pr_label_anchor(
    pos: Vec2,
    settings: &crate::settings::Settings,
    camera_z: f32,
    height: f32,
    ui_scale: f32,
) -> Vec2 {
    let selected_size = (18.0 * settings.font_scale) as i32;
    let label_size = settings
        .scaled_filename_font_size()
        .max(settings.scaled_dirname_font_size())
        .max(selected_size)
        .max(18);
    let clearance = (label_size as f32 + 6.0) * ui_scale;
    pos + Vec2::new(
        0.0,
        clearance * (-camera_z).max(0.0) * 2.0 / height.max(1.0),
    )
}

// Write into the warmed frame store rather than allocating intermediate strings.
fn pr_text(view: &mut SceneView, args: std::fmt::Arguments<'_>) -> TextRange {
    let start = view.text.len() as u32;
    view.text
        .write_fmt(args)
        .expect("writing to String cannot fail");
    TextRange {
        start,
        end: view.text.len() as u32,
    }
}

fn pr_short_label(
    view: &mut SceneView,
    node: &PrNode,
    style: crate::settings::PullRequestStyle,
    replay: i64,
    runtime: f32,
) -> TextRange {
    let visual = pr_style::appearance(style, node, runtime);
    pr_text(
        view,
        format_args!(
            "{} #{} {}d",
            visual.glyph,
            node.number,
            node.age_days(replay)
        ),
    )
}
