// SPDX-License-Identifier: GPL-3.0-or-later

//! H.264 in ISO BMFF: [`Mp4Writer`] and its configuration.
//!
//! Timing: samples arrive with microsecond presentation times (WebCodecs
//! `EncodedVideoChunk.timestamp`). Each one is converted on its own, relative
//! to the first sample, as `round(us * timescale / 1e6)`, and a sample's
//! duration is the difference to the next sample's ticks. Nothing accumulates,
//! so there is no drift. The default timescale is [`DEFAULT_TIMESCALE`]
//! (90 kHz): it divides by 24, 25, 30, 48, 50, 60, 72 and 75, so every frame
//! duration of those rates is an exact tick count (3750, 3600, 3000, ..., 1500),
//! and the controller's `round(i * 1e6 / fps)` timestamps are off by at most
//! 0.5 us, which is 0.045 ticks, so they round back to `i * timescale / fps`.
//! (60000 would serve 24/25/30/60 too but not 72 or 75, the C++ exporter's
//! other tick rates; 90 kHz is also the MPEG system clock most tools expect.)

mod boxes;
mod meta;
mod patch;
mod writer;

pub use meta::{Metadata, RECIPE_MAX_BYTES, TOOL_PREFIX};
pub use patch::{Patch, apply_patches, encode_patches};
pub use writer::Mp4Writer;

/// Track and movie timescale: ticks per second. See the module docs.
pub const DEFAULT_TIMESCALE: u32 = 90_000;

/// How the file is laid out. `record-layout=auto` (REC-59) is the caller's
/// choice between these: [`Layout::Faststart`] for a seekable sink with a
/// known frame bound, [`Layout::Fragmented`] otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// `ftyp`, `moov` (empty tables, `mvex`), one `moof`+`mdat` per GOP,
    /// `mfra`. Three duration patches at `finish` (`mvhd`, `tkhd`, `mehd`;
    /// `mdhd` stays zero, REC-58).
    Fragmented,
    /// `ftyp`, `free` reserve, `mdat` streamed sample by sample. `finish`
    /// writes `moov` into the reserve and patches the `mdat` size, so the
    /// sink must seek (or the caller patches its buffer). The reserve is
    /// sized for `max_samples` samples of which at most `max_sync_samples`
    /// are keyframes; a run that outgrows it gets `moov` appended at the end
    /// instead (still a valid, seekable file, just not faststart).
    Faststart {
        /// Upper bound on the frame count (the dry run's exact `N`, or
        /// `record-max-seconds * fps`).
        max_samples: u64,
        /// Upper bound on the keyframe count.
        max_sync_samples: u64,
    },
    /// `ftyp`, then the raw sample bytes with no box around them. `finish`
    /// returns `moov` plus the `mdat` header, to be inserted right after
    /// `ftyp` ([`Placement::InsertAt`]). For in-memory sinks that keep
    /// the parts (`new Blob([ftyp, finish, ...samples])`); no patches.
    FaststartDeferred,
}

/// Where the bytes produced by `finish` go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Placement {
    /// After everything written so far.
    Append,
    /// [`Layout::FaststartDeferred`] when the header was already drained:
    /// the first `len` pending bytes belong at file offset `offset` (right
    /// after `ftyp`, before every sample byte); any pending bytes after them
    /// are appended as usual. (When nothing past `ftyp` was drained yet,
    /// `finish` splices `moov` into the pending output itself and the
    /// placement stays [`Placement::Append`].)
    InsertAt {
        /// File offset of the insertion point (the length of `ftyp`).
        offset: u64,
        /// Bytes at the front of the pending output to insert there.
        len: usize,
    },
}

/// Colour description written to `colr` (`nclx`), ISO/IEC 23091-2 code points.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Colour {
    /// `colour_primaries` (1 is BT.709).
    pub primaries: u16,
    /// `transfer_characteristics` (1 is BT.709).
    pub transfer: u16,
    /// `matrix_coefficients` (1 is BT.709).
    pub matrix: u16,
    /// `full_range_flag`; false is limited (video) range.
    pub full_range: bool,
}

impl Colour {
    /// BT.709 primaries, transfer and matrix, limited range (REC-47).
    pub const BT709_LIMITED: Colour = Colour {
        primaries: 1,
        transfer: 1,
        matrix: 1,
        full_range: false,
    };
}

/// Writer configuration (the wasm `config_json`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mp4Config {
    /// Display width in pixels (`tkhd`, `avc1`). Must match the SPS.
    pub width: u32,
    /// Display height in pixels.
    pub height: u32,
    /// Nominal frame rate: the duration of the last sample when the caller
    /// gives none, and the faststart chunk length (one second of samples).
    pub fps: u32,
    /// Ticks per second for `mvhd` and `mdhd`; must be a multiple of `fps`.
    pub timescale: u32,
    /// `colr` contents.
    pub colour: Colour,
    /// Creation and modification time, seconds since 1904-01-01 UTC. Zero by
    /// default so identical inputs give identical bytes (REC-55).
    pub created: u64,
    /// File layout.
    pub layout: Layout,
    /// Fragmented layout only: also close a fragment after this many samples
    /// (0: only at keyframes, one fragment per GOP, REC-56).
    pub max_fragment_samples: u32,
}

impl Mp4Config {
    /// Fragmented, 90 kHz, BT.709 limited, creation time 0.
    pub fn new(width: u32, height: u32, fps: u32) -> Self {
        Mp4Config {
            width,
            height,
            fps,
            timescale: DEFAULT_TIMESCALE,
            colour: Colour::BT709_LIMITED,
            created: 0,
            layout: Layout::Fragmented,
            max_fragment_samples: 0,
        }
    }
}
