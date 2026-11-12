// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! WebGL2 backend for the Gource port.
//!
//! Implements the `gource-render` backend contract with raw `web-sys` calls:
//! four GLSL ES 3.00 programs (textured, shadow, bloom, text — ports of
//! `data/shaders/*`), one VAO and growable vertex buffer per stream with
//! zero-copy uploads, a shared quad index buffer, the sprite textures from
//! `data/*.png` (decoded at build time) and a canvas-2D glyph atlas. It
//! recovers from WebGL context loss by recreating every GPU resource.
//!
//! It compiles to an empty crate on non-wasm targets so the workspace builds
//! everywhere.
//!
//! The host page must register `data/fonts/FreeSans.ttf` as the font face
//! `GourceSans` (and wait for it to load) before creating the backend.

#[cfg(target_arch = "wasm32")]
mod atlas;
#[cfg(target_arch = "wasm32")]
mod backend;
#[cfg(any(target_arch = "wasm32", test))]
mod budget;
#[cfg(any(target_arch = "wasm32", test))]
mod packer;
#[cfg(target_arch = "wasm32")]
mod shaders;
#[cfg(target_arch = "wasm32")]
mod streams;
#[cfg(target_arch = "wasm32")]
mod textures;

#[cfg(target_arch = "wasm32")]
pub use atlas::{FONT_FAMILY, GlyphAtlas};
#[cfg(target_arch = "wasm32")]
pub use backend::WebGl2Backend;
