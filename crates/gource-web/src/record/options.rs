// SPDX-License-Identifier: GPL-3.0-or-later

//! Recording options (`recording.md` section 5.2): the `record-*` names, a
//! deliberate exception to the rule that options go through
//! `Simulation::set_option`. They describe the video, not the run, and must
//! never reach the live simulation or the parity tests, so they live here
//! with the same strict string-in, string-out contract as Gource's options:
//! [`RecordOptions::apply`] validates one value, [`RecordOptions::value`]
//! reports it in a form `apply` accepts unchanged, and an empty value resets
//! an option to its default (except `record-viewport`, where empty means
//! "match window").
//!
//! The TypeScript table `RECORD_OPTION_DEFS` mirrors [`RECORD_OPTION_DEFS`]
//! (names, defaults, ranges).

use std::fmt;

use crate::record::json::{Value, number, object, string};

/// Output frame rates. 25, 30 and 60 are Gource's `--output-framerate`
/// values; 24 is new (3 ticks at 72 Hz).
pub const FRAME_RATES: [u32; 4] = [24, 25, 30, 60];

/// Longest video, in seconds, whatever the options say (4 hours).
pub const HARD_MAX_SECONDS: u32 = 14_400;

/// Size limits checked by `record-viewport` (REC-44): sides even, 128 to
/// 4096 pixels, at most 36,864 coded macroblocks (H.264 level 5.x).
pub const MIN_SIDE: u32 = 128;
pub const MAX_SIDE: u32 = 4096;
pub const MAX_MACROBLOCKS: u32 = 36_864;

/// Default output size (1080p).
pub const DEFAULT_VIEWPORT: (u32, u32) = (1920, 1080);

/// Longest file name stem (REC-62).
pub const MAX_NAME_CHARS: usize = 80;

/// What the dialog offers for one option.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RecordOptionDef {
    pub name: &'static str,
    /// `value(name)` of the defaults.
    pub default: &'static str,
    /// The accepted words, for choice options (empty otherwise).
    pub choices: &'static [&'static str],
    /// Inclusive numeric range, for number options; extra accepted values
    /// outside it are listed in `special`.
    pub range: Option<(f32, f32)>,
    pub special: &'static [&'static str],
}

/// Every recording option, in dialog order.
pub const RECORD_OPTION_DEFS: &[RecordOptionDef] = &[
    def(
        "record-framerate",
        "60",
        &["24", "25", "30", "60"],
        None,
        &[],
    ),
    def("record-viewport", "1920x1080", &[], None, &[""]),
    def("record-ui-scale", "auto", &[], Some((0.25, 4.0)), &["auto"]),
    def("record-format", "mp4", &["mp4"], None, &[]),
    def(
        "record-layout",
        "auto",
        &["auto", "fragmented", "faststart"],
        None,
        &[],
    ),
    def(
        "record-quality",
        "high",
        &["standard", "high", "max"],
        None,
        &[],
    ),
    def("record-bitrate", "0", &[], Some((0.5, 100.0)), &["0"]),
    def("record-keyframe-seconds", "2", &[], Some((0.5, 10.0)), &[]),
    def(
        "record-hardware",
        "auto",
        &["auto", "prefer-hardware", "prefer-software"],
        None,
        &[],
    ),
    def("record-tail-seconds", "5", &[], Some((0.0, 120.0)), &["-1"]),
    def(
        "record-max-seconds",
        "0",
        &[],
        Some((1.0, 14_400.0)),
        &["0"],
    ),
    def("record-metadata", "embed", &["embed", "none"], None, &[]),
    def("record-name", "", &[], None, &[]),
];

const fn def(
    name: &'static str,
    default: &'static str,
    choices: &'static [&'static str],
    range: Option<(f32, f32)>,
    special: &'static [&'static str],
) -> RecordOptionDef {
    RecordOptionDef {
        name,
        default,
        choices,
        range,
        special,
    }
}

/// [`RECORD_OPTION_DEFS`] as JSON: `[{name, default, choices, range,
/// special}]`, `range` being `[min, max]` or null.
pub fn defs_value() -> Value {
    Value::Array(
        RECORD_OPTION_DEFS
            .iter()
            .map(|d| {
                let words = |list: &[&str]| Value::Array(list.iter().map(|w| string(w)).collect());
                object([
                    ("name", string(d.name)),
                    ("default", string(d.default)),
                    ("choices", words(d.choices)),
                    (
                        "range",
                        d.range.map_or(Value::Null, |(lo, hi)| {
                            Value::Array(vec![number(f64::from(lo)), number(f64::from(hi))])
                        }),
                    ),
                    ("special", words(d.special)),
                ])
            })
            .collect(),
    )
}

/// Rejected recording option name or value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RecordOptionError {
    UnknownOption(String),
    InvalidValue {
        option: String,
        value: String,
        reason: String,
    },
}

impl fmt::Display for RecordOptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecordOptionError::UnknownOption(name) => {
                write!(f, "unknown recording option '{name}'")
            }
            RecordOptionError::InvalidValue {
                option,
                value,
                reason,
            } => write!(f, "invalid value '{value}' for '{option}': {reason}"),
        }
    }
}

impl std::error::Error for RecordOptionError {}

/// `record-quality`: the bitrate preset (REC-50).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Quality {
    Standard,
    High,
    Max,
}

impl Quality {
    /// Multiplier of the `standard` bitrate.
    pub fn factor(self) -> f64 {
        match self {
            Quality::Standard => 1.0,
            Quality::High => 1.5,
            Quality::Max => 2.5,
        }
    }
}

/// `record-hardware`, mapped to WebCodecs `hardwareAcceleration`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Hardware {
    Auto,
    PreferHardware,
    PreferSoftware,
}

impl Hardware {
    /// The WebCodecs `HardwareAcceleration` value.
    pub fn webcodecs(self) -> &'static str {
        match self {
            Hardware::Auto => "no-preference",
            Hardware::PreferHardware => "prefer-hardware",
            Hardware::PreferSoftware => "prefer-software",
        }
    }
}

/// `record-layout` (REC-59).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum LayoutChoice {
    Auto,
    Fragmented,
    Faststart,
}

/// The parsed recording options.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordOptions {
    pub framerate: u32,
    /// `None`: match the live window (resolved by the REC-6 rule).
    pub viewport: Option<(u32, u32)>,
    /// `None`: `auto` (REC-38).
    pub ui_scale: Option<f32>,
    pub layout: LayoutChoice,
    pub quality: Quality,
    /// Mbps; 0 takes the quality preset.
    pub bitrate_mbps: f32,
    pub keyframe_seconds: f32,
    pub hardware: Hardware,
    /// Video seconds after the last commit; -1 runs until every user has
    /// left (Gource's exporter quits then).
    pub tail_seconds: f32,
    /// 0: no limit other than [`HARD_MAX_SECONDS`].
    pub max_seconds: f32,
    /// `record-metadata=embed`.
    pub embed_metadata: bool,
    /// File name stem; empty derives it from the source (REC-62).
    pub name: String,
}

impl Default for RecordOptions {
    fn default() -> Self {
        RecordOptions {
            framerate: 60,
            viewport: Some(DEFAULT_VIEWPORT),
            ui_scale: None,
            layout: LayoutChoice::Auto,
            quality: Quality::High,
            bitrate_mbps: 0.0,
            keyframe_seconds: 2.0,
            hardware: Hardware::Auto,
            tail_seconds: 5.0,
            max_seconds: 0.0,
            embed_metadata: true,
            name: String::new(),
        }
    }
}

struct Ctx<'a> {
    option: &'a str,
    value: &'a str,
}

impl Ctx<'_> {
    fn invalid(&self, reason: &str) -> RecordOptionError {
        RecordOptionError::InvalidValue {
            option: self.option.to_string(),
            value: self.value.to_string(),
            reason: reason.to_string(),
        }
    }

    /// A finite number in `lo..=hi`, or one of `special`.
    fn number(
        &self,
        lo: f32,
        hi: f32,
        special: &[f32],
        reason: &str,
    ) -> Result<f32, RecordOptionError> {
        let v: f32 = self
            .value
            .trim()
            .parse()
            .map_err(|_| self.invalid("expected a number"))?;
        if v.is_finite() && ((lo..=hi).contains(&v) || special.contains(&v)) {
            // -0 reads back as "0"
            Ok(v + 0.0)
        } else {
            Err(self.invalid(reason))
        }
    }

    fn choice<T: Copy>(&self, table: &[(&str, T)]) -> Result<T, RecordOptionError> {
        let words: Vec<&str> = table.iter().map(|(w, _)| *w).collect();
        table
            .iter()
            .find(|(w, _)| *w == self.value.trim())
            .map(|(_, v)| *v)
            .ok_or_else(|| self.invalid(&format!("expected one of {}", words.join(", "))))
    }
}

const QUALITIES: [(&str, Quality); 3] = [
    ("standard", Quality::Standard),
    ("high", Quality::High),
    ("max", Quality::Max),
];
const HARDWARE: [(&str, Hardware); 3] = [
    ("auto", Hardware::Auto),
    ("prefer-hardware", Hardware::PreferHardware),
    ("prefer-software", Hardware::PreferSoftware),
];
const LAYOUTS: [(&str, LayoutChoice); 3] = [
    ("auto", LayoutChoice::Auto),
    ("fragmented", LayoutChoice::Fragmented),
    ("faststart", LayoutChoice::Faststart),
];
const METADATA: [(&str, bool); 2] = [("embed", true), ("none", false)];

fn word<T: PartialEq>(table: &[(&'static str, T)], v: &T) -> String {
    table
        .iter()
        .find(|(_, t)| t == v)
        .map_or("", |(w, _)| w)
        .to_string()
}

/// `f32` as the shortest text that parses back to it.
fn fmt_f32(v: f32) -> String {
    format!("{v}")
}

/// Coded macroblocks of a frame: `ceil(w / 16) * ceil(h / 16)`.
pub fn macroblocks(width: u32, height: u32) -> u32 {
    width.div_ceil(16) * height.div_ceil(16)
}

/// Why a size cannot be recorded at any frame rate (REC-44), or `None`.
pub fn size_problem(width: u32, height: u32) -> Option<String> {
    if !width.is_multiple_of(2) || !height.is_multiple_of(2) {
        return Some("width and height must be even".into());
    }
    if width < MIN_SIDE || height < MIN_SIDE {
        return Some(format!("each side must be at least {MIN_SIDE} pixels"));
    }
    if width > MAX_SIDE || height > MAX_SIDE {
        return Some(format!("each side must be at most {MAX_SIDE} pixels"));
    }
    if macroblocks(width, height) > MAX_MACROBLOCKS {
        return Some("the frame is larger than H.264 allows (for example 4096x2304)".into());
    }
    None
}

/// Parse `WIDTHxHEIGHT` (positive integers, `x` or `X`).
pub fn parse_size(s: &str) -> Option<(u32, u32)> {
    let (w, h) = s.trim().split_once(['x', 'X'])?;
    let digits = |t: &str| !t.is_empty() && t.len() <= 5 && t.bytes().all(|b| b.is_ascii_digit());
    if !digits(w) || !digits(h) {
        return None;
    }
    let (w, h) = (w.parse().ok()?, h.parse().ok()?);
    (w > 0 && h > 0).then_some((w, h))
}

/// `record-name`: characters outside `[A-Za-z0-9._-]` become `-`, at most
/// [`MAX_NAME_CHARS`] characters (REC-62).
pub fn sanitise_name(s: &str) -> String {
    s.trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .take(MAX_NAME_CHARS)
        .collect()
}

impl RecordOptions {
    /// True for a name [`RecordOptions::apply`] knows.
    pub fn is_record_option(name: &str) -> bool {
        RECORD_OPTION_DEFS.iter().any(|d| d.name == name)
    }

    /// Every recording option name, in dialog order.
    pub fn names() -> impl Iterator<Item = &'static str> {
        RECORD_OPTION_DEFS.iter().map(|d| d.name)
    }

    /// Apply one option; on error the options are unchanged.
    pub fn apply(&mut self, name: &str, value: &str) -> Result<(), RecordOptionError> {
        let ctx = Ctx {
            option: name,
            value,
        };
        let defaults = RecordOptions::default();
        let empty = value.trim().is_empty();
        match name {
            "record-framerate" => {
                self.framerate = if empty {
                    defaults.framerate
                } else {
                    value
                        .trim()
                        .parse::<u32>()
                        .ok()
                        .filter(|f| FRAME_RATES.contains(f))
                        .ok_or_else(|| ctx.invalid("expected 24, 25, 30 or 60"))?
                };
            }
            "record-viewport" => {
                self.viewport = if empty {
                    None
                } else {
                    let (w, h) = parse_size(value)
                        .ok_or_else(|| ctx.invalid("expected WIDTHxHEIGHT, such as 1920x1080"))?;
                    if let Some(reason) = size_problem(w, h) {
                        return Err(ctx.invalid(&reason));
                    }
                    Some((w, h))
                };
            }
            "record-ui-scale" => {
                self.ui_scale = if empty || value.trim() == "auto" {
                    None
                } else {
                    Some(ctx.number(0.25, 4.0, &[], "expected auto or a number from 0.25 to 4")?)
                };
            }
            "record-format" => {
                if !empty && value.trim() != "mp4" {
                    return Err(ctx.invalid("expected mp4 (H.264)"));
                }
            }
            "record-layout" => {
                self.layout = if empty {
                    defaults.layout
                } else {
                    ctx.choice(&LAYOUTS)?
                };
            }
            "record-quality" => {
                self.quality = if empty {
                    defaults.quality
                } else {
                    ctx.choice(&QUALITIES)?
                };
            }
            "record-bitrate" => {
                self.bitrate_mbps = if empty {
                    defaults.bitrate_mbps
                } else {
                    ctx.number(
                        0.5,
                        100.0,
                        &[0.0],
                        "expected 0 (from quality) or 0.5 to 100 Mbps",
                    )?
                };
            }
            "record-keyframe-seconds" => {
                self.keyframe_seconds = if empty {
                    defaults.keyframe_seconds
                } else {
                    ctx.number(0.5, 10.0, &[], "expected 0.5 to 10 seconds")?
                };
            }
            "record-hardware" => {
                self.hardware = if empty {
                    defaults.hardware
                } else {
                    ctx.choice(&HARDWARE)?
                };
            }
            "record-tail-seconds" => {
                self.tail_seconds = if empty {
                    defaults.tail_seconds
                } else {
                    ctx.number(0.0, 120.0, &[-1.0], "expected -1 or 0 to 120 seconds")?
                };
            }
            "record-max-seconds" => {
                self.max_seconds = if empty {
                    defaults.max_seconds
                } else {
                    ctx.number(
                        1.0,
                        HARD_MAX_SECONDS as f32,
                        &[0.0],
                        "expected 0 (no limit) or 1 to 14400 seconds",
                    )?
                };
            }
            "record-metadata" => {
                self.embed_metadata = if empty {
                    defaults.embed_metadata
                } else {
                    ctx.choice(&METADATA)?
                };
            }
            "record-name" => self.name = sanitise_name(value),
            _ => return Err(RecordOptionError::UnknownOption(name.to_string())),
        }
        Ok(())
    }

    /// The current value in the syntax [`RecordOptions::apply`] accepts
    /// (`None` for an unknown name).
    pub fn value(&self, name: &str) -> Option<String> {
        Some(match name {
            "record-framerate" => self.framerate.to_string(),
            "record-viewport" => self
                .viewport
                .map_or(String::new(), |(w, h)| format!("{w}x{h}")),
            "record-ui-scale" => self.ui_scale.map_or("auto".into(), fmt_f32),
            "record-format" => "mp4".into(),
            "record-layout" => word(&LAYOUTS, &self.layout),
            "record-quality" => word(&QUALITIES, &self.quality),
            "record-bitrate" => fmt_f32(self.bitrate_mbps),
            "record-keyframe-seconds" => fmt_f32(self.keyframe_seconds),
            "record-hardware" => word(&HARDWARE, &self.hardware),
            "record-tail-seconds" => fmt_f32(self.tail_seconds),
            "record-max-seconds" => fmt_f32(self.max_seconds),
            "record-metadata" => word(&METADATA, &self.embed_metadata),
            "record-name" => self.name.clone(),
            _ => return None,
        })
    }

    /// Keyframe interval in frames (`round(record-keyframe-seconds * fps)`).
    pub fn keyframe_every(&self) -> u32 {
        ((f64::from(self.keyframe_seconds) * f64::from(self.framerate)).round() as u32).max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_the_table_defaults() {
        let o = RecordOptions::default();
        for d in RECORD_OPTION_DEFS {
            assert_eq!(o.value(d.name).as_deref(), Some(d.default), "{}", d.name);
        }
        assert_eq!(o.framerate, 60);
        assert_eq!(o.viewport, Some((1920, 1080)));
        assert_eq!(o.keyframe_every(), 120);
    }

    /// REC-30: every reported value applies back unchanged, for the
    /// defaults and for every accepted value.
    #[test]
    fn every_value_round_trips() {
        let cases: &[(&str, &str, &str)] = &[
            // (name, applied, reported)
            ("record-framerate", "24", "24"),
            ("record-framerate", " 25 ", "25"),
            ("record-framerate", "30", "30"),
            ("record-framerate", "", "60"),
            ("record-viewport", "1080x1920", "1080x1920"),
            ("record-viewport", "1080X1080", "1080x1080"),
            ("record-viewport", "4096x2304", "4096x2304"),
            ("record-viewport", "", ""),
            ("record-ui-scale", "2", "2"),
            ("record-ui-scale", "0.25", "0.25"),
            ("record-ui-scale", "", "auto"),
            ("record-format", "mp4", "mp4"),
            ("record-layout", "faststart", "faststart"),
            ("record-layout", "fragmented", "fragmented"),
            ("record-quality", "max", "max"),
            ("record-quality", "standard", "standard"),
            ("record-bitrate", "12.5", "12.5"),
            ("record-bitrate", "-0", "0"),
            ("record-keyframe-seconds", "0.5", "0.5"),
            ("record-hardware", "prefer-software", "prefer-software"),
            ("record-tail-seconds", "-1", "-1"),
            ("record-tail-seconds", "0", "0"),
            ("record-tail-seconds", "120", "120"),
            ("record-max-seconds", "60", "60"),
            ("record-max-seconds", "14400", "14400"),
            ("record-metadata", "none", "none"),
            ("record-name", "owner/repo v2", "owner-repo-v2"),
            ("record-name", "ok_name.1", "ok_name.1"),
        ];
        for &(name, applied, reported) in cases {
            let mut o = RecordOptions::default();
            o.apply(name, applied)
                .unwrap_or_else(|e| panic!("{name}={applied}: {e}"));
            let v = o.value(name).unwrap();
            assert_eq!(v, reported, "{name}={applied}");
            let mut again = o.clone();
            again.apply(name, &v).unwrap();
            assert_eq!(again, o, "{name}={v} does not apply back unchanged");
        }
        let d = RecordOptions::default();
        for name in RecordOptions::names() {
            let mut t = d.clone();
            t.apply(name, &d.value(name).unwrap()).unwrap();
            assert_eq!(t, d, "{name}");
        }
        assert_eq!(sanitise_name(&"x".repeat(200)).len(), MAX_NAME_CHARS);
    }

    #[test]
    fn invalid_values_are_rejected_with_a_reason() {
        let cases: &[(&str, &str)] = &[
            ("record-framerate", "50"),
            ("record-framerate", "30fps"),
            ("record-viewport", "1921x1080"),
            ("record-viewport", "64x64"),
            ("record-viewport", "4096x4096"),
            ("record-viewport", "8192x1080"),
            ("record-viewport", "1920*1080"),
            ("record-viewport", "-2x4"),
            ("record-ui-scale", "5"),
            ("record-ui-scale", "big"),
            ("record-format", "webm"),
            ("record-layout", "flat"),
            ("record-quality", "ultra"),
            ("record-bitrate", "0.1"),
            ("record-bitrate", "101"),
            ("record-bitrate", "NaN"),
            ("record-keyframe-seconds", "0"),
            ("record-hardware", "gpu"),
            ("record-tail-seconds", "-2"),
            ("record-tail-seconds", "121"),
            ("record-tail-seconds", "inf"),
            ("record-max-seconds", "0.5"),
            ("record-max-seconds", "14401"),
            ("record-metadata", "all"),
        ];
        for &(name, value) in cases {
            let mut o = RecordOptions::default();
            match o.apply(name, value) {
                Err(RecordOptionError::InvalidValue { reason, .. }) => {
                    assert!(!reason.is_empty());
                }
                other => panic!("{name}={value}: {other:?}"),
            }
            assert_eq!(
                o,
                RecordOptions::default(),
                "{name}={value} changed the options"
            );
        }
        assert_eq!(
            RecordOptions::default().apply("record-fps", "30"),
            Err(RecordOptionError::UnknownOption("record-fps".into()))
        );
        assert_eq!(RecordOptions::default().value("seconds-per-day"), None);
    }

    #[test]
    fn the_table_describes_the_parser() {
        for d in RECORD_OPTION_DEFS {
            let mut o = RecordOptions::default();
            for c in d.choices.iter().chain(d.special) {
                o.apply(d.name, c)
                    .unwrap_or_else(|e| panic!("{}={c}: {e}", d.name));
            }
            if let Some((lo, hi)) = d.range {
                o.apply(d.name, &lo.to_string()).unwrap();
                o.apply(d.name, &hi.to_string()).unwrap();
                assert!(
                    o.apply(d.name, &(hi + 1.0).to_string()).is_err(),
                    "{}",
                    d.name
                );
            }
        }
    }

    #[test]
    fn defs_json_lists_every_option() {
        let text = defs_value().to_json();
        assert!(text.starts_with(r#"[{"name":"record-framerate","default":"60","choices":["24","25","30","60"],"range":null,"special":[]}"#), "{text}");
        assert!(text.contains(r#"{"name":"record-tail-seconds","default":"5","choices":[],"range":[0,120],"special":["-1"]}"#), "{text}");
        assert!(text.contains(r#""range":[0.25,4]"#));
    }

    #[test]
    fn size_limits_follow_rec_44() {
        assert_eq!(macroblocks(1920, 1080), 8160);
        assert_eq!(macroblocks(4096, 2304), 36_864);
        assert!(size_problem(4096, 2304).is_none());
        assert!(size_problem(2304, 4096).is_none());
        assert!(size_problem(4096, 2306).is_some());
        assert!(size_problem(128, 128).is_none());
        assert!(size_problem(126, 128).is_some());
        assert_eq!(parse_size("640x360"), Some((640, 360)));
        assert_eq!(parse_size("0x360"), None);
        assert_eq!(parse_size("123456x1"), None);
    }
}
