// SPDX-License-Identifier: GPL-3.0-or-later

//! Video container writers for the browser export (F3, `docs/features/recording.md`
//! section 5.7).
//!
//! The browser encodes frames with WebCodecs `VideoEncoder` (H.264 in `avc`
//! format: length-prefixed NAL units, parameter sets out of band in
//! `decoderConfig.description`). This crate turns those chunks into an ISO base
//! media file:
//!
//! - [`mp4::Layout::Fragmented`]: `ftyp`, `moov` with empty sample tables and
//!   `mvex`, then one `moof`/`mdat` pair per GOP, then `mfra`. Streams with
//!   memory bounded by one GOP and survives truncation. Durations in the header
//!   are fixed up by [`mp4::Patch`]es when the sink can seek.
//! - [`mp4::Layout::Faststart`]: `ftyp`, a reserved `free` box, then `mdat`
//!   streamed sample by sample; `finish` writes `moov` into the reserve (as a
//!   patch), so the file has `moov` first.
//! - [`mp4::Layout::FaststartDeferred`]: `ftyp`, then raw sample bytes; `finish`
//!   returns `moov` and the `mdat` header, which the caller inserts after `ftyp`
//!   (an in-memory sink that keeps parts, such as a `Blob`).
//!
//! The writers are written from ISO/IEC 14496-12 and 14496-15 (box registry
//! <https://mp4ra.org/>); no third-party muxer code is used. Output is a pure
//! function of the inputs (no wall clock), the crate has no dependencies and
//! no `unsafe`, and it builds natively and for `wasm32-unknown-unknown` (the
//! `wasm-bindgen` wrapper lives in `gource-web`).

#![forbid(unsafe_code)]

pub mod avc;
mod error;
pub mod mp4;

pub use error::MuxError;
