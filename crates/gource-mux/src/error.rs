// SPDX-License-Identifier: GPL-3.0-or-later

//! Muxer errors.

use std::fmt;

/// Everything the muxer can reject. The web layer maps [`MuxError::code`] to
/// the recording error codes of `recording.md` section 5.11.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MuxError {
    /// The [`crate::mp4::Mp4Config`] is unusable (zero size, frame rate that
    /// does not divide the timescale, ...).
    InvalidConfig(&'static str),
    /// The `AVCDecoderConfigurationRecord` is malformed.
    InvalidAvcc(&'static str),
    /// The SPS in `avcC` decodes to a different picture size than the config.
    SizeMismatch {
        /// Width and height of the config.
        config: (u32, u32),
        /// Cropped width and height decoded from the SPS.
        sps: (u32, u32),
    },
    /// The first sample is not a keyframe.
    FirstSampleNotKey,
    /// A timestamp is not strictly after the previous one (B-frame reordering
    /// or a duplicate), also when two timestamps round to the same tick.
    NonMonotonic {
        /// Previous presentation time in microseconds.
        previous_us: i64,
        /// Rejected presentation time in microseconds.
        pts_us: i64,
    },
    /// A sample does not fit a 32-bit size field.
    SampleTooLarge(u64),
    /// A gap between two samples does not fit a 32-bit duration field.
    DurationTooLarge,
    /// `push` or `finish` after `finish`.
    Finished,
}

impl MuxError {
    /// The recording error code: `record_mux_reorder` for timestamp order
    /// problems (the controller may retry with `latencyMode: 'realtime'`),
    /// `record_mux_failed` for everything else.
    pub fn code(&self) -> &'static str {
        match self {
            MuxError::NonMonotonic { .. } => "record_mux_reorder",
            _ => "record_mux_failed",
        }
    }
}

impl fmt::Display for MuxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MuxError::InvalidConfig(why) => write!(f, "invalid muxer config: {why}"),
            MuxError::InvalidAvcc(why) => write!(f, "invalid avcC: {why}"),
            MuxError::SizeMismatch { config, sps } => write!(
                f,
                "SPS picture size {}x{} differs from the configured {}x{}",
                sps.0, sps.1, config.0, config.1
            ),
            MuxError::FirstSampleNotKey => f.write_str("the first sample is not a keyframe"),
            MuxError::NonMonotonic {
                previous_us,
                pts_us,
            } => write!(
                f,
                "timestamp {pts_us} us is not after the previous {previous_us} us"
            ),
            MuxError::SampleTooLarge(len) => write!(f, "sample of {len} bytes exceeds 4 GiB"),
            MuxError::DurationTooLarge => f.write_str("sample duration exceeds 32 bits"),
            MuxError::Finished => f.write_str("the writer is already finished"),
        }
    }
}

impl std::error::Error for MuxError {}
