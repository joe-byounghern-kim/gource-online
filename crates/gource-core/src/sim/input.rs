// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! `Gource::mouseMove`, `mouseClick`, `mouseWheel` and `keyPress`.
//!
//! - Pointer motion: with the left button dragging the background (after a
//!   click on empty space) the camera pans by the relative motion; with the
//!   right button held the tree rotates (up to 5 degrees per event, along
//!   the dominant axis); otherwise the mouse position updates and the next
//!   [`Simulation::extract_view`] traces it. Relative motion is the
//!   difference between consecutive pointer events (SDL's `xrel`/`yrel`).
//! - Buttons: left click selects the user or file under the mouse, or
//!   starts a background drag; right press grabs the mouse (hidden cursor)
//!   for rotation; middle toggles the camera mode. The position slider is
//!   the host's (Gource's `slider.click` + `seekTo` is
//!   [`Simulation::seek`]). A press also updates the mouse position when the
//!   mouse is not grabbed, so taps without a preceding move work.
//! - Wheel: `zoom(in)` for negative deltas, `zoom(out)` for positive ones.
//! - Keys handled: space (pause), `+`/`=` and `-` (days per second), `[` /
//!   `]` (gravity), `.` `,` `/` (time scale), tab (next user), `v` (camera
//!   mode), `n` (skip to the next commit), `s` (new colours), `t` `g` `u`
//!   `d` `f` `r` `k` `m` (visibility toggles), `c` (splash), `z` (gravity
//!   flag), arrow keys (camera pan while held), keypad `-` / `+` (zoom in /
//!   out). Quit, screenshot, debug and renderer toggles (escape, F12, `q`,
//!   `w`, `y`, `p`) are ignored.
//! - SDL matches the unshifted keysym, so Shift does not change which
//!   command a key runs: shifted US-layout punctuation is folded back
//!   ([`unshift`]); letters are lower-cased. `+` keeps its own arm, as
//!   `SDLK_PLUS` does.
//! - As in Gource, the arrow keys are polled state and still move the camera
//!   with `--disable-input`; every other event is ignored then, and pointer
//!   events (but not the wheel) are ignored with `--hide mouse` or before
//!   the log has commits.

use glam::Vec2;

use super::Simulation;
use crate::frustum::DEGREES_TO_RADIANS;
use crate::input::{InputEvent, Key, PointerButton, buttons};
use crate::vectors::{cpp_max, cpp_min};

fn button_bit(button: PointerButton) -> u32 {
    match button {
        PointerButton::Left => buttons::LEFT,
        PointerButton::Right => buttons::RIGHT,
        PointerButton::Middle => buttons::MIDDLE,
    }
}

/// The unshifted US-layout key of a shifted punctuation character.
fn unshift(c: char) -> char {
    match c {
        '<' => ',',
        '>' => '.',
        '?' => '/',
        '{' => '[',
        '}' => ']',
        '_' => '-',
        c => c.to_ascii_lowercase(),
    }
}

/// The rotation one mouse event asks for: `min(1, |d| / 10) * 5` degrees.
fn drag_angle(d: f32) -> f32 {
    let a = (cpp_min(1.0, libm::fabsf(d) / 10.0) * 5.0) as f64 * DEGREES_TO_RADIANS;
    let a = a as f32;
    if d < 0.0 { -a } else { a }
}

impl Simulation {
    pub(super) fn input(&mut self, event: InputEvent) {
        match event {
            InputEvent::PointerMove { x, y, buttons } => {
                let p = Vec2::new(x, y);
                let rel = self
                    .pointer
                    .last_event_pos
                    .map(|last| p - last)
                    .unwrap_or(Vec2::ZERO);
                self.pointer.last_event_pos = Some(p);
                self.pointer.buttons = buttons;
                self.pointer.focus = true;
                self.mouse_move(p, rel);
            }
            InputEvent::PointerDown { x, y, button } => {
                let p = Vec2::new(x, y);
                self.pointer.last_event_pos = Some(p);
                self.pointer.buttons |= button_bit(button);
                self.pointer.focus = true;
                self.mouse_click(true, button, p);
            }
            InputEvent::PointerUp { x, y, button } => {
                self.pointer.last_event_pos = Some(Vec2::new(x, y));
                self.pointer.buttons &= !button_bit(button);
                self.mouse_click(false, button, Vec2::new(x, y));
            }
            InputEvent::PointerLeave => {
                self.pointer.focus = false;
                self.pointer.last_event_pos = None;
            }
            InputEvent::Wheel { delta } => self.mouse_wheel(delta),
            InputEvent::Key {
                key,
                pressed,
                shift: _,
                ctrl: _,
            } => {
                match key {
                    Key::Left => self.keys.left = pressed,
                    Key::Right => self.keys.right = pressed,
                    Key::Up => self.keys.up = pressed,
                    Key::Down => self.keys.down = pressed,
                    _ => {}
                }
                if pressed {
                    self.key_press(key);
                }
            }
        }
    }

    /// `Gource::mouseMove`.
    fn mouse_move(&mut self, p: Vec2, rel: Vec2) {
        if !self.initialised || self.settings.disable_input || self.settings.hide_mouse {
            return;
        }

        let rightmouse = self.pointer.right_pressed();

        // move camera in direction the user dragged the mouse
        if self.mousedragged || rightmouse {
            let mag = rel;

            // if right mouse button is held while dragging, rotate tree
            // instead of moving camera
            if rightmouse {
                self.manual_rotate = true;
                self.rotate_angle = if libm::fabsf(mag.x) > libm::fabsf(mag.y) {
                    drag_angle(mag.x)
                } else {
                    drag_angle(mag.y)
                };
                return;
            }

            self.cursor_move += mag;
            return;
        }

        if self.grab_mouse {
            return;
        }

        self.mousepos = p;
        self.mousemoved = true;
        self.pointer.touch();
    }

    /// `Gource::mouseClick`.
    fn mouse_click(&mut self, down: bool, button: PointerButton, p: Vec2) {
        if !self.initialised || self.settings.disable_input || self.settings.hide_mouse {
            return;
        }

        if down && !self.grab_mouse && !self.mousedragged {
            self.mousepos = p;
        }

        // mouse click should stop the cursor being idle
        self.pointer.touch();

        if !down {
            if button == PointerButton::Left {
                // stop dragging mouse
                self.mousedragged = false;
            }

            if matches!(button, PointerButton::Left | PointerButton::Right)
                && self.pointer.buttons & (buttons::LEFT | buttons::RIGHT) == 0
            {
                self.grab_mouse(false);
            }
            return;
        }

        match button {
            PointerButton::Middle => self.toggle_camera_mode(),
            PointerButton::Right => self.grab_mouse(true),
            PointerButton::Left => self.mouseclicked = true,
        }
    }

    /// `Gource::mouseWheel`.
    fn mouse_wheel(&mut self, delta: f32) {
        if self.settings.disable_input {
            return;
        }
        if delta < 0.0 {
            self.zoom(true);
        }
        if delta > 0.0 {
            self.zoom(false);
        }
    }

    /// `Gource::keyPress` (key down only).
    fn key_press(&mut self, key: Key) {
        if self.settings.disable_input {
            return;
        }

        if !self.initialised {
            return;
        }

        match key {
            Key::Tab => self.select_next_user(),
            Key::Space => self.paused = !self.paused,
            Key::Char(c) => self.char_key(unshift(c)),
            Key::KeypadMinus => self.zoom(true),
            Key::KeypadPlus => self.zoom(false),
            _ => {}
        }
    }

    fn char_key(&mut self, c: char) {
        let s = &mut self.settings;
        match c {
            'm' => {
                // toggle mouse visibility unless mouse clicked/pressed/dragged
                if !(self.mousedragged || self.mouseclicked || self.pointer.left_pressed()) {
                    if !self.pointer.hidden {
                        self.pointer.show(false);
                        s.hide_mouse = true;
                    } else {
                        self.pointer.show(true);
                        s.hide_mouse = false;
                    }
                }
            }
            'n' => self.idle_time = s.auto_skip_seconds,
            't' => s.hide_tree = !s.hide_tree,
            'g' => s.hide_users = !s.hide_users,
            'u' => {
                if s.hide_usernames && !s.highlight_all_users {
                    s.hide_usernames = false;
                    s.highlight_all_users = true;
                } else if s.highlight_all_users && !s.hide_usernames {
                    s.hide_usernames = false;
                    s.highlight_all_users = false;
                } else {
                    s.hide_usernames = true;
                    s.highlight_all_users = false;
                }
            }
            'd' => {
                if s.hide_dirnames && !s.highlight_dirs {
                    s.hide_dirnames = false;
                    s.highlight_dirs = true;
                } else if s.highlight_dirs && !s.hide_dirnames {
                    s.hide_dirnames = false;
                    s.highlight_dirs = false;
                } else {
                    s.hide_dirnames = true;
                    s.highlight_dirs = false;
                }
            }
            'f' => {
                if s.hide_filenames && !s.file_extensions {
                    s.hide_filenames = false;
                } else if !s.hide_filenames && s.file_extensions {
                    s.file_extensions = false;
                    s.hide_filenames = true;
                } else {
                    s.file_extensions = true;
                    s.hide_filenames = false;
                }
            }
            'r' => s.hide_root = !s.hide_root,
            'k' => s.show_key = !s.show_key,
            'c' => self.splash = super::SPLASH_SECONDS,
            'v' => self.toggle_camera_mode(),
            'z' => self.tree.config.gravity = !self.tree.config.gravity,
            's' => self.recolour = true,
            '=' | '+' => {
                s.days_per_second = if s.days_per_second >= 1.0 {
                    cpp_min(30.0, libm::floorf(s.days_per_second) + 1.0)
                } else {
                    cpp_min(1.0, s.days_per_second * 2.0)
                };
            }
            '-' => {
                s.days_per_second = if s.days_per_second > 1.0 {
                    cpp_max(0.0, libm::floorf(s.days_per_second) - 1.0)
                } else {
                    cpp_max(0.0, s.days_per_second * 0.5)
                };
            }
            '[' => {
                let g = &mut self.tree.config.force_gravity;
                *g = (*g as f64 / 1.1) as f32;
            }
            ']' => {
                let g = &mut self.tree.config.force_gravity;
                *g = (*g as f64 * 1.1) as f32;
            }
            '.' => {
                s.time_scale = if s.time_scale >= 1.0 {
                    cpp_min(4.0, libm::floorf(s.time_scale) + 1.0)
                } else {
                    cpp_min(1.0, s.time_scale * 2.0)
                };
            }
            ',' => {
                s.time_scale = if s.time_scale > 1.0 {
                    cpp_max(0.0, libm::floorf(s.time_scale) - 1.0)
                } else {
                    cpp_max(0.25, s.time_scale * 0.5)
                };
            }
            '/' => s.time_scale = 1.0,
            _ => {}
        }
    }
}
