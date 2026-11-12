// SPDX-License-Identifier: GPL-3.0-or-later

//! Encoder configuration (`recording.md` section 5.6): H.264 level and
//! bitrate selection (REC-50), the size checks that depend on the frame rate
//! (REC-44), the "match window" size (REC-6) and the support ladder the page
//! walks with `VideoEncoder.isConfigSupported` (REC-52).

use crate::record::json::{Value, number, object, string};
use crate::record::options::{
    Hardware, MAX_MACROBLOCKS, MAX_SIDE, Quality, macroblocks, size_problem,
};

/// One row of H.264 Table A-1, with the High-profile bitrate limit (1.25
/// times the Main limit).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Level {
    /// `level_idc`: 31 is level 3.1, 40 is 4.0, ...
    pub idc: u8,
    pub max_mb_per_s: u32,
    pub max_frame_mb: u32,
    pub max_kbps_high: u32,
}

const fn level(idc: u8, max_mb_per_s: u32, max_frame_mb: u32, max_kbps_high: u32) -> Level {
    Level {
        idc,
        max_mb_per_s,
        max_frame_mb,
        max_kbps_high,
    }
}

/// Levels the recorder selects from; below 3.1 is never needed (a decoder
/// accepts a stream at a higher level than it uses).
pub const LEVELS: [Level; 8] = [
    level(31, 108_000, 3_600, 17_500),
    level(32, 216_000, 5_120, 25_000),
    level(40, 245_760, 8_192, 25_000),
    level(41, 245_760, 8_192, 62_500),
    level(42, 522_240, 8_704, 62_500),
    level(50, 589_824, 22_080, 168_750),
    level(51, 983_040, 36_864, 300_000),
    level(52, 2_073_600, 36_864, 300_000),
];

/// A level's bitrate limit is used up to this share, so 4.0 gives way to
/// 4.1 above 22.5 Mbps.
const LEVEL_BITRATE_SHARE: f64 = 0.9;

/// The smallest level whose frame size, macroblock rate and bitrate (at
/// most 90 % of its High limit) all fit; `None` when even 5.2 does not.
pub fn select_level(width: u32, height: u32, fps: u32, bitrate_bps: u64) -> Option<Level> {
    let mb = macroblocks(width, height);
    let rate = u64::from(mb) * u64::from(fps);
    LEVELS.into_iter().find(|l| {
        mb <= l.max_frame_mb
            && rate <= u64::from(l.max_mb_per_s)
            && bitrate_bps as f64 <= f64::from(l.max_kbps_high) * 1000.0 * LEVEL_BITRATE_SHARE
    })
}

/// H.264 profiles the ladder uses.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Profile {
    High,
    Main,
}

impl Profile {
    fn prefix(self) -> &'static str {
        match self {
            Profile::High => "avc1.6400",
            Profile::Main => "avc1.4d00",
        }
    }

    fn name(self) -> &'static str {
        match self {
            Profile::High => "high",
            Profile::Main => "main",
        }
    }
}

/// The WebCodecs codec string, such as `avc1.640028` for High at level 4.0.
pub fn codec_string(profile: Profile, level_idc: u8) -> String {
    format!("{}{level_idc:02x}", profile.prefix())
}

/// `standard` bitrate anchors at 30 fps or less: (pixels, Mbps), YouTube's
/// SDR upload recommendations.
const ANCHORS: [(f64, f64); 4] = [
    (921_600.0, 5.0),
    (2_073_600.0, 8.0),
    (3_686_400.0, 16.0),
    (8_294_400.0, 40.0),
];
const MIN_MBPS: f64 = 1.0;
const MAX_MBPS: f64 = 100.0;

/// `interp(pixels) * q * f` in bits per second: linear between the anchors,
/// proportional below 720p and above 4K, `q` from the quality, `f` = 1.5
/// above 30 fps; clamped to 1..100 Mbps.
pub fn preset_bitrate_bps(width: u32, height: u32, fps: u32, quality: Quality) -> u64 {
    let px = f64::from(width) * f64::from(height);
    let (first, last) = (ANCHORS[0], ANCHORS[ANCHORS.len() - 1]);
    let mbps = if px <= first.0 {
        first.1 * px / first.0
    } else if px >= last.0 {
        last.1 * px / last.0
    } else {
        let i = ANCHORS.iter().position(|a| a.0 >= px).unwrap_or(1);
        let (a, b) = (ANCHORS[i - 1], ANCHORS[i]);
        a.1 + (b.1 - a.1) * (px - a.0) / (b.0 - a.0)
    };
    let fps_factor = if fps > 30 { 1.5 } else { 1.0 };
    let mbps = (mbps * quality.factor() * fps_factor).clamp(MIN_MBPS, MAX_MBPS);
    (mbps * 1e6).round() as u64
}

/// The bitrate to use: `record-bitrate` when set, else the preset.
pub fn bitrate_bps(width: u32, height: u32, fps: u32, quality: Quality, override_mbps: f32) -> u64 {
    if override_mbps > 0.0 {
        (f64::from(override_mbps) * 1e6).round() as u64
    } else {
        preset_bitrate_bps(width, height, fps, quality)
    }
}

/// Why `width x height` at `fps` cannot be recorded, or `None` (REC-44):
/// the static limits, then the macroblock rate of level 5.2.
pub fn size_problem_at(width: u32, height: u32, fps: u32) -> Option<String> {
    if let Some(p) = size_problem(width, height) {
        return Some(p);
    }
    let rate = u64::from(macroblocks(width, height)) * u64::from(fps);
    if rate > u64::from(LEVELS[LEVELS.len() - 1].max_mb_per_s) {
        return Some(format!(
            "{width}x{height} is not supported at {fps} fps; use 30 fps or a smaller size"
        ));
    }
    None
}

/// Largest even value not above `v`.
fn even_floor(v: f64) -> u32 {
    let v = v.max(0.0).floor() as u32;
    v - v % 2
}

/// "Match window" (REC-6): the live drawable rounded down to even sides,
/// then scaled down with its aspect (sides rounded down to even) until it
/// fits every limit at `fps`. Returns the size and the scale factor, which
/// multiplies the live device pixel ratio to give `ui_scale`.
pub fn match_window(device_w: u32, device_h: u32, fps: u32) -> Option<(u32, u32, f64)> {
    let (w, h) = (device_w - device_w % 2, device_h - device_h % 2);
    if w == 0 || h == 0 {
        return None;
    }
    let (wf, hf) = (f64::from(w), f64::from(h));
    let max_rate = f64::from(LEVELS[LEVELS.len() - 1].max_mb_per_s);
    // first guess from the side and area limits (macroblocks ~ area / 256)
    let mut f: f64 = 1.0f64
        .min(f64::from(MAX_SIDE) / wf)
        .min(f64::from(MAX_SIDE) / hf)
        .min((f64::from(MAX_MACROBLOCKS) * 256.0 / (wf * hf)).sqrt())
        .min((max_rate / f64::from(fps.max(1)) * 256.0 / (wf * hf)).sqrt());
    // then shrink until the rounded size passes (padding to 16 can push it over)
    for _ in 0..64 {
        let (sw, sh) = if f >= 1.0 {
            (w, h)
        } else {
            (even_floor(wf * f), even_floor(hf * f))
        };
        if sw < 2 || sh < 2 {
            return None;
        }
        let rate = u64::from(macroblocks(sw, sh)) * u64::from(fps);
        if sw <= MAX_SIDE
            && sh <= MAX_SIDE
            && macroblocks(sw, sh) <= MAX_MACROBLOCKS
            && rate as f64 <= max_rate
        {
            return Some((sw, sh, f.min(1.0)));
        }
        f *= 0.995;
    }
    None
}

/// One configuration for `VideoEncoder.isConfigSupported`, in ladder order.
#[derive(Clone, Debug, PartialEq)]
pub struct EncoderCandidate {
    pub codec: String,
    pub profile: Profile,
    pub level_idc: u8,
    pub width: u32,
    pub height: u32,
    pub bitrate_bps: u64,
    pub framerate: u32,
    pub hardware_acceleration: &'static str,
    /// `ui_scale` to record with at this size (it follows the size).
    pub ui_scale: f32,
    /// Plain-language difference from the request; `None` for the request.
    pub note: Option<String>,
}

impl EncoderCandidate {
    pub fn to_value(&self) -> Value {
        object([
            ("codec", string(&self.codec)),
            ("profile", string(self.profile.name())),
            ("level_idc", number(f64::from(self.level_idc))),
            ("width", number(f64::from(self.width))),
            ("height", number(f64::from(self.height))),
            ("bitrate", number(self.bitrate_bps as f64)),
            ("framerate", number(f64::from(self.framerate))),
            ("hardware_acceleration", string(self.hardware_acceleration)),
            ("avc_format", string("avc")),
            ("bitrate_mode", string("variable")),
            ("latency_mode", string("quality")),
            ("ui_scale", number(f64::from(self.ui_scale))),
            ("note", self.note.as_deref().map_or(Value::Null, string)),
        ])
    }
}

/// What the ladder starts from.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LadderRequest {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub ui_scale: f32,
    pub quality: Quality,
    pub bitrate_override_mbps: f32,
    pub hardware: Hardware,
}

/// The support ladder of REC-52, steps 1 to 4: High at the requested size
/// with the requested hardware preference; the other preferences; Main;
/// then the same at 75 % and 50 % of the size (same aspect, even sides,
/// `ui_scale` scaled with the size). Sizes or rates no level allows are
/// skipped. The page takes the first candidate `isConfigSupported` accepts.
pub fn ladder(req: &LadderRequest) -> Vec<EncoderCandidate> {
    let requested = req.hardware.webcodecs();
    let alternatives: Vec<&'static str> = ["prefer-software", "prefer-hardware", "no-preference"]
        .into_iter()
        .filter(|h| *h != requested)
        .take(2)
        .collect();
    let mut out = Vec::new();
    for scale in [1.0f64, 0.75, 0.5] {
        let (w, h) = if scale == 1.0 {
            (req.width, req.height)
        } else {
            (
                even_floor(f64::from(req.width) * scale),
                even_floor(f64::from(req.height) * scale),
            )
        };
        if size_problem_at(w, h, req.fps).is_some() {
            continue;
        }
        let bitrate = bitrate_bps(w, h, req.fps, req.quality, req.bitrate_override_mbps);
        let Some(level) = select_level(w, h, req.fps, bitrate) else {
            continue;
        };
        let ui_scale = req.ui_scale * scale as f32;
        let size_note = (scale != 1.0).then(|| {
            format!(
                "H.264 is not available at {}x{} here; using {w}x{h} instead",
                req.width, req.height
            )
        });
        let mut push = |profile: Profile, hardware: &'static str, note: Option<String>| {
            out.push(EncoderCandidate {
                codec: codec_string(profile, level.idc),
                profile,
                level_idc: level.idc,
                width: w,
                height: h,
                bitrate_bps: bitrate,
                framerate: req.fps,
                hardware_acceleration: hardware,
                ui_scale,
                note,
            });
        };
        push(Profile::High, requested, size_note.clone());
        for alt in &alternatives {
            let note = match &size_note {
                Some(n) => format!("{n} ({})", hardware_words(alt)),
                None => format!(
                    "H.264 with {} is not available at {w}x{h} here; using {}",
                    hardware_words(requested),
                    hardware_words(alt)
                ),
            };
            push(Profile::High, alt, Some(note));
        }
        let main_note = match &size_note {
            Some(n) => format!("{n} (H.264 Main profile)"),
            None => format!("H.264 High is not available at {w}x{h} here; using the Main profile"),
        };
        push(Profile::Main, requested, Some(main_note));
    }
    out
}

fn hardware_words(h: &str) -> &'static str {
    match h {
        "prefer-hardware" => "hardware encoding",
        "prefer-software" => "software encoding",
        _ => "automatic encoder choice",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn level_of(w: u32, h: u32, fps: u32, mbps: f64) -> Option<u8> {
        select_level(w, h, fps, (mbps * 1e6) as u64).map(|l| l.idc)
    }

    /// T-8: the worked results of section 5.6.
    #[test]
    fn level_vectors() {
        let high = |w, h, fps| bitrate_bps(w, h, fps, Quality::High, 0.0) as f64 / 1e6;
        let rows: &[(u32, u32, u32, u8)] = &[
            (1280, 720, 30, 31),
            (1280, 720, 60, 32),
            (1920, 1080, 30, 40),
            (1920, 1080, 60, 42),
            (2560, 1440, 30, 50),
            (2560, 1440, 60, 51),
            (3840, 2160, 30, 51),
            (3840, 2160, 60, 52),
            (1080, 1080, 30, 32),
            (1080, 1920, 30, 40),
            (1080, 1920, 60, 42),
            (4096, 2304, 30, 52),
        ];
        for &(w, h, fps, idc) in rows {
            assert_eq!(
                level_of(w, h, fps, high(w, h, fps)),
                Some(idc),
                "{w}x{h}@{fps}"
            );
        }
        assert_eq!(level_of(1920, 1080, 30, 30.0), Some(41));
        assert_eq!(level_of(1920, 1080, 30, 100.0), Some(50));
        assert_eq!(level_of(4096, 2304, 60, 1.0), None);
        assert_eq!(codec_string(Profile::High, 40), "avc1.640028");
        assert_eq!(codec_string(Profile::High, 42), "avc1.64002a");
        assert_eq!(codec_string(Profile::Main, 31), "avc1.4d001f");
        // the spike's recommended table, at the standard preset
        let std = |w, h, fps| bitrate_bps(w, h, fps, Quality::Standard, 0.0);
        assert_eq!(std(1280, 720, 30), 5_000_000);
        assert_eq!(std(1280, 720, 60), 7_500_000);
        assert_eq!(std(1920, 1080, 30), 8_000_000);
        assert_eq!(std(1920, 1080, 60), 12_000_000);
        assert_eq!(std(2560, 1440, 60), 24_000_000);
        assert_eq!(std(3840, 2160, 30), 40_000_000);
        assert_eq!(std(3840, 2160, 60), 60_000_000);
        assert_eq!(std(640, 360, 30), 1_250_000);
        assert_eq!(std(128, 128, 30), 1_000_000);
        assert_eq!(bitrate_bps(4096, 2304, 60, Quality::Max, 0.0), 100_000_000);
        assert_eq!(bitrate_bps(1920, 1080, 30, Quality::Max, 0.0), 20_000_000);
        assert_eq!(bitrate_bps(1920, 1080, 30, Quality::Max, 12.5), 12_500_000);
    }

    /// REC-50 properties: the level never decreases as size or fps grows,
    /// the bitrate is within 90 % of the level's limit, every row is
    /// reachable.
    #[test]
    fn level_properties() {
        let sizes = [
            (128, 128),
            (640, 360),
            (1280, 720),
            (1080, 1080),
            (1920, 1080),
            (2560, 1440),
            (3840, 2160),
            (4096, 2304),
        ];
        let mut reached = std::collections::BTreeSet::new();
        for q in [Quality::Standard, Quality::High, Quality::Max] {
            let mut prev_by_fps = [0u8; 4];
            for &(w, h) in &sizes {
                let mut prev = 0;
                for (i, fps) in [24, 25, 30, 60].into_iter().enumerate() {
                    let b = bitrate_bps(w, h, fps, q, 0.0);
                    let Some(l) = select_level(w, h, fps, b) else {
                        assert!(size_problem_at(w, h, fps).is_some(), "{w}x{h}@{fps}");
                        continue;
                    };
                    assert!(l.idc >= prev, "{w}x{h}: {fps} fps lowers the level");
                    assert!(
                        l.idc >= prev_by_fps[i],
                        "{w}x{h}@{fps}: a larger size lowers the level"
                    );
                    assert!(b as f64 <= f64::from(l.max_kbps_high) * 900.0);
                    prev = l.idc;
                    prev_by_fps[i] = l.idc;
                    reached.insert(l.idc);
                }
            }
        }
        // 4.1 is reached through a bitrate override only
        reached.insert(level_of(1920, 1080, 30, 30.0).unwrap());
        assert_eq!(reached.len(), LEVELS.len(), "{reached:?}");
    }

    #[test]
    fn rate_limit_rejects_4096_at_60() {
        assert!(size_problem_at(4096, 2304, 30).is_none());
        assert!(size_problem_at(4096, 2304, 60).unwrap().contains("30 fps"));
        assert!(size_problem_at(3840, 2160, 60).is_none());
        assert!(size_problem_at(1921, 1080, 30).is_some());
    }

    /// T-8: match window vectors of REC-6.
    #[test]
    fn match_window_vectors() {
        assert_eq!(match_window(1457, 817, 30), Some((1456, 816, 1.0)));
        assert_eq!(match_window(1251, 703, 30), Some((1250, 702, 1.0)));
        let (w, h, f) = match_window(5120, 2880, 30).unwrap();
        assert_eq!((w, h), (4096, 2304));
        assert!((f - 0.8).abs() < 1e-9, "{f}");
        // at 60 fps level 5.2's macroblock rate also limits the size
        let (w, h, f) = match_window(5120, 2880, 60).unwrap();
        assert!(size_problem_at(w, h, 60).is_none(), "{w}x{h}");
        assert!(w < 4096 && f < 0.8, "{w}x{h} {f}");
        assert!((f64::from(w) / f64::from(h) - 16.0 / 9.0).abs() < 0.01);
        for (dw, dh) in [(3000, 3000), (8000, 1000), (4097, 4097), (1, 1)] {
            if let Some((w, h, _)) = match_window(dw, dh, 60) {
                assert!(
                    size_problem_at(w, h, 60).is_none() || w < 128 || h < 128,
                    "{dw}x{dh}"
                );
            }
        }
    }

    /// T-10 (native part): the ladder order of REC-52.
    #[test]
    fn ladder_order() {
        let req = LadderRequest {
            width: 3840,
            height: 2160,
            fps: 30,
            ui_scale: 2.0,
            quality: Quality::High,
            bitrate_override_mbps: 0.0,
            hardware: Hardware::Auto,
        };
        let l = ladder(&req);
        let summary: Vec<(String, u32, &str)> = l
            .iter()
            .map(|c| (c.codec.clone(), c.width, c.hardware_acceleration))
            .collect();
        assert_eq!(
            summary[..4],
            [
                ("avc1.640033".to_string(), 3840, "no-preference"),
                ("avc1.640033".to_string(), 3840, "prefer-software"),
                ("avc1.640033".to_string(), 3840, "prefer-hardware"),
                ("avc1.4d0033".to_string(), 3840, "no-preference"),
            ]
        );
        assert_eq!(l.len(), 12);
        assert!(l[0].note.is_none());
        assert!(l[1..].iter().all(|c| c.note.is_some()));
        assert_eq!((l[4].width, l[4].height, l[4].ui_scale), (2880, 1620, 1.5));
        assert_eq!((l[8].width, l[8].height, l[8].ui_scale), (1920, 1080, 1.0));
        assert_eq!(l[8].codec, "avc1.640028");
        assert!(l[4].note.as_deref().unwrap().contains("2880x1620"));

        let hw = ladder(&LadderRequest {
            hardware: Hardware::PreferHardware,
            ..req
        });
        let prefs: Vec<&str> = hw[..3].iter().map(|c| c.hardware_acceleration).collect();
        assert_eq!(
            prefs,
            ["prefer-hardware", "prefer-software", "no-preference"]
        );
        assert!(
            hw[1]
                .note
                .as_deref()
                .unwrap()
                .contains("hardware encoding is not available")
        );

        // a size with no level at the requested rate starts lower
        let big = ladder(&LadderRequest {
            width: 4096,
            height: 2304,
            fps: 60,
            ..req
        });
        assert_eq!(big[0].width, 3072);
    }
}
