// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Host input delivered to the simulation, replacing SDL events in
//! `Gource::mouseMove`, `mouseClick`, `mouseWheel` and `keyPress`.
//!
//! Coordinates are in drawable (device) pixels with the origin at the top
//! left, the same space as the viewport passed to `Simulation::set_viewport`.

/// Mouse buttons, numbered like DOM `MouseEvent.button`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PointerButton {
    Left,
    Middle,
    Right,
}

/// Bitmask for [`InputEvent::PointerMove::buttons`] (DOM `MouseEvent.buttons`).
pub mod buttons {
    pub const LEFT: u32 = 1;
    pub const RIGHT: u32 = 2;
    pub const MIDDLE: u32 = 4;
}

/// Keys Gource reacts to. Printable keys arrive as [`Key::Char`] with the
/// character produced (so `+` and `=` are distinct, as with SDL keysyms).
/// Gource matches the unshifted keysym, so the simulation folds US-layout
/// shifted punctuation back (`<` is `,`, `>` is `.`, `?` is `/`, `{` is
/// `[`, `}` is `]`, `_` is `-`). The keypad `+` and `-` keys are
/// [`Key::KeypadPlus`] / [`Key::KeypadMinus`] (DOM `KeyboardEvent.code`
/// `NumpadAdd` / `NumpadSubtract`), not `Char`: they zoom instead of
/// changing the speed.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Key {
    Char(char),
    /// `SDLK_KP_PLUS`: zoom out.
    KeypadPlus,
    /// `SDLK_KP_MINUS`: zoom in.
    KeypadMinus,
    Space,
    Tab,
    Escape,
    Enter,
    Left,
    Right,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub enum InputEvent {
    PointerMove {
        x: f32,
        y: f32,
        buttons: u32,
    },
    PointerDown {
        x: f32,
        y: f32,
        button: PointerButton,
    },
    PointerUp {
        x: f32,
        y: f32,
        button: PointerButton,
    },
    /// Pointer left the canvas: hide the cursor, clear hover state.
    PointerLeave,
    /// Wheel rotation in notches; positive = towards the user (zoom out),
    /// matching SDL's negative `wheel.y`.
    Wheel {
        delta: f32,
    },
    Key {
        key: Key,
        pressed: bool,
        shift: bool,
        ctrl: bool,
    },
}
