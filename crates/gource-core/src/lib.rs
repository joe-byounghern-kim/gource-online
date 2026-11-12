// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Simulation core of the Gource port.
//!
//! This crate owns everything that decides *what* is on screen: the event
//! stream, the directory tree and its layout forces, users, actions, files,
//! the camera and the simulation clock. It must stay free of I/O, GPU, and
//! platform dependencies so the same code runs in the browser, in a native
//! binary, and in a headless cloud renderer, and so a given event stream, dt
//! sequence and hash seed always produce the same frame.
//!
//! The behaviour to reproduce is documented under "Reference architecture" in
//! the repository `CLAUDE.md`; the C++ sources under `src/` are the reference.
//!
//! Public surface (the contract other crates build against):
//! - [`events`]: the append-only, interned commit store fed by `gource-sources`.
//! - [`agents`]: the AI agent table (display name, tint, badge per agent id).
//! - [`settings`]: Gource's tunables, addressed by Gource option names.
//! - [`style`]: the contributor style rules (AI decorations).
//! - [`input`]: pointer and keyboard events delivered by the host.
//! - [`sim`]: the [`sim::Simulation`] facade (`Gource::logic` and the read side
//!   of `Gource::draw`).
//! - [`view`]: the per-frame [`view::SceneView`] consumed by `gource-render`.
//! - [`view_hash`]: a deterministic 64-bit fingerprint of a scene view
//!   (frame hashes for parity runs and the recorder's pre-encode oracle).
//!
//! Transcendental maths (`sin`, `cos`, `acos`, `atan2`, `pow`, ...) goes through
//! the `libm` crate and vectors use `glam` with `scalar-math`, so native and
//! wasm32 builds produce bit-identical simulations.

pub mod agents;
pub mod events;
pub mod input;
pub mod pr_style;
pub mod pull_request;
pub mod settings;
pub mod sim;
pub mod style;
pub mod view;
pub mod view_hash;

// Crate-internal port of Gource's simulation, reached through `sim`.
mod action;
mod arena;
mod bounds;
mod camera;
mod datefmt;
mod dirnode;
mod file;
mod filters;
mod frustum;
mod hash;
mod options;
mod pawn;
mod quadtree;
mod rand;
mod tree;
mod user;
mod vectors;

pub use glam;

/// Version of the Gource release this port is derived from.
pub const REFERENCE_GOURCE_VERSION: &str = "0.57";

#[cfg(test)]
mod tests {
    #[test]
    fn reference_version_is_recorded() {
        assert_eq!(super::REFERENCE_GOURCE_VERSION, "0.57");
    }
}
