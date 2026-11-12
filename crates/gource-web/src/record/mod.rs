// SPDX-License-Identifier: GPL-3.0-or-later

//! Video recording, the Rust side of F3 (`docs/features/recording.md`).
//!
//! A recording renders a whole run into frames as fast as the machine
//! allows, independent of the window and of real time. The page forks the
//! live engine into a second one on its own canvas (same event log, a
//! snapshot of the settings, fixed time step, no input), steps it one output
//! frame at a time, captures each frame with WebCodecs and muxes the
//! encoded chunks with `gource-mux`.
//!
//! - [`options`]: the `record-*` options (strict apply/value, REC-30).
//! - [`timing`]: the frame plan mirroring Gource's exporter (5.3).
//! - [`encoder`]: H.264 level, bitrate, size limits and the support ladder
//!   (5.6, REC-44, REC-50, REC-52).
//! - [`estimate`]: length estimate and progress table (5.9).
//! - [`setup`]: the settings snapshot with the forced overrides and the
//!   resolved plan (5.2, REC-35, REC-38).
//! - [`stop`]: stop conditions and progress, shared by the recorder and the
//!   dry run (REC-10, REC-17, REC-67).
//! - [`driver`]: the recorder engine, generic over backend and clock.
//! - [`planner`]: the exact dry run (REC-67).
//! - [`recipe`]: the recipe JSON (5.10).
//! - [`source`]: where a fork's event log comes from (`LogSource`).
//! - [`mux`]: `gource-mux` configured from JSON, for the page.
//! - `wasm` (wasm32 only): `GourceRecorder`, `RecordPlanner`, `Mp4Muxer`.

pub mod driver;
pub mod encoder;
pub mod estimate;
pub mod json;
pub mod mux;
pub mod options;
pub mod planner;
pub mod recipe;
pub mod setup;
pub mod source;
pub mod stop;
pub mod timing;

#[cfg(target_arch = "wasm32")]
pub(crate) mod wasm;

use std::fmt;

/// A recording error: one of the codes of `recording.md` section 5.11 and a
/// human-readable text. JavaScript receives it as a `JsError` whose message
/// is `"<code>: <text>"` (read with `parseWasmError`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordError {
    pub code: &'static str,
    pub message: String,
}

impl RecordError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        RecordError {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for RecordError {}

/// Error codes produced in Rust (the others are raised by the page).
pub mod codes {
    pub const EMPTY_RUN: &str = "record_empty_run";
    pub const NOT_STARTED: &str = "record_not_started";
    /// A preview was asked for while recording (a programming error).
    pub const BUSY: &str = "record_busy";
    pub const LOG_INCOMPLETE: &str = "record_log_incomplete";
    pub const INVALID_OPTION: &str = "record_invalid_option";
    pub const UNBOUNDED: &str = "record_unbounded";
    pub const SIZE_UNSUPPORTED: &str = "record_size_unsupported";
    pub const GL_FAILED: &str = "record_gl_failed";
}

/// Options whose values can name people or places in a repository (F1
/// AI-37): their values are left out of the embedded recipe and listed in its
/// `redacted` field (REC-71).
pub use gource_core::settings::NAME_BEARING_OPTIONS;
