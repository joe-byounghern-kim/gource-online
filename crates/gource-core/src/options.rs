// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! The option table behind [`Settings::apply_option`] and
//! [`Settings::option_value`]: Gource's option names, aliases, value syntax
//! and validation from `GourceSettings::importGourceSettings`
//! (`gource_settings.cpp`).
//!
//! Value syntax:
//! - booleans: Gource's `ConfEntry::getBool` spellings (`true`, `1`, `yes`,
//!   ... and `false`, `0`, `no`, ...); an empty value means `true`, like a
//!   bare command-line flag. Unlike a conf file, `false` clears the flag so
//!   a live control can switch it back off;
//! - colours: `RRGGBB` (six hex digits, as `sscanf("%02x%02x%02x")` reads
//!   them) or the conf-file form `vec3(r, g, b)`; they are reported as
//!   `RRGGBB`, rounded to the nearest 1/255 (so the defaults 0.1 and 0.3
//!   come back as `1A` and `4D`);
//! - multi-value options (`file-filter`, `user-filter`, `follow-user`, ...):
//!   one value per line; empty lines are ignored and the list replaces the
//!   previous one;
//! - `hide`: Gource's comma list; it sets exactly the listed elements
//!   (Gource starts from defaults on every import, so the effect is the
//!   same), and `mouse` also hides the progress bar;
//! - dates: `SDLAppSettings::parseDateTime` syntax, local time is UTC +
//!   [`Settings::utc_offset_at`] (set the offsets before the dates).
//!
//! Differences from the C++ parser, all for live controls:
//! - numbers must parse completely (`atof("1.5x")` would accept `1.5`);
//! - an empty value resets a non-boolean option to its default, and every
//!   value [`Settings::option_value`] reports is accepted back, including
//!   defaults outside Gource's range (`elasticity` 0, `dir-name-depth` 0,
//!   `stop-at-time` -1, start/stop position 0, unset dates) and a negative
//!   `auto-skip-seconds` (disabled, as `--disable-auto-skip` stores it);
//! - `start-position random` draws from a keyed hash of the hash seed
//!   instead of `srand(time(0))`.
//!
//! Port-only: `rand-mode` (`keyed`, the default, or `glibc`), a hidden
//! option for parity runs against the C++ reference renders (see
//! [`RandMode`]), and the AI attribution options: `user-style` (rule
//! lines, see [`crate::style`]; reported trimmed, without comments and
//! blank lines), `ai-style` (`default`, `cape`, `shades`, `hat`, `none`),
//! `ai-filter` (`all`, `only-assisted`, `exclude-assisted`), `agent-key`,
//! `decor-motion` (`full`, `off`) and the `hide` element `decorations`
//! (alias `hide-decorations`).
//!
//! [`OptionEffect::Restart`] is returned for the filters (`ai-filter`
//! included), the start/stop date and position and `rand-mode`; everything
//! else is [`OptionEffect::Live`].

use glam::{Vec2, Vec3};

use crate::datefmt::parse_date_time;
use crate::hash::{KeyedRng, stream_key};
use crate::settings::{
    AiFilter, AiStyle, CameraMode, OptionEffect, OptionError, PullRequestMode, PullRequestStyle,
    RandMode, Settings,
};
use crate::style::{self, MAX_ECHO_BYTES};

/// Elements of `--hide`, in the order `option_value("hide")` lists them.
const HIDE_FIELDS: [&str; 13] = [
    "date",
    "users",
    "tree",
    "files",
    "usernames",
    "filenames",
    "dirnames",
    "progress",
    "bloom",
    "mouse",
    "root",
    "pull-requests",
    "decorations",
];

/// Every option name [`Settings::apply_option`] accepts (canonical names;
/// aliases are listed in [`canonical_name`]).
pub(crate) const OPTION_NAMES: &[&str] = &[
    "hide",
    "hide-date",
    "hide-users",
    "hide-tree",
    "hide-files",
    "hide-usernames",
    "hide-filenames",
    "hide-dirnames",
    "hide-progress",
    "hide-bloom",
    "hide-mouse",
    "hide-root",
    "hide-decorations",
    "hide-pull-requests",
    "pull-request-mode",
    "pull-request-style",
    "pr-flow-hud",
    "key",
    "disable-auto-rotate",
    "disable-auto-skip",
    "disable-input",
    "loop",
    "loop-delay-seconds",
    "realtime",
    "no-time-travel",
    "dont-stop",
    "stop-at-end",
    "stop-on-idle",
    "fixed-user-size",
    "author-time",
    "highlight-users",
    "highlight-dirs",
    "file-extensions",
    "file-extension-fallback",
    "date-format",
    "crop",
    "caption-duration",
    "caption-size",
    "caption-offset",
    "caption-colour",
    "filename-colour",
    "filename-time",
    "bloom-intensity",
    "bloom-multiplier",
    "elasticity",
    "font-size",
    "file-font-size",
    "dir-font-size",
    "user-font-size",
    "font-scale",
    "hash-seed",
    "font-colour",
    "background-colour",
    "highlight-colour",
    "selection-colour",
    "dir-colour",
    "title",
    "logo-offset",
    "seconds-per-day",
    "auto-skip-seconds",
    "file-idle-time",
    "file-idle-time-at-end",
    "user-idle-time",
    "time-scale",
    "start-date",
    "stop-date",
    "start-position",
    "stop-position",
    "stop-at-time",
    "max-files",
    "max-file-lag",
    "user-friction",
    "user-scale",
    "max-user-speed",
    "camera-mode",
    "padding",
    "highlight-user",
    "follow-user",
    "file-filter",
    "file-show-filter",
    "user-filter",
    "user-show-filter",
    "dir-name-depth",
    "dir-name-position",
    "rand-mode",
    "user-style",
    "ai-style",
    "ai-filter",
    "agent-key",
    "decor-motion",
];

/// Options whose values routinely hold contributor or file names (AI-37).
/// The list is final; it is cited by private-repository share links and by
/// recording recipes, which redact these options.
pub const NAME_BEARING_OPTIONS: &[&str] = &[
    "user-filter",
    "user-show-filter",
    "file-filter",
    "file-show-filter",
    "highlight-user",
    "follow-user",
    "title",
    "user-style",
];

/// Gource's `arg_aliases`.
pub(crate) fn canonical_name(name: &str) -> &str {
    let name = name.strip_prefix("--").unwrap_or(name);
    match name {
        "p" => "start-position",
        "a" => "auto-skip-seconds",
        "s" => "seconds-per-day",
        "t" => "stop-at-time",
        "i" => "file-idle-time",
        "e" => "elasticity",
        "b" | "background" => "background-colour",
        "c" => "time-scale",
        "disable-bloom" => "hide-bloom",
        "disable-progress" => "hide-progress",
        "highlight-all-users" => "highlight-users",
        other => other,
    }
}

/// Options that need `Simulation::restart` to take effect.
pub(crate) fn effect_of(name: &str) -> OptionEffect {
    match name {
        "file-filter" | "file-show-filter" | "user-filter" | "user-show-filter" | "start-date"
        | "stop-date" | "start-position" | "stop-position" | "rand-mode" | "ai-filter"
        | "max-files" => OptionEffect::Restart,
        _ => OptionEffect::Live,
    }
}

struct Ctx<'a> {
    option: &'a str,
    value: &'a str,
}

impl Ctx<'_> {
    fn invalid(&self, reason: &str) -> OptionError {
        OptionError::InvalidValue {
            option: self.option.to_string(),
            value: self.value.to_string(),
            reason: reason.to_string(),
        }
    }

    fn bool(&self) -> Result<bool, OptionError> {
        match self.value {
            "" | "1" | "true" | "True" | "TRUE" | "yes" | "Yes" | "YES" => Ok(true),
            "0" | "false" | "False" | "FALSE" | "no" | "No" | "NO" => Ok(false),
            _ => Err(self.invalid("expected true or false")),
        }
    }

    fn float(&self) -> Result<f32, OptionError> {
        let v: f32 = self
            .value
            .trim()
            .parse()
            .map_err(|_| self.invalid("expected a number"))?;
        if !v.is_finite() {
            return Err(self.invalid("expected a finite number"));
        }
        Ok(v)
    }

    fn int(&self) -> Result<i32, OptionError> {
        self.value
            .trim()
            .parse()
            .map_err(|_| self.invalid("expected an integer"))
    }

    fn colour(&self) -> Result<Vec3, OptionError> {
        parse_colour(self.value).ok_or_else(|| self.invalid("expected a colour as RRGGBB"))
    }

    fn check(&self, ok: bool, reason: &str) -> Result<(), OptionError> {
        if ok {
            Ok(())
        } else {
            Err(self.invalid(reason))
        }
    }

    fn lines(&self) -> Vec<String> {
        self.value
            .lines()
            .map(|l| l.trim_end_matches('\r'))
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect()
    }

    fn regex_lines(&self) -> Result<Vec<String>, OptionError> {
        let lines = self.lines();
        for l in &lines {
            if regex_lite::Regex::new(l).is_err() {
                return Err(OptionError::InvalidValue {
                    option: self.option.to_string(),
                    value: l.clone(),
                    reason: format!("invalid {} regular expression", self.option),
                });
            }
        }
        Ok(lines)
    }
}

/// `RRGGBB` to a colour in 0..=1, as `vec3(r, g, b) / 255.0f`; Gource's
/// conf-file form `vec3(r, g, b)` is accepted too.
pub(crate) fn parse_colour(s: &str) -> Option<Vec3> {
    if let Some(inner) = s
        .trim()
        .strip_prefix("vec3(")
        .and_then(|r| r.strip_suffix(')'))
    {
        let mut it = inner.split(',').map(|c| c.trim().parse::<f32>());
        let v = Vec3::new(it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?);
        return if it.next().is_none() { Some(v) } else { None };
    }
    if s.len() != 6 || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let c = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok();
    let mut v = Vec3::new(c(0)? as f32, c(2)? as f32, c(4)? as f32);
    v /= 255.0;
    Some(v)
}

fn format_colour(c: Vec3) -> String {
    let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("{:02X}{:02X}{:02X}", b(c.x), b(c.y), b(c.z))
}

/// The `atoi`-based parse of `file-idle-time`: an integer number of seconds.
fn parse_idle_time(ctx: &Ctx) -> Result<f32, OptionError> {
    let s = ctx.value.trim();
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    let end = digits
        .bytes()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(digits.len());
    if end == 0 {
        return Err(ctx.invalid("expected a number of seconds"));
    }
    let v: i64 = s[..s.len() - digits.len() + end]
        .parse()
        .map_err(|_| ctx.invalid("expected a number of seconds"))?;
    let v = v as f32;
    ctx.check(v >= 0.0, "must not be negative")?;
    Ok(v)
}

fn hide_flag<'a>(s: &'a mut Settings, field: &str) -> Option<&'a mut bool> {
    Some(match field {
        "date" => &mut s.hide_date,
        "users" => &mut s.hide_users,
        "tree" => &mut s.hide_tree,
        "files" => &mut s.hide_files,
        "usernames" => &mut s.hide_usernames,
        "filenames" => &mut s.hide_filenames,
        "dirnames" => &mut s.hide_dirnames,
        "progress" => &mut s.hide_progress,
        "bloom" => &mut s.hide_bloom,
        "mouse" => &mut s.hide_mouse,
        "root" => &mut s.hide_root,
        "decorations" => &mut s.hide_decorations,
        "pull-requests" => &mut s.hide_pull_requests,
        _ => return None,
    })
}

fn hide_value(s: &Settings, field: &str) -> bool {
    match field {
        "date" => s.hide_date,
        "users" => s.hide_users,
        "tree" => s.hide_tree,
        "files" => s.hide_files,
        "usernames" => s.hide_usernames,
        "filenames" => s.hide_filenames,
        "dirnames" => s.hide_dirnames,
        "progress" => s.hide_progress,
        "bloom" => s.hide_bloom,
        "mouse" => s.hide_mouse,
        "root" => s.hide_root,
        "decorations" => s.hide_decorations,
        "pull-requests" => s.hide_pull_requests,
        _ => false,
    }
}

/// Split Gource's `--hide` list exactly as `importGourceSettings` does.
fn split_hide_list(value: &str) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut rest = value;
    while let Some(sep) = rest.find(',') {
        if sep == 0 && rest.len() == 1 {
            rest = "";
            break;
        }
        if sep == 0 {
            rest = &rest[1..];
            continue;
        }
        fields.push(&rest[..sep]);
        rest = &rest[sep + 1..];
    }
    if !rest.is_empty() && rest != "," {
        fields.push(rest);
    }
    fields
}

/// The plain boolean flags: option name to `Settings` field.
macro_rules! bool_options {
    ($($name:literal => $field:ident,)*) => {
        fn simple_bool<'a>(s: &'a mut Settings, name: &str) -> Option<&'a mut bool> {
            match name {
                $($name => Some(&mut s.$field),)*
                _ => None,
            }
        }

        fn simple_bool_value(s: &Settings, name: &str) -> Option<bool> {
            match name {
                $($name => Some(s.$field),)*
                _ => None,
            }
        }
    };
}

bool_options! {
    "hide-date" => hide_date,
    "hide-users" => hide_users,
    "hide-tree" => hide_tree,
    "hide-files" => hide_files,
    "hide-usernames" => hide_usernames,
    "hide-filenames" => hide_filenames,
    "hide-dirnames" => hide_dirnames,
    "hide-progress" => hide_progress,
    "hide-bloom" => hide_bloom,
    "hide-root" => hide_root,
    "hide-decorations" => hide_decorations,
    "key" => show_key,
    "agent-key" => agent_key,
    "pr-flow-hud" => pr_flow_hud,
    "disable-auto-rotate" => disable_auto_rotate,
    "disable-input" => disable_input,
    "loop" => loop_playback,
    "no-time-travel" => no_time_travel,
    "dont-stop" => dont_stop,
    "stop-at-end" => stop_at_end,
    "stop-on-idle" => stop_on_idle,
    "fixed-user-size" => fixed_user_size,
    "author-time" => author_time,
    "highlight-users" => highlight_all_users,
    "highlight-dirs" => highlight_dirs,
    "file-extensions" => file_extensions,
    "file-extension-fallback" => file_extension_fallback,
}

fn realtime_days_per_second() -> f32 {
    (1.0f64 / 86400.0) as f32
}

fn fmt_f32(v: f32) -> String {
    format!("{v}")
}

/// `1.0 / x` computed as Gource does for `seconds-per-day` and
/// `user-friction` (double division, stored as float).
fn inverse(x: f32) -> f32 {
    (1.0f64 / x as f64) as f32
}

pub(crate) fn apply(
    s: &mut Settings,
    name: &str,
    value: &str,
) -> Result<OptionEffect, OptionError> {
    let name = canonical_name(name);
    let ctx = Ctx {
        option: name,
        value,
    };
    let d = Settings::default();
    let empty = value.trim().is_empty();

    if let Some(flag) = simple_bool(s, name) {
        *flag = ctx.bool()?;
        return Ok(effect_of(name));
    }

    match name {
        "hide" => {
            let previously_hidden = s.hide_pull_requests;
            let fields = split_hide_list(value);
            for f in &fields {
                if !HIDE_FIELDS.contains(f) {
                    return Err(ctx.invalid(&format!("unknown option hide {f}")));
                }
            }
            for f in HIDE_FIELDS {
                *hide_flag(s, f).expect("known field") = false;
            }
            for f in fields {
                *hide_flag(s, f).expect("known field") = true;
                if f == "mouse" {
                    s.hide_progress = true;
                }
            }
            return Ok(if previously_hidden != s.hide_pull_requests {
                OptionEffect::Restart
            } else {
                OptionEffect::Live
            });
        }
        "hide-pull-requests" => {
            let hidden = ctx.bool()?;
            let changed = hidden != s.hide_pull_requests;
            s.hide_pull_requests = hidden;
            return Ok(if changed {
                OptionEffect::Restart
            } else {
                OptionEffect::Live
            });
        }
        "pull-request-mode" => {
            s.pull_request_mode = match value.trim() {
                "" | "basic" => PullRequestMode::Basic,
                "full" => PullRequestMode::Full,
                _ => return Err(ctx.invalid("expected basic or full")),
            };
        }
        "pull-request-style" => {
            s.pull_request_style = match value.trim() {
                "" | "dim-highlight" => PullRequestStyle::DimHighlight,
                _ => return Err(ctx.invalid("expected dim-highlight")),
            };
        }
        "hide-mouse" => {
            let on = ctx.bool()?;
            s.hide_mouse = on;
            if on {
                s.hide_progress = true;
            }
        }
        "disable-auto-skip" => {
            if ctx.bool()? {
                s.auto_skip_seconds = -1.0;
            } else if s.auto_skip_seconds < 0.0 {
                s.auto_skip_seconds = d.auto_skip_seconds;
            }
        }
        "realtime" => {
            if ctx.bool()? {
                s.days_per_second = realtime_days_per_second();
            } else if s.days_per_second == realtime_days_per_second() {
                s.days_per_second = d.days_per_second;
            }
        }
        "date-format" => {
            s.date_format = if empty {
                d.date_format
            } else {
                value.to_string()
            };
        }
        "title" => s.title = value.to_string(),
        "crop" => {
            let (v, h) = match value.trim() {
                "" => (false, false),
                "vertical" => (true, false),
                "horizontal" => (false, true),
                _ => return Err(ctx.invalid("specify crop (vertical,horizontal)")),
            };
            s.crop_vertical = v;
            s.crop_horizontal = h;
        }
        "rand-mode" => {
            s.rand_mode = match value.trim() {
                "" | "keyed" => RandMode::Keyed,
                "glibc" => RandMode::Glibc,
                _ => return Err(ctx.invalid("specify rand-mode (keyed,glibc)")),
            };
        }
        "camera-mode" => {
            s.camera_mode = match value.trim() {
                "" | "overview" => CameraMode::Overview,
                "track" => CameraMode::Track,
                _ => return Err(ctx.invalid("specify camera-mode (overview,track)")),
            };
        }
        "font-colour" | "background-colour" | "highlight-colour" | "selection-colour"
        | "dir-colour" | "filename-colour" | "caption-colour" => {
            let (field, default) = match name {
                "font-colour" => (&mut s.font_colour, d.font_colour),
                "background-colour" => (&mut s.background_colour, d.background_colour),
                "highlight-colour" => (&mut s.highlight_colour, d.highlight_colour),
                "selection-colour" => (&mut s.selection_colour, d.selection_colour),
                "dir-colour" => (&mut s.dir_colour, d.dir_colour),
                "filename-colour" => (&mut s.filename_colour, d.filename_colour),
                _ => (&mut s.caption_colour, d.caption_colour),
            };
            *field = if empty { default } else { ctx.colour()? };
        }
        "logo-offset" => {
            if empty {
                s.logo_offset = d.logo_offset;
            } else {
                let (x, y) = value
                    .trim()
                    .split_once('x')
                    .ok_or_else(|| ctx.invalid("specify logo-offset (XxY)"))?;
                let x: i32 = x
                    .parse()
                    .map_err(|_| ctx.invalid("specify logo-offset (XxY)"))?;
                let y: i32 = y
                    .parse()
                    .map_err(|_| ctx.invalid("specify logo-offset (XxY)"))?;
                s.logo_offset = Vec2::new(x as f32, y as f32);
            }
        }
        "hash-seed" => {
            s.hash_seed = if empty {
                d.hash_seed
            } else {
                ctx.int()? as u32
            };
        }
        "max-files" => {
            if empty {
                s.max_files = d.max_files;
            } else {
                let v = ctx.int()?;
                ctx.check(v >= 0, "must not be negative")?;
                s.max_files = v;
            }
        }
        "font-size" | "file-font-size" | "dir-font-size" | "user-font-size" | "caption-size" => {
            let (field, default) = match name {
                "font-size" => (&mut s.font_size, d.font_size),
                "file-font-size" => (&mut s.filename_font_size, d.filename_font_size),
                "dir-font-size" => (&mut s.dirname_font_size, d.dirname_font_size),
                "user-font-size" => (&mut s.user_font_size, d.user_font_size),
                _ => (&mut s.caption_size, d.caption_size),
            };
            *field = if empty {
                default
            } else {
                let v = ctx.int()?;
                ctx.check((1..=100).contains(&v), "must be between 1 and 100")?;
                v
            };
        }
        "caption-offset" => {
            s.caption_offset = if empty { d.caption_offset } else { ctx.int()? };
        }
        "dir-name-depth" => {
            s.dir_name_depth = if empty {
                d.dir_name_depth
            } else {
                let v = ctx.int()?;
                ctx.check(v > 0 || value.trim() == "0", "must be greater than 0")?;
                v
            };
        }
        "file-idle-time" => {
            s.file_idle_time = if empty {
                d.file_idle_time
            } else {
                parse_idle_time(&ctx)?
            };
        }
        "file-idle-time-at-end" => {
            s.file_idle_time_at_end = if empty {
                d.file_idle_time_at_end
            } else {
                parse_idle_time(&ctx)?
            };
        }
        "seconds-per-day" => {
            s.days_per_second = if empty {
                d.days_per_second
            } else {
                let v = ctx.float()?;
                ctx.check(v > 0.0, "must be greater than 0")?;
                inverse(v)
            };
        }
        "user-friction" => {
            s.user_friction = if empty {
                d.user_friction
            } else {
                let v = ctx.float()?;
                ctx.check(v > 0.0, "must be greater than 0")?;
                inverse(v)
            };
        }
        "start-position" => {
            s.start_position = if empty {
                d.start_position
            } else if value.trim() == "random" {
                let mut rng =
                    KeyedRng::new(s.hash_seed as u64, stream_key(b"settings.start-position"));
                ((rng.rand() % 1000) as f32) / 1000.0
            } else {
                let v = ctx.float()?;
                ctx.check(
                    (v > 0.0 && v < 1.0) || v == 0.0,
                    "start-position outside of range 0.0 - 1.0 (non-inclusive)",
                )?;
                v
            };
        }
        "start-date" | "stop-date" => {
            let ts = if empty {
                0
            } else {
                parse_date_time(value.trim(), |ts| s.utc_offset_at(ts))
                    .ok_or_else(|| ctx.invalid("specify a date as YYYY-MM-DD hh:mm:ss"))?
            };
            let raw = if empty {
                String::new()
            } else {
                value.trim().to_string()
            };
            if name == "start-date" {
                s.start_timestamp = ts;
                s.start_date = raw;
            } else {
                s.stop_timestamp = ts;
                s.stop_date = raw;
            }
        }
        "file-filter" => s.file_filters = ctx.regex_lines()?,
        "file-show-filter" => s.file_show_filters = ctx.regex_lines()?,
        "user-filter" => s.user_filters = ctx.regex_lines()?,
        "user-show-filter" => s.user_show_filters = ctx.regex_lines()?,
        "highlight-user" => s.highlight_users = ctx.lines(),
        "follow-user" => s.follow_users = ctx.lines(),
        "user-style" => {
            let rules = style::parse_rules(value).map_err(|e| OptionError::InvalidValue {
                option: name.to_string(),
                value: style::cap_echo(&e.text, MAX_ECHO_BYTES),
                reason: e.to_string(),
            })?;
            s.user_style = rules.iter().map(|r| r.text().to_string()).collect();
        }
        "ai-style" => {
            s.ai_style = match value.trim() {
                "" | "default" => AiStyle::Default,
                "cape" => AiStyle::Cape,
                "shades" => AiStyle::Shades,
                "hat" => AiStyle::Hat,
                "none" => AiStyle::None,
                _ => return Err(ctx.invalid("specify ai-style (default,cape,shades,hat,none)")),
            };
        }
        "ai-filter" => {
            s.ai_filter = match value.trim() {
                "" | "all" => AiFilter::All,
                "only-assisted" => AiFilter::OnlyAssisted,
                "exclude-assisted" => AiFilter::ExcludeAssisted,
                _ => {
                    return Err(
                        ctx.invalid("specify ai-filter (all,only-assisted,exclude-assisted)")
                    );
                }
            };
        }
        "decor-motion" => {
            s.decor_motion = match value.trim() {
                "" | "full" => true,
                "off" => false,
                _ => return Err(ctx.invalid("specify decor-motion (full,off)")),
            };
        }
        "font-scale" => {
            apply_float(s, &ctx, &d, empty)?;
            // GourceSettings: an explicit --font-scale disables the
            // resolution-based default
            s.default_font_scale = empty;
        }
        _ => return apply_float(s, &ctx, &d, empty),
    }
    Ok(effect_of(name))
}

/// The plain float options: (field, default, validity check, reason).
fn apply_float(
    s: &mut Settings,
    ctx: &Ctx,
    d: &Settings,
    empty: bool,
) -> Result<OptionEffect, OptionError> {
    type Check = fn(f32) -> bool;
    let name = ctx.option;
    let (field, default, ok, reason): (&mut f32, f32, Check, &str) = match name {
        "bloom-intensity" => (
            &mut s.bloom_intensity,
            d.bloom_intensity,
            |v| v > 0.0,
            "must be greater than 0",
        ),
        "bloom-multiplier" => (
            &mut s.bloom_multiplier,
            d.bloom_multiplier,
            |v| v > 0.0,
            "must be greater than 0",
        ),
        "elasticity" => (
            &mut s.elasticity,
            d.elasticity,
            |v| v >= 0.0,
            "must be greater than 0",
        ),
        "auto-skip-seconds" => (
            &mut s.auto_skip_seconds,
            d.auto_skip_seconds,
            |v| v != 0.0,
            "must not be 0",
        ),
        "stop-at-time" => (
            &mut s.stop_at_time,
            d.stop_at_time,
            |v| v > 0.0 || v == -1.0,
            "must be greater than 0",
        ),
        "max-user-speed" => (
            &mut s.max_user_speed,
            d.max_user_speed,
            |v| v > 0.0,
            "must be greater than 0",
        ),
        "padding" => (
            &mut s.padding,
            d.padding,
            |v| v > 0.0 && v < 2.0,
            "must be between 0 and 2 (exclusive)",
        ),
        "time-scale" => (
            &mut s.time_scale,
            d.time_scale,
            |v| v > 0.0 && v <= 4.0,
            "time-scale outside of range 0.0 - 4.0",
        ),
        "dir-name-position" => (
            &mut s.dir_name_position,
            d.dir_name_position,
            |v| (0.1..=1.0).contains(&v),
            "dir-name-position outside of range 0.1 - 1.0 (inclusive)",
        ),
        "loop-delay-seconds" => (
            &mut s.loop_delay_seconds,
            d.loop_delay_seconds,
            |v| v > 0.0,
            "must be greater than 0",
        ),
        "font-scale" => (
            &mut s.font_scale,
            d.font_scale,
            |v| (0.0..=10.0).contains(&v),
            "must be between 0 and 10",
        ),
        "caption-duration" => (
            &mut s.caption_duration,
            d.caption_duration,
            |v| v > 0.0,
            "must be greater than 0",
        ),
        "filename-time" => (
            &mut s.filename_time,
            d.filename_time,
            |v| v >= 2.0,
            "filename-time must be >= 2.0",
        ),
        "user-idle-time" => (
            &mut s.user_idle_time,
            d.user_idle_time,
            |v| v >= 0.0,
            "must not be negative",
        ),
        "max-file-lag" => (
            &mut s.max_file_lag,
            d.max_file_lag,
            |v| v != 0.0,
            "must not be 0",
        ),
        "user-scale" => (
            &mut s.user_scale,
            d.user_scale,
            |v| v > 0.0 && v <= 100.0,
            "must be between 0 and 100",
        ),
        "stop-position" => (
            &mut s.stop_position,
            d.stop_position,
            |v| (v > 0.0 && v <= 1.0) || v == 0.0,
            "stop-position outside of range 0.0 - 1.0 (inclusive)",
        ),
        _ => return Err(OptionError::UnknownOption(name.to_string())),
    };
    *field = if empty {
        default
    } else {
        let v = ctx.float()?;
        ctx.check(ok(v), reason)?;
        v
    };
    Ok(effect_of(name))
}

pub(crate) fn ai_style_name(style: AiStyle) -> &'static str {
    match style {
        AiStyle::Default => "default",
        AiStyle::Cape => "cape",
        AiStyle::Shades => "shades",
        AiStyle::Hat => "hat",
        AiStyle::None => "none",
    }
}

pub(crate) fn ai_filter_name(filter: AiFilter) -> &'static str {
    match filter {
        AiFilter::All => "all",
        AiFilter::OnlyAssisted => "only-assisted",
        AiFilter::ExcludeAssisted => "exclude-assisted",
    }
}

pub(crate) fn value(s: &Settings, name: &str) -> Option<String> {
    let name = canonical_name(name);
    if let Some(v) = simple_bool_value(s, name) {
        return Some(v.to_string());
    }
    let b = |v: bool| Some(v.to_string());
    let f = |v: f32| Some(fmt_f32(v));
    let i = |v: i32| Some(v.to_string());
    let lines = |v: &[String]| Some(v.join("\n"));
    match name {
        "hide" => {
            let list: Vec<&str> = HIDE_FIELDS
                .iter()
                .copied()
                .filter(|f| hide_value(s, f))
                .collect();
            Some(list.join(","))
        }
        "hide-mouse" => b(s.hide_mouse),
        "hide-pull-requests" => b(s.hide_pull_requests),
        "pull-request-mode" => Some(
            match s.pull_request_mode {
                PullRequestMode::Basic => "basic",
                PullRequestMode::Full => "full",
            }
            .to_string(),
        ),
        "pull-request-style" => Some(
            match s.pull_request_style {
                PullRequestStyle::DimHighlight => "dim-highlight",
            }
            .to_string(),
        ),
        "disable-auto-skip" => b(s.auto_skip_seconds < 0.0),
        "realtime" => b(s.days_per_second == realtime_days_per_second()),
        "date-format" => Some(s.date_format.clone()),
        "title" => Some(s.title.clone()),
        "crop" => Some(
            if s.crop_vertical {
                "vertical"
            } else if s.crop_horizontal {
                "horizontal"
            } else {
                ""
            }
            .to_string(),
        ),
        "rand-mode" => Some(
            match s.rand_mode {
                RandMode::Keyed => "keyed",
                RandMode::Glibc => "glibc",
            }
            .to_string(),
        ),
        "camera-mode" => Some(
            match s.camera_mode {
                CameraMode::Overview => "overview",
                CameraMode::Track => "track",
            }
            .to_string(),
        ),
        "font-colour" => Some(format_colour(s.font_colour)),
        "background-colour" => Some(format_colour(s.background_colour)),
        "highlight-colour" => Some(format_colour(s.highlight_colour)),
        "selection-colour" => Some(format_colour(s.selection_colour)),
        "dir-colour" => Some(format_colour(s.dir_colour)),
        "filename-colour" => Some(format_colour(s.filename_colour)),
        "caption-colour" => Some(format_colour(s.caption_colour)),
        "logo-offset" => Some(format!(
            "{}x{}",
            s.logo_offset.x as i32, s.logo_offset.y as i32
        )),
        "hash-seed" => i(s.hash_seed as i32),
        "max-files" => i(s.max_files),
        "font-size" => i(s.font_size),
        "file-font-size" => i(s.filename_font_size),
        "dir-font-size" => i(s.dirname_font_size),
        "user-font-size" => i(s.user_font_size),
        "caption-size" => i(s.caption_size),
        "caption-offset" => i(s.caption_offset),
        "dir-name-depth" => i(s.dir_name_depth),
        "file-idle-time" => i(s.file_idle_time as i32),
        "file-idle-time-at-end" => i(s.file_idle_time_at_end as i32),
        "seconds-per-day" => f(inverse(s.days_per_second)),
        "user-friction" => f(inverse(s.user_friction)),
        "start-position" => f(s.start_position),
        "start-date" => Some(s.start_date.clone()),
        "stop-date" => Some(s.stop_date.clone()),
        "file-filter" => lines(&s.file_filters),
        "file-show-filter" => lines(&s.file_show_filters),
        "user-filter" => lines(&s.user_filters),
        "user-show-filter" => lines(&s.user_show_filters),
        "highlight-user" => lines(&s.highlight_users),
        "follow-user" => lines(&s.follow_users),
        "user-style" => lines(&s.user_style),
        "ai-style" => Some(ai_style_name(s.ai_style).to_string()),
        "ai-filter" => Some(ai_filter_name(s.ai_filter).to_string()),
        "decor-motion" => Some(if s.decor_motion { "full" } else { "off" }.to_string()),
        "bloom-intensity" => f(s.bloom_intensity),
        "bloom-multiplier" => f(s.bloom_multiplier),
        "elasticity" => f(s.elasticity),
        "auto-skip-seconds" => f(s.auto_skip_seconds),
        "stop-at-time" => f(s.stop_at_time),
        "max-user-speed" => f(s.max_user_speed),
        "padding" => f(s.padding),
        "time-scale" => f(s.time_scale),
        "dir-name-position" => f(s.dir_name_position),
        "loop-delay-seconds" => f(s.loop_delay_seconds),
        "font-scale" => f(s.font_scale),
        "caption-duration" => f(s.caption_duration),
        "filename-time" => f(s.filename_time),
        "user-idle-time" => f(s.user_idle_time),
        "max-file-lag" => f(s.max_file_lag),
        "user-scale" => f(s.user_scale),
        "stop-position" => f(s.stop_position),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply_ok(s: &mut Settings, name: &str, value: &str) -> OptionEffect {
        s.apply_option(name, value)
            .unwrap_or_else(|e| panic!("{name}={value}: {e}"))
    }

    fn is_colour(name: &str) -> bool {
        name.ends_with("-colour")
    }

    #[test]
    fn every_option_value_applies_back_unchanged() {
        let s = Settings::default();
        for &name in OPTION_NAMES {
            let v = s
                .option_value(name)
                .unwrap_or_else(|| panic!("{name} has no value"));
            let mut t = s.clone();
            apply_ok(&mut t, name, &v);
            if is_colour(name) {
                // rounded to 1/255, then a fixed point
                assert_eq!(t.option_value(name).unwrap(), v, "{name}");
            } else if name == "font-scale" {
                // the value applies back, but is now explicit
                assert!(!t.default_font_scale);
                t.default_font_scale = true;
                assert_eq!(t, s, "{name}={v:?} changed the defaults");
            } else {
                assert_eq!(t, s, "{name}={v:?} changed the defaults");
            }
        }
        assert_eq!(s.option_value("background-colour").unwrap(), "1A1A1A");
        let mut t = s.clone();
        apply_ok(&mut t, "background-colour", "vec3(0.1, 0.1, 0.1)");
        assert_eq!(t, s);
    }

    #[test]
    fn options_round_trip() {
        let cases: &[(&str, &str, &str)] = &[
            // (name, value applied, value reported)
            ("hide", "date,users,filenames", "date,users,filenames"),
            ("hide", ",root,,bloom,", "bloom,root"),
            ("hide", "mouse", "progress,mouse"),
            ("hide", "", ""),
            ("hide-tree", "true", "true"),
            ("hide-tree", "no", "false"),
            ("key", "", "true"),
            ("background-colour", "FF8000", "FF8000"),
            ("background", "00ff7f", "00FF7F"),
            ("b", "102030", "102030"),
            ("seconds-per-day", "10", "10"),
            ("s", "0.5", "0.5"),
            ("realtime", "true", "true"),
            ("user-friction", "0.5", "0.5"),
            ("file-idle-time", "3.7", "3"),
            ("i", "0", "0"),
            ("camera-mode", "track", "track"),
            ("rand-mode", "glibc", "glibc"),
            ("rand-mode", "", "keyed"),
            ("crop", "vertical", "vertical"),
            ("crop", "horizontal", "horizontal"),
            ("padding", "1.5", "1.5"),
            ("time-scale", "4", "4"),
            ("bloom-multiplier", "1.25", "1.25"),
            ("bloom-intensity", "0.5", "0.5"),
            ("elasticity", "0.3", "0.3"),
            ("max-user-speed", "250", "250"),
            ("auto-skip-seconds", "1.5", "1.5"),
            ("disable-auto-skip", "true", "true"),
            ("font-scale", "2", "2"),
            ("font-size", "20", "20"),
            ("file-font-size", "10", "10"),
            ("dir-font-size", "12", "12"),
            ("user-font-size", "18", "18"),
            ("dir-name-depth", "3", "3"),
            ("dir-name-position", "0.25", "0.25"),
            ("highlight-dirs", "true", "true"),
            ("highlight-all-users", "1", "true"),
            ("highlight-user", "alice\nbob\n\n", "alice\nbob"),
            ("follow-user", "carol", "carol"),
            ("file-extensions", "true", "true"),
            ("user-scale", "2.5", "2.5"),
            ("filename-time", "6", "6"),
            ("max-file-lag", "-1", "-1"),
            ("max-files", "100", "100"),
            ("hash-seed", "-5", "-5"),
            ("title", "My repo", "My repo"),
            ("date-format", "%Y-%m-%d", "%Y-%m-%d"),
            (
                "file-filter",
                "\\.png$\nnode_modules",
                "\\.png$\nnode_modules",
            ),
            ("file-show-filter", "^src/", "^src/"),
            ("user-filter", "bot$", "bot$"),
            ("user-show-filter", "^a", "^a"),
            ("start-date", "2020-01-02 03:04:05", "2020-01-02 03:04:05"),
            ("stop-date", "2021-06-01", "2021-06-01"),
            ("start-position", "0.25", "0.25"),
            ("p", "0.5", "0.5"),
            ("stop-position", "1", "1"),
            ("stop-at-time", "30", "30"),
            ("loop", "true", "true"),
            ("loop-delay-seconds", "2", "2"),
            ("no-time-travel", "true", "true"),
            ("logo-offset", "10x30", "10x30"),
            ("user-idle-time", "5", "5"),
            ("hide", "root,decorations", "root,decorations"),
            ("hide-decorations", "true", "true"),
            ("agent-key", "true", "true"),
            ("ai-style", "hat", "hat"),
            ("ai-style", "none", "none"),
            ("ai-style", "", "default"),
            ("ai-filter", "only-assisted", "only-assisted"),
            ("ai-filter", "exclude-assisted", "exclude-assisted"),
            ("ai-filter", "", "all"),
            ("decor-motion", "off", "off"),
            ("decor-motion", "", "full"),
            (
                "user-style",
                "# my look\n  ai:codex => hat=agent; glow=agent  \n\n!ai & human => badge=FF8000\r\n",
                "ai:codex => hat=agent; glow=agent\n!ai & human => badge=FF8000",
            ),
        ];
        for &(name, value, reported) in cases {
            let mut s = Settings::default();
            apply_ok(&mut s, name, value);
            let v = s.option_value(name).unwrap();
            assert_eq!(v, reported, "{name}={value:?}");
            let mut t = s.clone();
            apply_ok(&mut t, name, &v);
            assert_eq!(t, s, "{name}: re-applying {v:?} changed the settings");
        }
    }

    #[test]
    fn values_land_in_the_settings_as_gource_stores_them() {
        let mut s = Settings::default();
        apply_ok(&mut s, "seconds-per-day", "10");
        assert_eq!(s.days_per_second, (1.0f64 / 10.0) as f32);
        apply_ok(&mut s, "user-friction", "0.5");
        assert_eq!(s.user_friction, 2.0);
        apply_ok(&mut s, "background-colour", "FF0080");
        assert_eq!(s.background_colour, Vec3::new(1.0, 0.0, 128.0 / 255.0));
        apply_ok(&mut s, "hide", "mouse");
        assert!(s.hide_mouse && s.hide_progress && !s.hide_date);
        apply_ok(&mut s, "realtime", "true");
        assert_eq!(s.days_per_second, (1.0f64 / 86400.0) as f32);
        apply_ok(&mut s, "realtime", "false");
        assert_eq!(s.days_per_second, 0.1);
        apply_ok(&mut s, "disable-auto-skip", "true");
        assert_eq!(s.auto_skip_seconds, -1.0);
        apply_ok(&mut s, "disable-auto-skip", "false");
        assert_eq!(s.auto_skip_seconds, 3.0);
        apply_ok(&mut s, "hash-seed", "-5");
        assert_eq!(s.hash_seed as i32, -5);
        apply_ok(&mut s, "start-date", "2010-01-01T00:00:00Z");
        assert_eq!(s.start_timestamp, 1262304000);
        apply_ok(&mut s, "start-date", "");
        assert_eq!((s.start_timestamp, s.start_date.as_str()), (0, ""));
        s.utc_offset_minutes = 60;
        apply_ok(&mut s, "stop-date", "2010-01-01");
        assert_eq!(s.stop_timestamp, 1262304000 - 3600);
        apply_ok(&mut s, "font-scale", "2");
        assert_eq!(s.scaled_font_size(), 32);
        apply_ok(&mut s, "start-position", "random");
        assert!(s.start_position >= 0.0 && s.start_position < 1.0);
        apply_ok(&mut s, "title", "");
        assert_eq!(s.title, "");
        apply_ok(&mut s, "--key", "true");
        assert!(s.show_key);
    }

    #[test]
    fn restart_classification() {
        let restart = [
            ("file-filter", "x"),
            ("file-show-filter", "x"),
            ("user-filter", "x"),
            ("user-show-filter", "x"),
            ("start-date", "2020-01-01"),
            ("stop-date", "2020-01-01"),
            ("start-position", "0.5"),
            ("p", "0.5"),
            ("stop-position", "0.5"),
            ("rand-mode", "glibc"),
            ("ai-filter", "only-assisted"),
            ("max-files", "2"),
        ];
        for (name, value) in restart {
            let mut s = Settings::default();
            assert_eq!(
                apply_ok(&mut s, name, value),
                OptionEffect::Restart,
                "{name}"
            );
        }
        let defaults = Settings::default();
        for &name in OPTION_NAMES {
            if restart.iter().any(|(r, _)| *r == name) {
                continue;
            }
            let v = defaults.option_value(name).unwrap();
            let mut s = Settings::default();
            assert_eq!(apply_ok(&mut s, name, &v), OptionEffect::Live, "{name}");
        }
    }

    #[test]
    fn invalid_values_are_rejected_like_gource() {
        let bad: &[(&str, &str)] = &[
            ("bloom-intensity", "0"),
            ("bloom-multiplier", "-1"),
            ("elasticity", "-0.5"),
            ("padding", "2"),
            ("padding", "0"),
            ("time-scale", "5"),
            ("time-scale", "0"),
            ("seconds-per-day", "0"),
            ("seconds-per-day", "fast"),
            ("user-friction", "0"),
            ("max-user-speed", "0"),
            ("user-scale", "101"),
            ("filename-time", "1.5"),
            ("dir-name-position", "0.05"),
            ("dir-name-depth", "-1"),
            ("font-size", "0"),
            ("font-size", "101"),
            ("font-scale", "11"),
            ("max-files", "-1"),
            ("max-file-lag", "0"),
            ("auto-skip-seconds", "0"),
            ("file-idle-time", "-1"),
            ("file-idle-time", "soon"),
            ("start-position", "1"),
            ("stop-position", "1.5"),
            ("stop-at-time", "0"),
            ("background-colour", "GGGGGG"),
            ("background-colour", "FFF"),
            ("camera-mode", "orbit"),
            ("rand-mode", "glib"),
            ("crop", "diagonal"),
            ("hide", "date,clouds"),
            ("hide-date", "maybe"),
            ("file-filter", "(unclosed"),
            ("start-date", "yesterday"),
            ("stop-date", "2020-1-1"),
            ("logo-offset", "10,10"),
            ("hide", "decoration"),
            ("ai-style", "glowing"),
            ("ai-filter", "assisted"),
            ("decor-motion", "reduced"),
            ("agent-key", "maybe"),
            ("user-style", "ai => capes=on"),
        ];
        for &(name, value) in bad {
            let mut s = Settings::default();
            match s.apply_option(name, value) {
                Err(OptionError::InvalidValue { option, .. }) => {
                    assert_eq!(option, canonical_name(name), "{name}={value}")
                }
                other => panic!("{name}={value}: expected an error, got {other:?}"),
            }
            assert_eq!(
                s,
                Settings::default(),
                "{name}={value} changed the settings"
            );
        }
        assert_eq!(
            Settings::default().apply_option("no-such-option", "1"),
            Err(OptionError::UnknownOption("no-such-option".into()))
        );
        assert_eq!(Settings::default().option_value("no-such-option"), None);
    }

    #[test]
    fn hide_list_keeps_its_order_with_decorations_last() {
        let mut s = Settings::default();
        apply_ok(&mut s, "hide", "decorations,date,root");
        assert!(s.hide_decorations);
        assert_eq!(s.option_value("hide").unwrap(), "date,root,decorations");
        apply_ok(&mut s, "hide-decorations", "false");
        assert_eq!(s.option_value("hide").unwrap(), "date,root");
    }

    #[test]
    fn user_style_errors_carry_code_line_and_capped_echo() {
        let mut s = Settings::default();
        apply_ok(&mut s, "user-style", "ai => badge");
        let long = format!("ai => cape=on; {}", "x".repeat(120));
        let text = format!("# c\nai => badge=on\n{long}");
        match s.apply_option("user-style", &text) {
            Err(OptionError::InvalidValue {
                option,
                value,
                reason,
            }) => {
                assert_eq!(option, "user-style");
                assert!(reason.starts_with("style_bad_property: "), "{reason}");
                assert!(reason.ends_with("(line 3)"), "{reason}");
                assert_eq!(value.len(), MAX_ECHO_BYTES + 3);
                assert!(value.ends_with("..."));
            }
            other => panic!("{other:?}"),
        }
        // the previous rules stay
        assert_eq!(s.option_value("user-style").unwrap(), "ai => badge");
    }

    #[test]
    fn name_bearing_options_are_options() {
        assert_eq!(NAME_BEARING_OPTIONS.len(), 8);
        for name in NAME_BEARING_OPTIONS {
            assert!(OPTION_NAMES.contains(name), "{name}");
        }
    }
}
