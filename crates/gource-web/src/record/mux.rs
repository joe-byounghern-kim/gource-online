// SPDX-License-Identifier: GPL-3.0-or-later

//! The muxer as the page uses it (`recording.md` section 5.7): `gource-mux`
//! behind a JSON configuration, with output drained into the caller's
//! buffer through a reused scratch buffer, so a steady run allocates
//! nothing per chunk in Rust.
//!
//! Configuration (`Mp4Muxer` constructor):
//!
//! ```json
//! {"width": 1920, "height": 1080, "fps": 60,
//!  "layout": "fragmented" | "faststart" | "faststart-deferred",
//!  "max_samples": 3600, "max_sync_samples": 31,
//!  "colour": {"primaries": "bt709", "transfer": "iec61966-2-1",
//!             "matrix": "bt709", "full_range": true},
//!  "created": 0, "comment": "", "build": "", "timescale": 90000}
//! ```
//!
//! `colour` is the encoder's `decoderConfig.colorSpace` as WebCodecs reports
//! it (names) or ISO/IEC 23091-2 code points (numbers). Without it the
//! colour of Chrome's canvas capture is assumed: BT.709 primaries and
//! matrix, sRGB transfer, full range (M0 spike: the `colr` box must agree
//! with the stream's VUI, which takes the frame's tag). `max_samples` and
//! `max_sync_samples` size the faststart reserve (`plan.layout`).

use gource_mux::MuxError;
use gource_mux::mp4::{
    Colour, DEFAULT_TIMESCALE, Layout, Metadata, Mp4Config, Mp4Writer, Placement, encode_patches,
};

use crate::record::RecordError;
use crate::record::codes;
use crate::record::json::{self, Value, number, object, string};

/// Colour of a `VideoFrame` taken from a WebGL canvas in Chrome, as the
/// encoder tags the stream (BT.709 primaries and matrix, sRGB transfer,
/// full range).
pub const CANVAS_COLOUR: Colour = Colour {
    primaries: 1,
    transfer: 13,
    matrix: 1,
    full_range: true,
};

fn invalid(message: impl Into<String>) -> RecordError {
    RecordError::new(codes::INVALID_OPTION, message)
}

/// A muxer error as a recording error (`record_mux_reorder` or
/// `record_mux_failed`).
pub fn mux_error(e: MuxError) -> RecordError {
    RecordError::new(e.code(), e.to_string())
}

/// A code point from a WebCodecs enum name or a number.
fn code_point(
    v: Option<&Value>,
    names: &[(&str, u16)],
    default: u16,
    what: &str,
) -> Result<u16, RecordError> {
    match v {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Number(_)) => v
            .and_then(Value::as_f64)
            .filter(|n| n.fract() == 0.0 && (0.0..=255.0).contains(n))
            .map(|n| n as u16)
            .ok_or_else(|| invalid(format!("colour {what} must be 0 to 255"))),
        Some(Value::String(s)) => names
            .iter()
            .find(|(n, _)| n == s)
            .map(|(_, c)| *c)
            .ok_or_else(|| invalid(format!("unknown colour {what} '{s}'"))),
        Some(_) => Err(invalid(format!("colour {what} must be a name or a number"))),
    }
}

/// `VideoColorSpaceInit` names to code points.
const PRIMARIES: [(&str, u16); 5] = [
    ("bt709", 1),
    ("bt470bg", 5),
    ("smpte170m", 6),
    ("bt2020", 9),
    ("smpte432", 12),
];
const TRANSFER: [(&str, u16); 7] = [
    ("bt709", 1),
    ("smpte170m", 6),
    ("linear", 8),
    ("iec61966-2-1", 13),
    ("pq", 16),
    ("hlg", 18),
    ("srgb", 13),
];
const MATRIX: [(&str, u16); 5] = [
    ("rgb", 0),
    ("bt709", 1),
    ("bt470bg", 5),
    ("smpte170m", 6),
    ("bt2020-ncl", 9),
];

fn colour(v: Option<&Value>) -> Result<Colour, RecordError> {
    let Some(v) = v.filter(|v| **v != Value::Null) else {
        return Ok(CANVAS_COLOUR);
    };
    if v.members().is_none() {
        return Err(invalid("colour must be an object"));
    }
    let d = CANVAS_COLOUR;
    Ok(Colour {
        primaries: code_point(v.get("primaries"), &PRIMARIES, d.primaries, "primaries")?,
        transfer: code_point(v.get("transfer"), &TRANSFER, d.transfer, "transfer")?,
        matrix: code_point(v.get("matrix"), &MATRIX, d.matrix, "matrix")?,
        full_range: match v.get("full_range").or_else(|| v.get("fullRange")) {
            None | Some(Value::Null) => d.full_range,
            Some(b) => b
                .as_bool()
                .ok_or_else(|| invalid("colour full_range must be true or false"))?,
        },
    })
}

fn uint(v: &Value, key: &str, default: Option<u64>) -> Result<u64, RecordError> {
    match v.get(key) {
        None | Some(Value::Null) => {
            default.ok_or_else(|| invalid(format!("muxer config needs '{key}'")))
        }
        Some(n) => n
            .as_f64()
            .filter(|n| n.fract() == 0.0 && *n >= 0.0 && *n <= 9.0e15)
            .map(|n| n as u64)
            .ok_or_else(|| invalid(format!("muxer config '{key}' must be a whole number"))),
    }
}

/// Parse the muxer configuration; returns the writer config and the tags.
pub fn parse_config(text: &str) -> Result<(Mp4Config, Metadata), RecordError> {
    let v = json::parse(text).map_err(|e| invalid(format!("muxer config: {e}")))?;
    if v.members().is_none() {
        return Err(invalid("muxer config must be a JSON object"));
    }
    let small = |key: &str, default: Option<u64>| -> Result<u32, RecordError> {
        u32::try_from(uint(&v, key, default)?)
            .map_err(|_| invalid(format!("muxer config '{key}' is too large")))
    };
    let (width, height, fps) = (
        small("width", None)?,
        small("height", None)?,
        small("fps", None)?,
    );
    let layout = match v
        .get("layout")
        .and_then(Value::as_str)
        .unwrap_or("fragmented")
    {
        "fragmented" => Layout::Fragmented,
        "faststart" => {
            let max_samples = uint(&v, "max_samples", None)?;
            let max_sync = uint(&v, "max_sync_samples", Some(max_samples))?;
            Layout::Faststart {
                max_samples,
                max_sync_samples: max_sync,
            }
        }
        "faststart-deferred" => Layout::FaststartDeferred,
        other => return Err(invalid(format!("unknown muxer layout '{other}'"))),
    };
    let text_of = |key: &str| v.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let config = Mp4Config {
        width,
        height,
        fps,
        timescale: small("timescale", Some(u64::from(DEFAULT_TIMESCALE)))?,
        colour: colour(v.get("colour"))?,
        created: uint(&v, "created", Some(0))?,
        layout,
        max_fragment_samples: small("max_fragment_samples", Some(0))?,
    };
    let meta = Metadata {
        comment: text_of("comment"),
        build: text_of("build"),
        recipe: Vec::new(),
    };
    Ok((config, meta))
}

/// An MP4 writer fed by the page.
pub struct Muxer {
    writer: Mp4Writer,
    scratch: Vec<u8>,
}

impl Muxer {
    /// `avcc` is `decoderConfig.description`; `recipe` the embedded recipe
    /// (none: no `udta` unless the config has a comment).
    pub fn new(
        config_json: &str,
        avcc: &[u8],
        recipe: Option<Vec<u8>>,
    ) -> Result<Self, RecordError> {
        let (config, mut meta) = parse_config(config_json)?;
        let embed = recipe.is_some() || !meta.comment.is_empty();
        meta.recipe = recipe.unwrap_or_default();
        let writer = Mp4Writer::new(config, avcc, embed.then_some(&meta)).map_err(mux_error)?;
        Ok(Muxer {
            writer,
            scratch: Vec::new(),
        })
    }

    /// One encoded chunk; returns the bytes waiting to be taken.
    pub fn push(
        &mut self,
        data: &[u8],
        pts_us: f64,
        duration_us: f64,
        is_key: bool,
    ) -> Result<usize, RecordError> {
        if !pts_us.is_finite() || pts_us.abs() > 9.0e15 {
            return Err(RecordError::new(
                "record_mux_failed",
                "the chunk timestamp is not a number",
            ));
        }
        let duration = if duration_us.is_finite() && duration_us > 0.0 {
            duration_us as u64
        } else {
            0
        };
        self.writer
            .push(data, pts_us as i64, duration, is_key)
            .map_err(mux_error)
    }

    pub fn pending(&self) -> usize {
        self.writer.pending_output().len()
    }

    /// Move up to `n` pending bytes into the scratch buffer and return
    /// them (the wasm layer copies them into the page's buffer).
    pub fn take(&mut self, n: usize) -> &[u8] {
        let n = n.min(self.pending());
        self.scratch.resize(n, 0);
        let got = self.writer.take_output(&mut self.scratch[..n]);
        &self.scratch[..got]
    }

    /// Ends the file; returns the bytes waiting to be taken.
    pub fn finish(&mut self) -> Result<usize, RecordError> {
        self.writer.finish().map_err(mux_error)
    }

    /// The patches, encoded as `(offset u64 LE, length u32 LE, bytes)*`.
    pub fn patches(&self) -> Vec<u8> {
        let mut out = Vec::new();
        encode_patches(self.writer.patches(), &mut out);
        out
    }

    /// `{"kind":"append"}` or `{"kind":"insert_at","offset":N,"len":N}`.
    pub fn placement_value(&self) -> Value {
        match self.writer.placement() {
            Placement::Append => object([("kind", string("append"))]),
            Placement::InsertAt { offset, len } => object([
                ("kind", string("insert_at")),
                ("offset", number(offset as f64)),
                ("len", number(len as f64)),
            ]),
        }
    }

    pub fn writer(&self) -> &Mp4Writer {
        &self.writer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_and_colour_names() {
        let (c, m) = parse_config(r#"{"width":1920,"height":1080,"fps":60}"#).unwrap();
        assert_eq!(
            (c.width, c.height, c.fps, c.timescale),
            (1920, 1080, 60, 90_000)
        );
        assert_eq!(c.colour, CANVAS_COLOUR);
        assert_eq!(c.layout, Layout::Fragmented);
        assert_eq!((c.created, c.max_fragment_samples), (0, 0));
        assert_eq!(m, Metadata::default());

        let (c, m) = parse_config(
            r#"{"width":640,"height":360,"fps":30,"layout":"faststart","max_samples":900,"max_sync_samples":16,
                "colour":{"primaries":"bt709","transfer":"bt709","matrix":"bt709","fullRange":false},
                "comment":"made with gource","build":"abc"}"#,
        )
        .unwrap();
        assert_eq!(c.colour, Colour::BT709_LIMITED);
        assert_eq!(
            c.layout,
            Layout::Faststart {
                max_samples: 900,
                max_sync_samples: 16
            }
        );
        assert_eq!(
            (m.comment.as_str(), m.build.as_str()),
            ("made with gource", "abc")
        );
        let (c, _) = parse_config(
            r#"{"width":2,"height":2,"fps":25,"layout":"faststart-deferred","colour":{"primaries":1,"transfer":13,"matrix":1,"full_range":true}}"#,
        )
        .unwrap();
        assert_eq!(c.colour, CANVAS_COLOUR);
        assert_eq!(c.layout, Layout::FaststartDeferred);
    }

    #[test]
    fn bad_configs_are_rejected() {
        for bad in [
            "[]",
            r#"{"height":1080,"fps":60}"#,
            r#"{"width":1.5,"height":1080,"fps":60}"#,
            r#"{"width":1920,"height":1080,"fps":60,"layout":"flat"}"#,
            r#"{"width":1920,"height":1080,"fps":60,"layout":"faststart"}"#,
            r#"{"width":1920,"height":1080,"fps":60,"colour":{"primaries":"p3"}}"#,
            r#"{"width":1920,"height":1080,"fps":60,"colour":{"full_range":1}}"#,
            r#"{"width":1920,"height":1080,"fps":60,"colour":7}"#,
        ] {
            assert_eq!(
                parse_config(bad).unwrap_err().code,
                codes::INVALID_OPTION,
                "{bad}"
            );
        }
        // a malformed avcC is a muxer error
        let err = Muxer::new(r#"{"width":64,"height":36,"fps":30}"#, &[0, 1, 2], None)
            .err()
            .unwrap();
        assert_eq!(err.code, "record_mux_failed");
    }

    /// The `avcC` payload of the x264 test vector of `gource-mux` (64x36).
    fn avcc() -> Vec<u8> {
        let file = include_bytes!("../../../gource-mux/tests/data/x264-high-64x36.mp4");
        let at = file.windows(4).position(|w| w == b"avcC").unwrap();
        let size = u32::from_be_bytes(file[at - 4..at].try_into().unwrap()) as usize;
        file[at + 4..at - 4 + size].to_vec()
    }

    #[test]
    fn chunks_go_through_to_the_output() {
        let config = r#"{"width":64,"height":36,"fps":30}"#;
        let mut m = Muxer::new(config, &avcc(), Some(br#"{"v":1}"#.to_vec())).unwrap();
        let mut file = Vec::new();
        let header = m.pending();
        assert!(header > 0);
        file.extend_from_slice(m.take(header));
        for i in 0..90u32 {
            let pts = f64::from(i) * 1e6 / 30.0;
            let n = m
                .push(&[0, 0, 0, 2, 0x65, i as u8], pts.round(), 0.0, i % 30 == 0)
                .unwrap();
            // a fragment leaves when the next GOP starts
            assert_eq!(n > 0, i == 30 || i == 60, "chunk {i}");
            let half = m.take(n / 2).to_vec();
            file.extend_from_slice(&half);
            let rest = m.pending();
            file.extend_from_slice(m.take(rest));
        }
        let tail = m.finish().unwrap();
        file.extend_from_slice(m.take(tail));
        assert_eq!(file.len() as u64, m.writer().bytes_written());
        assert_eq!(m.writer().frames(), 90);
        assert_eq!(m.writer().duration_us(), 3_000_000);
        assert!(!m.patches().is_empty());
        assert_eq!(m.placement_value().to_json(), r#"{"kind":"append"}"#);
        let err = m.push(&[0], 4e6, 0.0, true).unwrap_err();
        assert_eq!(err.code, "record_mux_failed");

        // timestamps that go backwards are a reorder error
        let mut m = Muxer::new(config, &avcc(), None).unwrap();
        m.push(&[1], 0.0, 0.0, true).unwrap();
        m.push(&[1], 33_333.0, 0.0, false).unwrap();
        assert_eq!(
            m.push(&[1], 10.0, 0.0, false).unwrap_err().code,
            "record_mux_reorder"
        );
        assert_eq!(
            m.push(&[1], f64::NAN, 0.0, false).unwrap_err().code,
            "record_mux_failed"
        );
    }
}
