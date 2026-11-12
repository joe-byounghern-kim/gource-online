// SPDX-License-Identifier: GPL-3.0-or-later

//! Browser entry point for the Gource port.
//!
//! Exposes the JavaScript API of PLAN section 4 through `wasm-bindgen`:
//! `GourceApp` (main thread: event log, simulation, frame building and the
//! WebGL2 backend on one canvas) and `LogParser` (Web Worker: custom-log
//! bytes to encoded event batches). The page drives the animation-frame
//! loop and forwards DOM input; everything else happens here. Video
//! recording adds `GourceRecorder`, `RecordPlanner` and `Mp4Muxer`
//! ([`record`]).
//!
//! The bindings only exist on `wasm32`. The engine itself ([`engine`]) is
//! generic over the backend and the clock, so it builds and is tested
//! natively.

pub mod ai;
pub mod engine;
pub mod input;
pub mod parser;
pub mod record;
pub mod status;

#[cfg(target_arch = "wasm32")]
mod wasm;

#[cfg(target_arch = "wasm32")]
pub use record::wasm::{GourceRecorder, Mp4Muxer, WasmRecordPlanner};
#[cfg(target_arch = "wasm32")]
pub use wasm::{GourceApp, LogParser};
