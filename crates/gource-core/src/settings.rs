// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Gource's tunables (`GourceSettings`), with the defaults of
//! `GourceSettings::setGourceDefaults()` and the option names of Gource's
//! command line / conf file (`gource_settings.cpp`).
//!
//! Hosts change settings through [`Settings::apply_option`] using Gource
//! option names and value syntax (`"bloom-multiplier" = "1.5"`,
//! `"hide" = "filenames,dirnames"`, `"background-colour" = "000000"`,
//! boolean flags as `"true"`/`"false"`). [`OptionEffect`] tells the caller
//! whether the change is live or requires the simulation to be rebuilt from
//! the event log (file and user filters, start/stop date and position).

use glam::{Vec2, Vec3};

/// `--camera-mode`.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum CameraMode {
    #[default]
    Overview,
    Track,
}

/// `--rand-mode` (port-only, hidden): where the former `rand()` call sites
/// (overlap tie-breaks of dirs, users and actions; colour reseeding) draw
/// from.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum RandMode {
    /// Seeded keyed hashing, one stream per call site (deterministic on
    /// every platform; the default).
    #[default]
    Keyed,
    /// One shared emulation of glibc's unseeded `rand()` in the C++ call
    /// order, for parity runs against the reference renders of the C++
    /// build. Takes effect when the simulation is rebuilt.
    Glibc,
}

/// `ai-style`: which built-in decoration rules form the first style layer
/// (see [`crate::style::builtin_rules`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum AiStyle {
    /// [`crate::style::BUILTIN_RULES_V1`]: cape and shades when assisted,
    /// halo and glow for an agent author, a badge for both.
    #[default]
    Default,
    Cape,
    Shades,
    /// A glowing hat instead of cape and shades.
    Hat,
    /// No built-in rules (the viewer's own rules still apply).
    None,
}

/// `ai-filter`: which commits are played, by their AI attribution.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum AiFilter {
    #[default]
    All,
    /// Only commits credited to an agent (assisted or agent-authored).
    OnlyAssisted,
    /// Only commits not credited to an agent.
    ExcludeAssisted,
}

/// Hosted source mode. Changing this setting is live in the engine,
/// while the page explicitly reloads an acquired source when appropriate.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum PullRequestMode {
    #[default]
    Basic,
    Full,
}

/// Pluggable PR visual style, independent of the lifecycle model.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum PullRequestStyle {
    #[default]
    DimHighlight,
}

/// What a caller must do after an option changed.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum OptionEffect {
    /// Takes effect from the next frame.
    Live,
    /// The simulation must be rebuilt from the event log
    /// (`Simulation::restart`), e.g. filters or the date range changed.
    Restart,
}

/// Rejected option name or value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OptionError {
    UnknownOption(String),
    InvalidValue {
        option: String,
        value: String,
        reason: String,
    },
}

impl std::fmt::Display for OptionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OptionError::UnknownOption(name) => write!(f, "unknown option '{name}'"),
            OptionError::InvalidValue {
                option,
                value,
                reason,
            } => {
                write!(f, "invalid value '{value}' for '{option}': {reason}")
            }
        }
    }
}

impl std::error::Error for OptionError {}

/// All tunables that exist in the port. Field names follow `GourceSettings`.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    // --- visibility (`--hide`, `--key`) ---
    pub hide_date: bool,
    pub hide_users: bool,
    pub hide_tree: bool,
    pub hide_files: bool,
    pub hide_usernames: bool,
    pub hide_filenames: bool,
    pub hide_dirnames: bool,
    pub hide_progress: bool,
    pub hide_bloom: bool,
    pub hide_mouse: bool,
    pub hide_root: bool,
    pub show_key: bool,
    /// `hide=decorations`: no AI decoration is drawn, from any style layer.
    pub hide_decorations: bool,
    /// Removes typed PR geometry and actions before layout, not by path regex.
    pub hide_pull_requests: bool,
    pub pull_request_mode: PullRequestMode,
    pub pull_request_style: PullRequestStyle,
    /// Repository-wide flow summary, only with complete lifecycle coverage.
    pub pr_flow_hud: bool,
    pub disable_auto_rotate: bool,
    pub disable_input: bool,

    // --- time model ---
    /// Raw `--start-date` / `--stop-date` strings (Gource date syntax).
    pub start_date: String,
    pub stop_date: String,
    /// Parsed dates; 0 = unset.
    pub start_timestamp: i64,
    pub stop_timestamp: i64,
    pub start_position: f32,
    pub stop_position: f32,
    /// `--stop-at-time` seconds of runtime; negative = unset.
    pub stop_at_time: f32,
    pub stop_on_idle: bool,
    pub stop_at_end: bool,
    pub dont_stop: bool,
    pub no_time_travel: bool,
    pub loop_playback: bool,
    pub loop_delay_seconds: f32,
    /// Negative disables auto skip (`--disable-auto-skip`).
    pub auto_skip_seconds: f32,
    /// `--seconds-per-day` is stored inverted, as Gource does.
    pub days_per_second: f32,
    pub time_scale: f32,
    /// Seconds before an untouched file fades; 0 = never. Gource replaces
    /// it with `file_idle_time_at_end` (when that is set) once the log ends.
    pub file_idle_time: f32,
    pub file_idle_time_at_end: f32,
    pub author_time: bool,

    // --- camera ---
    pub camera_mode: CameraMode,
    pub camera_zoom_min: f32,
    pub camera_zoom_default: f32,
    pub camera_zoom_max: f32,
    pub padding: f32,
    pub crop_vertical: bool,
    pub crop_horizontal: bool,
    pub follow_users: Vec<String>,

    // --- look ---
    pub bloom_multiplier: f32,
    pub bloom_intensity: f32,
    pub background_colour: Vec3,
    pub title: String,
    pub font_scale: f32,
    /// `default_font_scale`: no `font-scale` was given, so the simulation
    /// picks `font_scale` from the viewport (Gource's constructor rule; see
    /// [`crate::sim::Simulation::set_viewport`]). Applying `font-scale`
    /// clears it; applying an empty value sets it again.
    pub default_font_scale: bool,
    pub font_size: i32,
    pub filename_font_size: i32,
    pub dirname_font_size: i32,
    pub user_font_size: i32,
    pub font_colour: Vec3,
    pub dir_colour: Vec3,
    pub highlight_colour: Vec3,
    pub selection_colour: Vec3,
    pub filename_colour: Vec3,
    pub dir_name_depth: i32,
    pub dir_name_position: f32,
    pub elasticity: f32,
    /// strftime-style `--date-format`.
    pub date_format: String,
    pub logo_offset: Vec2,
    pub fixed_user_size: bool,
    pub user_scale: f32,
    pub highlight_dirs: bool,
    pub highlight_all_users: bool,
    pub highlight_users: Vec<String>,
    pub file_extensions: bool,
    pub file_extension_fallback: bool,
    pub filename_time: f32,
    pub caption_duration: f32,
    pub caption_size: i32,
    pub caption_offset: i32,
    pub caption_colour: Vec3,

    // --- simulation limits ---
    pub max_files: i32,
    pub max_user_speed: f32,
    pub max_file_lag: f32,
    pub user_idle_time: f32,
    /// Stored inverted, as Gource does: `--user-friction 0.5` gives 2.0.
    pub user_friction: f32,

    // --- event filters (regex sources; compiled by the simulation) ---
    pub file_filters: Vec<String>,
    pub file_show_filters: Vec<String>,
    pub user_filters: Vec<String>,
    pub user_show_filters: Vec<String>,

    /// `--hash-seed` (`gStringHashSeed`, default 31). Gource's seed is an
    /// `int`; it is stored bit for bit (`seed as u32`) and used as `i32`.
    pub hash_seed: u32,

    // --- AI attribution and contributor styles (port-only) ---
    /// `user-style`: the viewer's rule lines, trimmed, without comments
    /// and blank lines (validated by [`crate::style::parse_rules`]).
    pub user_style: Vec<String>,
    pub ai_style: AiStyle,
    pub ai_filter: AiFilter,
    /// `agent-key`: list the agents seen so far, top right.
    pub agent_key: bool,
    /// `decor-motion`: `full` (true) or `off` (false: capes neither lean
    /// nor flutter).
    pub decor_motion: bool,

    // --- port-only ---
    /// Offset of local time from UTC in minutes (local = UTC + offset, so
    /// UTC+9 is `540`), used for the date label and for dates without a time
    /// zone in `start-date` / `stop-date`. A browser host sets it to
    /// `-Date.getTimezoneOffset()`. Gource uses the process time zone
    /// (`localtime`); the core stays platform-free by taking the offsets
    /// from the host: this one, then [`Settings::utc_offset_changes`].
    pub utc_offset_minutes: i32,
    /// Changes of the local offset (DST, zone rule changes) as `(unix
    /// seconds, offset minutes)` pairs in ascending time: from that instant
    /// on, local = UTC + offset. Before the first change (or when empty)
    /// `utc_offset_minutes` applies. A browser host can fill it by probing
    /// `new Date(ts * 1000).getTimezoneOffset()` across the log's range, so
    /// historical dates show the offset `localtime` would use.
    pub utc_offset_changes: Vec<(i64, i32)>,
    /// `rand-mode`: see [`RandMode`].
    pub rand_mode: RandMode,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            hide_date: false,
            hide_users: false,
            hide_tree: false,
            hide_files: false,
            hide_usernames: false,
            hide_filenames: false,
            hide_dirnames: false,
            hide_progress: false,
            hide_bloom: false,
            hide_mouse: false,
            hide_root: false,
            show_key: false,
            hide_decorations: false,
            disable_auto_rotate: false,
            disable_input: false,

            start_date: String::new(),
            stop_date: String::new(),
            start_timestamp: 0,
            stop_timestamp: 0,
            start_position: 0.0,
            stop_position: 0.0,
            stop_at_time: -1.0,
            stop_on_idle: false,
            stop_at_end: false,
            dont_stop: false,
            no_time_travel: false,
            loop_playback: false,
            loop_delay_seconds: 3.0,
            auto_skip_seconds: 3.0,
            days_per_second: 0.1,
            time_scale: 1.0,
            file_idle_time: 0.0,
            file_idle_time_at_end: 0.0,
            author_time: false,

            camera_mode: CameraMode::Overview,
            camera_zoom_min: 50.0,
            camera_zoom_default: 100.0,
            camera_zoom_max: 10000.0,
            padding: 1.1,
            crop_vertical: false,
            crop_horizontal: false,
            follow_users: Vec::new(),

            bloom_multiplier: 1.0,
            bloom_intensity: 0.75,
            background_colour: Vec3::splat(0.1),
            title: String::new(),
            font_scale: 1.0,
            default_font_scale: true,
            font_size: 16,
            filename_font_size: 14,
            dirname_font_size: 14,
            user_font_size: 14,
            font_colour: Vec3::ONE,
            dir_colour: Vec3::ONE,
            highlight_colour: Vec3::ONE,
            selection_colour: Vec3::new(1.0, 1.0, 0.3),
            filename_colour: Vec3::ONE,
            dir_name_depth: 0,
            dir_name_position: 0.5,
            elasticity: 0.0,
            date_format: "%A, %d %B, %Y %X".to_string(),
            logo_offset: Vec2::new(20.0, 20.0),
            fixed_user_size: false,
            user_scale: 1.0,
            highlight_dirs: false,
            highlight_all_users: false,
            highlight_users: Vec::new(),
            file_extensions: false,
            file_extension_fallback: false,
            filename_time: 4.0,
            caption_duration: 10.0,
            caption_size: 16,
            caption_offset: 0,
            caption_colour: Vec3::ONE,

            max_files: 0,
            max_user_speed: 500.0,
            max_file_lag: 5.0,
            user_idle_time: 3.0,
            user_friction: 1.0,

            file_filters: Vec::new(),
            file_show_filters: Vec::new(),
            user_filters: Vec::new(),
            user_show_filters: Vec::new(),

            hash_seed: 31,

            user_style: Vec::new(),
            ai_style: AiStyle::Default,
            hide_pull_requests: false,
            pull_request_mode: PullRequestMode::Basic,
            pull_request_style: PullRequestStyle::DimHighlight,
            pr_flow_hud: false,
            ai_filter: AiFilter::All,
            agent_key: false,
            decor_motion: true,

            utc_offset_minutes: 0,
            utc_offset_changes: Vec::new(),
            rand_mode: RandMode::Keyed,
        }
    }
}

impl Settings {
    /// Local offset from UTC in minutes at unix time `ts` (see
    /// [`Settings::utc_offset_changes`]).
    pub fn utc_offset_at(&self, ts: i64) -> i32 {
        let changes = &self.utc_offset_changes;
        match changes.partition_point(|&(t, _)| t <= ts) {
            0 => self.utc_offset_minutes,
            i => changes[i - 1].1,
        }
    }

    /// `setScaledFontSizes()`: font sizes multiplied by `font_scale`, clamped to 1..=100.
    pub fn scaled_font_size(&self) -> i32 {
        scale_font(self.font_size, self.font_scale)
    }

    pub fn scaled_user_font_size(&self) -> i32 {
        scale_font(self.user_font_size, self.font_scale)
    }

    pub fn scaled_dirname_font_size(&self) -> i32 {
        scale_font(self.dirname_font_size, self.font_scale)
    }

    pub fn scaled_filename_font_size(&self) -> i32 {
        scale_font(self.filename_font_size, self.font_scale)
    }

    /// Apply one option by its Gource name (without the leading `--`;
    /// Gource's aliases such as `s` or `background` are accepted too).
    ///
    /// Values use Gource's syntax and validation; the full rules are in the
    /// documentation of the crate-private `options` module: booleans
    /// `"true"`/`"false"`, colours `RRGGBB`, multi-value options one value
    /// per line, an empty value resets an option to its default. Only the
    /// settings change; [`crate::sim::Simulation::set_option`] also applies
    /// the side effects on a running simulation.
    pub fn apply_option(&mut self, name: &str, value: &str) -> Result<OptionEffect, OptionError> {
        crate::options::apply(self, name, value)
    }

    /// Current value of an option in the syntax [`Settings::apply_option`]
    /// accepts (`None` for an unknown option). Applying the returned value
    /// leaves the settings unchanged, except that colours are reported as
    /// `RRGGBB` and so round to the nearest 1/255, and that applying the
    /// reported `font-scale` makes it explicit (clears
    /// [`Settings::default_font_scale`]).
    pub fn option_value(&self, name: &str) -> Option<String> {
        crate::options::value(self, name)
    }

    /// Every option name [`Settings::apply_option`] accepts (canonical
    /// names, without aliases).
    pub fn option_names() -> &'static [&'static str] {
        crate::options::OPTION_NAMES
    }
}

/// Options whose values routinely hold contributor or file names (AI-37):
/// they are redacted from embedded recording recipes and flagged on
/// private-repository share links.
pub use crate::options::NAME_BEARING_OPTIONS;

fn scale_font(size: i32, scale: f32) -> i32 {
    ((size as f32 * scale) as i32).clamp(1, 100)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_set_gource_defaults() {
        let s = Settings::default();
        assert_eq!(s.days_per_second, 0.1);
        assert_eq!(s.auto_skip_seconds, 3.0);
        assert_eq!(s.hash_seed, 31);
        assert_eq!(s.scaled_filename_font_size(), 14);
        assert_eq!(s.selection_colour, Vec3::new(1.0, 1.0, 0.3));
    }

    #[test]
    fn utc_offset_follows_the_change_table() {
        let mut s = Settings {
            utc_offset_minutes: 60,
            ..Settings::default()
        };
        assert_eq!(s.utc_offset_at(i64::MIN), 60);
        s.utc_offset_changes = vec![(1000, 120), (2000, 60)];
        assert_eq!(s.utc_offset_at(999), 60);
        assert_eq!(s.utc_offset_at(1000), 120);
        assert_eq!(s.utc_offset_at(1999), 120);
        assert_eq!(s.utc_offset_at(2000), 60);
        assert_eq!(s.utc_offset_at(i64::MAX), 60);
    }
}
