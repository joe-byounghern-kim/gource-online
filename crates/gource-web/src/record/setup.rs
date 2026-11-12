// SPDX-License-Identifier: GPL-3.0-or-later

//! Everything a recording is made from, resolved once (`recording.md`
//! sections 5.2 to 5.6): the settings snapshot with the dialog's overrides
//! and the forced overrides, the output size and `ui_scale`, the frame plan,
//! the encoder choice, the estimate and the stop rules.
//!
//! The snapshot is the live simulation's settings
//! ([`Simulation::settings_snapshot`], which carries keyboard-changed state
//! such as speed, hide toggles and the hash seed, and the time-zone table),
//! then the dialog's overrides applied by Gource option name. The forced
//! overrides follow (REC-35, REC-38):
//!
//! - for an explicit size, `font_scale` is pinned to 1 unless the viewer set
//!   `font-scale` (Gource's `1 + height / 1600` rule would double the labels
//!   of a 1080x1920 frame); "match window" keeps the rule, as the live view
//!   does;
//! - `stop-at-end` is on unless `loop` is;
//! - `hide` gains `mouse` and `progress` (no visible effect in a recording,
//!   set so the snapshot is explicit).
//!
//! The recorder has no input path at all, which is what `disable-input`
//! would give.

use gource_core::events::EventLog;
use gource_core::settings::Settings;
use gource_core::sim::Simulation;

use crate::record::encoder::{
    EncoderCandidate, LadderRequest, Level, Profile, bitrate_bps, codec_string, ladder,
    match_window, select_level, size_problem_at,
};
use crate::record::estimate::{Estimate, estimate};
use crate::record::json::{self, Value, number, object, string};
use crate::record::options::{LayoutChoice, MAX_MACROBLOCKS, MAX_SIDE, MIN_SIDE, RecordOptions};
use crate::record::stop::StopRules;
use crate::record::timing::FramePlan;
use crate::record::{RecordError, codes};

/// The file size bound is the target bitrate times this: Chrome's software
/// H.264 encoder stays above a quality floor and produced up to 2.4 times
/// the `standard` target on dense, moving scenes (M0 spike).
pub const SIZE_BOUND_FACTOR: f64 = 2.5;

/// What "match window" resolves against: the live canvas in device pixels
/// and its device pixel ratio.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LiveView {
    pub device: [u32; 2],
    pub dpr: f64,
}

/// The dialog's request: Gource option overrides (by name, applied in
/// order) and the recording options.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecordRequest {
    pub overrides: Vec<(String, String)>,
    pub options: RecordOptions,
}

impl RecordRequest {
    /// From a JSON object of option names to values (strings, or numbers
    /// and booleans as written): `record-*` names are recording options,
    /// every other name a Gource option override. Recording options are
    /// validated here, Gource options by [`RecordSetup::resolve`].
    pub fn from_json(text: &str) -> Result<Self, RecordError> {
        let text = if text.trim().is_empty() { "{}" } else { text };
        let v = json::parse(text)
            .map_err(|e| RecordError::new(codes::INVALID_OPTION, e.to_string()))?;
        let members = v.members().ok_or_else(|| {
            RecordError::new(
                codes::INVALID_OPTION,
                "expected a JSON object of option names to values",
            )
        })?;
        let mut req = RecordRequest::default();
        for (name, value) in members {
            let value = value.as_option_text().ok_or_else(|| {
                RecordError::new(
                    codes::INVALID_OPTION,
                    format!("the value of '{name}' must be a string, number or boolean"),
                )
            })?;
            req.set(name, value)?;
        }
        Ok(req)
    }

    /// One option by name.
    pub fn set(&mut self, name: &str, value: &str) -> Result<(), RecordError> {
        if name.starts_with("record-") {
            self.options
                .apply(name, value)
                .map_err(|e| RecordError::new(codes::INVALID_OPTION, e.to_string()))
        } else {
            self.overrides.push((name.to_string(), value.to_string()));
            Ok(())
        }
    }
}

/// A resolved recording.
#[derive(Clone, Debug)]
pub struct RecordSetup {
    pub request: RecordRequest,
    /// The snapshot plus the dialog's overrides: what the recipe serialises
    /// and a reproduce link sets on the live view.
    pub recipe_settings: Settings,
    /// `recipe_settings` plus the forced overrides: the recorder's
    /// simulation settings.
    pub sim_settings: Settings,
    pub width: u32,
    pub height: u32,
    pub ui_scale: f32,
    /// The size came from the live window (REC-6).
    pub match_window: bool,
    pub plan: FramePlan,
    pub bitrate_bps: u64,
    pub level: Level,
    pub ladder: Vec<EncoderCandidate>,
    pub estimate: Estimate,
    pub stop: StopRules,
    pub warnings: Vec<&'static str>,
}

impl RecordSetup {
    /// Resolve `request` against the live settings snapshot `base`, the
    /// live view and the (complete) log.
    pub fn resolve(
        base: &Settings,
        request: &RecordRequest,
        live: LiveView,
        log: &EventLog,
    ) -> Result<Self, RecordError> {
        if !log.is_complete() {
            return Err(RecordError::new(
                codes::LOG_INCOMPLETE,
                "The full history has not arrived yet.",
            ));
        }
        let opts = &request.options;
        let fps = opts.framerate;

        let mut recipe_settings = base.clone();
        for (name, value) in &request.overrides {
            recipe_settings
                .apply_option(name, value)
                .map_err(|e| RecordError::new(codes::INVALID_OPTION, e.to_string()))?;
        }
        if recipe_settings.loop_playback && opts.max_seconds <= 0.0 {
            return Err(RecordError::new(
                codes::UNBOUNDED,
                "A looping recording needs a maximum length.",
            ));
        }

        let (width, height, ui_scale) = match opts.viewport {
            Some((w, h)) => {
                let auto = w.min(h) as f32 / 1080.0;
                (w, h, opts.ui_scale.unwrap_or(auto))
            }
            None => {
                let [dw, dh] = live.device;
                let (w, h, factor) = match_window(dw, dh, fps).ok_or_else(|| {
                    RecordError::new(
                        codes::SIZE_UNSUPPORTED,
                        "The window is too small to record.",
                    )
                })?;
                let dpr = if live.dpr.is_finite() && live.dpr > 0.0 {
                    live.dpr
                } else {
                    1.0
                };
                (w, h, opts.ui_scale.unwrap_or((dpr * factor) as f32))
            }
        };
        if let Some(reason) = size_problem_at(width, height, fps) {
            return Err(RecordError::new(codes::SIZE_UNSUPPORTED, reason));
        }

        let mut sim_settings = recipe_settings.clone();
        if opts.viewport.is_some() && sim_settings.default_font_scale {
            sim_settings.default_font_scale = false;
            sim_settings.font_scale = 1.0;
        }
        sim_settings.stop_at_end = !sim_settings.loop_playback;
        sim_settings.hide_mouse = true;
        sim_settings.hide_progress = true;

        let plan = FramePlan::new(fps, opts.keyframe_every());
        let bitrate = bitrate_bps(width, height, fps, opts.quality, opts.bitrate_mbps);
        let level = select_level(width, height, fps, bitrate).ok_or_else(|| {
            RecordError::new(
                codes::SIZE_UNSUPPORTED,
                format!(
                    "No H.264 level allows {width}x{height} at {fps} fps and {} Mbps.",
                    bitrate as f64 / 1e6
                ),
            )
        })?;
        let ladder = ladder(&LadderRequest {
            width,
            height,
            fps,
            ui_scale,
            quality: opts.quality,
            bitrate_override_mbps: opts.bitrate_mbps,
            hardware: opts.hardware,
        });
        let estimate = estimate(log, &sim_settings, fps, opts.tail_seconds, opts.max_seconds);
        let stop = StopRules::new(
            &plan,
            opts.tail_seconds,
            opts.max_seconds,
            sim_settings.loop_playback,
        );

        let mut warnings = Vec::new();
        if estimate.empty() {
            warnings.push("empty_run");
        }
        if estimate.upper_s > 3600.0 {
            warnings.push("long_render");
        }
        if estimate.upper_s > 600.0 {
            warnings.push("crash_loses_file");
        }
        let seconds_per_day = 1.0 / f64::from(sim_settings.days_per_second.max(f32::MIN_POSITIVE));
        if seconds_per_day < 0.05 {
            warnings.push("speed_very_fast");
        } else if seconds_per_day > 30.0 {
            warnings.push("speed_very_slow");
        }
        if fps > 30 && u64::from(width) * u64::from(height) >= 3840 * 2160 {
            warnings.push("software_encoder_slow");
        }
        if sim_settings.time_scale < 1.0 && opts.tail_seconds >= 0.0 {
            warnings.push("tail_may_cut_users");
        }

        Ok(RecordSetup {
            request: request.clone(),
            recipe_settings,
            sim_settings,
            width,
            height,
            ui_scale,
            match_window: opts.viewport.is_none(),
            plan,
            bitrate_bps: bitrate,
            level,
            ladder,
            estimate,
            stop,
            warnings,
        })
    }

    pub fn options(&self) -> &RecordOptions {
        &self.request.options
    }

    /// A fresh simulation of the run, sized for the output. The recorder and
    /// the dry run both start from this, so they run the same ticks.
    pub fn new_simulation(&self) -> Simulation {
        let mut sim = Simulation::new(self.sim_settings.clone());
        sim.set_viewport(self.width, self.height, self.ui_scale);
        sim
    }

    /// The H.264 High codec string of the request.
    pub fn codec(&self) -> String {
        codec_string(Profile::High, self.level.idc)
    }

    /// Frame count bound for a `faststart` reserve: the dry run's count, a
    /// `record-max-seconds` limit, or (`known_only` false) the upper bound.
    pub fn max_samples(&self, exact_frames: Option<u64>, known_only: bool) -> Option<u64> {
        let limit = (self.stop.max_frames > 0).then_some(self.stop.max_frames);
        let known = exact_frames.or(limit);
        if known.is_some() || known_only {
            return known;
        }
        Some(self.plan.frames_in(self.estimate.upper_s) + 1)
    }

    /// `RecordPlan` JSON (`recording.md` section 5.15).
    pub fn plan_value(&self, exact_frames: Option<u64>) -> Value {
        let plan = &self.plan;
        let fps = f64::from(plan.fps);
        let e = &self.estimate;
        let duration_s = exact_frames.map(|n| n as f64 / fps);
        let bound_s = duration_s.unwrap_or(e.upper_s);
        let size_upper = SIZE_BOUND_FACTOR * self.bitrate_bps as f64 * bound_s / 8.0;
        let opt_number = |v: Option<u64>| v.map_or(Value::Null, |n| number(n as f64));

        let opts = self.options();
        let seekable_max = self.max_samples(exact_frames, opts.layout == LayoutChoice::Auto);
        let (seekable, memory) = match opts.layout {
            LayoutChoice::Fragmented => ("fragmented", "fragmented"),
            LayoutChoice::Faststart => ("faststart", "faststart-deferred"),
            LayoutChoice::Auto if seekable_max.is_some() => ("faststart", "faststart-deferred"),
            LayoutChoice::Auto => ("fragmented", "faststart-deferred"),
        };
        let sync = |n: u64| n.div_ceil(u64::from(plan.keyframe_every)) + 1;
        let max_samples = if seekable == "faststart" {
            seekable_max
        } else {
            None
        };

        object([
            ("fps", number(fps)),
            ("ticks_first", number(1.0)),
            ("ticks_per_frame", number(f64::from(plan.ticks_per_frame))),
            ("tick_hz", number(f64::from(plan.tick_hz))),
            ("fixed_dt", Value::Number(plan.fixed_dt.to_string())),
            ("width", number(f64::from(self.width))),
            ("height", number(f64::from(self.height))),
            ("ui_scale", Value::Number(self.ui_scale.to_string())),
            ("match_window", Value::Bool(self.match_window)),
            ("level_idc", number(f64::from(self.level.idc))),
            ("codec", string(&self.codec())),
            ("bitrate_bps", number(self.bitrate_bps as f64)),
            ("keyframe_every", number(f64::from(plan.keyframe_every))),
            (
                "max_frames",
                opt_number((self.stop.max_frames > 0).then_some(self.stop.max_frames)),
            ),
            ("tail_s", number(e.tail_s)),
            (
                "estimate",
                object([
                    ("about_s", number(round3(e.about_s))),
                    ("upper_s", number((e.upper_s * 1000.0).ceil() / 1000.0)),
                    ("exact", Value::Bool(exact_frames.is_some())),
                    ("frames", opt_number(exact_frames)),
                    (
                        "duration_s",
                        duration_s.map_or(Value::Null, |d| number(round3(d))),
                    ),
                    ("about_frames", number(plan.frames_in(e.about_s) as f64)),
                    ("upper_frames", number(plan.frames_in(e.upper_s) as f64)),
                    ("size_upper_bytes", number(size_upper.ceil())),
                    ("empty", Value::Bool(e.empty())),
                    ("commits", number(e.range.len() as f64)),
                    ("span_days", number(round3(e.span_days))),
                    ("clipped", Value::Bool(e.clipped)),
                ]),
            ),
            (
                "layout",
                object([
                    ("seekable", string(seekable)),
                    ("memory", string(memory)),
                    ("max_samples", opt_number(max_samples)),
                    ("max_sync_samples", opt_number(max_samples.map(sync))),
                ]),
            ),
            (
                "ladder",
                Value::Array(self.ladder.iter().map(EncoderCandidate::to_value).collect()),
            ),
            (
                "warnings",
                Value::Array(self.warnings.iter().map(|w| string(w)).collect()),
            ),
            (
                "limits",
                object([
                    ("min_side", number(f64::from(MIN_SIDE))),
                    ("max_side", number(f64::from(MAX_SIDE))),
                    ("max_macroblocks", number(f64::from(MAX_MACROBLOCKS))),
                ]),
            ),
        ])
    }
}

/// Milliseconds are enough for durations shown to people.
fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}
