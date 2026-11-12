// SPDX-License-Identifier: GPL-3.0-or-later

//! DOM input to [`gource_core::input`] events.
//!
//! - `KeyboardEvent.key` values become [`Key`]s: a single printable character
//!   is [`Key::Char`], the named keys Gource knows map to their variants.
//!   [`gource_reacts_to`] lists the keys `Gource::keyPress` (and the held
//!   arrow keys) actually use, so the page can keep the browser's default
//!   behaviour for every other key.
//! - `MouseEvent.button` numbers become [`PointerButton`]s; the `buttons`
//!   bitmask already has Gource's layout ([`gource_core::input::buttons`]).
//! - Wheel deltas arrive in DOM pixels and are turned into whole notches
//!   ([`WheelAccumulator`]), because Gource zooms by a fixed step per SDL
//!   wheel event.

use gource_core::input::{Key, PointerButton, buttons};

/// Map a DOM `KeyboardEvent.key` value. `None` for keys Gource has no use
/// for (function keys, modifiers, `Unidentified`, ...).
pub fn map_key(key: &str) -> Option<Key> {
    let named = match key {
        " " | "Spacebar" => Key::Space,
        "Tab" => Key::Tab,
        "Escape" | "Esc" => Key::Escape,
        "Enter" => Key::Enter,
        "ArrowLeft" | "Left" => Key::Left,
        "ArrowRight" | "Right" => Key::Right,
        "ArrowUp" | "Up" => Key::Up,
        "ArrowDown" | "Down" => Key::Down,
        "PageUp" => Key::PageUp,
        "PageDown" => Key::PageDown,
        "Home" => Key::Home,
        "End" => Key::End,
        _ => {
            let mut chars = key.chars();
            let c = chars.next()?;
            if chars.next().is_some() || c.is_control() {
                return None;
            }
            Key::Char(c)
        }
    };
    Some(named)
}

/// Characters `Gource::keyPress` reacts to (letters in either case).
const GOURCE_CHARS: &str = "mntgudfrkcvzs=+-[].,/";

/// True when Gource does something with `key`: space, tab, the arrow keys
/// and the character commands. Escape (quit), F12 and the renderer toggles
/// have no meaning in the browser and stay with the page.
pub fn gource_reacts_to(key: Key) -> bool {
    match key {
        Key::Space | Key::Tab | Key::Left | Key::Right | Key::Up | Key::Down => true,
        Key::KeypadPlus | Key::KeypadMinus => true,
        Key::Char(c) => GOURCE_CHARS.contains(c.to_ascii_lowercase()),
        Key::Escape | Key::Enter | Key::PageUp | Key::PageDown | Key::Home | Key::End => false,
    }
}

/// DOM `MouseEvent.button`: 0 left, 1 middle (wheel), 2 right. Back and
/// forward buttons are ignored.
pub fn map_button(button: i32) -> Option<PointerButton> {
    match button {
        0 => Some(PointerButton::Left),
        1 => Some(PointerButton::Middle),
        2 => Some(PointerButton::Right),
        _ => None,
    }
}

/// DOM `MouseEvent.buttons`, reduced to the three buttons Gource uses.
pub fn map_buttons(mask: u32) -> u32 {
    mask & (buttons::LEFT | buttons::RIGHT | buttons::MIDDLE)
}

/// DOM wheel pixels per Gource wheel notch. Chrome reports 100 px (120 on
/// some platforms) per mouse-wheel click; trackpads send small deltas that
/// add up.
pub const PIXELS_PER_NOTCH: f64 = 100.0;

/// Upper bound on notches emitted for one DOM event (page-mode deltas).
pub const MAX_NOTCHES_PER_EVENT: u32 = 10;

/// Turns DOM pixel deltas into whole wheel notches.
#[derive(Clone, Debug, Default)]
pub struct WheelAccumulator {
    pending: f64,
}

impl WheelAccumulator {
    /// Add a DOM `deltaY` (pixels, positive = scroll down). Returns the
    /// number of notches to emit and their direction: `+1.0` zooms out
    /// (towards the user, SDL's negative `wheel.y`), `-1.0` zooms in.
    pub fn push(&mut self, delta_y: f64) -> (u32, f32) {
        if !delta_y.is_finite() || delta_y == 0.0 {
            return (0, 0.0);
        }
        // A change of direction starts a fresh gesture.
        if self.pending != 0.0 && self.pending.signum() != delta_y.signum() {
            self.pending = 0.0;
        }
        self.pending += delta_y;
        let notches = (self.pending.abs() / PIXELS_PER_NOTCH).floor();
        if notches < 1.0 {
            return (0, 0.0);
        }
        let sign = self.pending.signum();
        self.pending -= sign * notches * PIXELS_PER_NOTCH;
        let count = (notches as u32).min(MAX_NOTCHES_PER_EVENT);
        (count, sign as f32)
    }

    pub fn reset(&mut self) {
        self.pending = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(map_key(" "), Some(Key::Space));
        assert_eq!(map_key("Tab"), Some(Key::Tab));
        assert_eq!(map_key("ArrowLeft"), Some(Key::Left));
        assert_eq!(map_key("PageDown"), Some(Key::PageDown));
        assert_eq!(map_key("+"), Some(Key::Char('+')));
        assert_eq!(map_key("="), Some(Key::Char('=')));
        assert_eq!(map_key("S"), Some(Key::Char('S')));
        assert_eq!(map_key("é"), Some(Key::Char('é')));
        assert_eq!(map_key("F12"), None);
        assert_eq!(map_key("Shift"), None);
        assert_eq!(map_key("Unidentified"), None);
        assert_eq!(map_key(""), None);
        assert_eq!(map_key("\u{7f}"), None);
    }

    #[test]
    fn handled_keys() {
        for k in [
            " ", "Tab", "ArrowUp", "s", "S", "+", "=", "-", "[", "]", ".", ",", "/", "v",
        ] {
            assert!(gource_reacts_to(map_key(k).unwrap()), "{k}");
        }
        for k in [
            "Escape", "Enter", "Home", "End", "PageUp", "a", "q", "w", "1", "é",
        ] {
            assert!(!gource_reacts_to(map_key(k).unwrap()), "{k}");
        }
    }

    #[test]
    fn buttons() {
        assert_eq!(map_button(0), Some(PointerButton::Left));
        assert_eq!(map_button(1), Some(PointerButton::Middle));
        assert_eq!(map_button(2), Some(PointerButton::Right));
        assert_eq!(map_button(3), None);
        assert_eq!(map_button(-1), None);
        assert_eq!(map_buttons(1 | 2 | 4 | 8 | 16), 7);
    }

    #[test]
    fn wheel_notches() {
        let mut w = WheelAccumulator::default();
        assert_eq!(w.push(100.0), (1, 1.0));
        assert_eq!(w.push(-120.0), (1, -1.0));
        assert_eq!(w.push(-80.0), (1, -1.0)); // 20 left over + 80
        assert_eq!(w.push(30.0), (0, 0.0)); // direction change drops the rest
        assert_eq!(w.push(30.0), (0, 0.0));
        assert_eq!(w.push(40.0), (1, 1.0));
        assert_eq!(w.push(5000.0), (MAX_NOTCHES_PER_EVENT, 1.0));
        w.reset();
        assert_eq!(w.push(f64::NAN), (0, 0.0));
        assert_eq!(w.push(0.0), (0, 0.0));
    }
}
