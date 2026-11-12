// SPDX-License-Identifier: GPL-3.0-or-later

//! The recipe (`recording.md` section 5.10, D9): the settings that made a
//! video, as JSON, in two renderings.
//!
//! - **Embedded** (REC-71), written into the file before the first frame:
//!   only reproducible inputs. No time zone name, no creation time, and no
//!   values of the [`NAME_BEARING_OPTIONS`] (or `record-name`); their names
//!   are listed in `redacted`. For private and unknown-provenance sources
//!   (a local file, `?log=`) `source.path`, `branch`, `subdir` and `head`
//!   are left out as well unless the viewer confirmed them.
//! - **Sidecar** (REC-72), the `.recipe.json` offered at the end: every
//!   value plus the `result` of the run; for private and unknown-provenance
//!   sources `source.path` and `head` are redacted unless confirmed.
//!
//! `options` holds the non-default Gource options of the snapshot before
//! the forced overrides (those follow from the recording options), as
//! `Settings::option_value` reports them (multi-value options joined by
//! `\n`), plus `hash-seed` always (REC-75). `record` holds the non-default
//! recording options, with `record-viewport` and `record-ui-scale` always
//! present and resolved (REC-70). Reproducing is applying `options` to the
//! default settings by name and `record` to the recording options: same
//! build and same log give the same frames (REC-73).
//!
//! The page supplies what only it knows as a context object:
//! `{"build", "source": {"kind": "repo"|"file"|"url", "host", "path",
//! "branch", "since", "until", "subdir", "head", "private"}, "confirmed",
//! "time_zone", "created", "result": {...}}`.

use gource_core::settings::Settings;

use crate::record::json::{self, Value, number, string};
use crate::record::options::{RECORD_OPTION_DEFS, RecordOptions};
use crate::record::setup::RecordSetup;
use crate::record::{NAME_BEARING_OPTIONS, RecordError, codes};

/// Recipe format version.
pub const RECIPE_VERSION: u32 = 1;

/// `engine` field.
pub const ENGINE: &str = concat!("gource-core ", env!("CARGO_PKG_VERSION"));

/// Option names covered by another name in the recipe (`hide` lists the
/// hide elements; `auto-skip-seconds` and `seconds-per-day` carry
/// `disable-auto-skip` and `realtime`).
fn covered(name: &str) -> bool {
    name.starts_with("hide-") || matches!(name, "disable-auto-skip" | "realtime")
}

/// Which rendering to produce.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum RecipeKind {
    Embedded,
    Sidecar,
}

impl RecipeKind {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "embedded" => Some(RecipeKind::Embedded),
            "sidecar" => Some(RecipeKind::Sidecar),
            _ => None,
        }
    }
}

/// The non-default Gource options of `settings`, in option-table order,
/// with `hash-seed` always.
pub fn non_default_options(settings: &Settings) -> Vec<(&'static str, String)> {
    let defaults = Settings::default();
    Settings::option_names()
        .iter()
        .copied()
        .filter(|name| !covered(name))
        .filter(|&name| name != "font-scale" || !settings.default_font_scale)
        .filter_map(|name| {
            let v = settings.option_value(name)?;
            (name == "hash-seed" || defaults.option_value(name).as_deref() != Some(v.as_str()))
                .then_some((name, v))
        })
        .collect()
}

/// The recording options of the recipe: non-default values, with the
/// resolved size and `ui_scale`.
pub fn record_options(setup: &RecordSetup) -> Vec<(&'static str, String)> {
    let opts: &RecordOptions = setup.options();
    RECORD_OPTION_DEFS
        .iter()
        .filter_map(|d| match d.name {
            "record-viewport" => Some((d.name, format!("{}x{}", setup.width, setup.height))),
            "record-ui-scale" => Some((d.name, setup.ui_scale.to_string())),
            name => {
                let v = opts.value(name)?;
                (v != d.default).then_some((name, v))
            }
        })
        .collect()
}

fn pairs(list: Vec<(&str, String)>) -> Value {
    Value::Object(
        list.into_iter()
            .map(|(k, v)| (k.to_string(), Value::String(v)))
            .collect(),
    )
}

/// Build the recipe of `setup` in the `kind` rendering. `context_json` is
/// the page's context (see the module docs); `result` the recorder's
/// result fields (sidecar only); `first_commit_ts` the timestamp of the
/// first commit of the run (for `utc_offset_minutes`).
pub fn recipe(
    setup: &RecordSetup,
    context_json: &str,
    kind: RecipeKind,
    result: Option<Value>,
    first_commit_ts: Option<i64>,
) -> Result<Value, RecordError> {
    let text = if context_json.trim().is_empty() {
        "{}"
    } else {
        context_json
    };
    let ctx = json::parse(text)
        .map_err(|e| RecordError::new(codes::INVALID_OPTION, format!("recipe context: {e}")))?;
    if ctx.members().is_none() {
        return Err(RecordError::new(
            codes::INVALID_OPTION,
            "recipe context must be a JSON object",
        ));
    }
    let embedded = kind == RecipeKind::Embedded;
    let mut redacted: Vec<Value> = Vec::new();

    // source
    let src = ctx.get("source");
    let src_kind = src
        .and_then(|s| s.get("kind"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let private = src
        .and_then(|s| s.get("private"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let confirmed = ctx
        .get("confirmed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    // a log of unknown provenance may come from a private repository
    let sensitive = (private || src_kind != "repo") && !confirmed;
    let hidden: &[&str] = match (sensitive, embedded) {
        (false, _) => &[],
        (true, true) => &["path", "branch", "subdir", "head"],
        (true, false) => &["path", "head"],
    };
    let mut source = Vec::new();
    for (k, v) in src.and_then(Value::members).unwrap_or(&[]) {
        if hidden.contains(&k.as_str()) {
            if v.as_str().is_some_and(|s| !s.is_empty()) {
                redacted.push(string(&format!("source.{k}")));
            }
            continue;
        }
        source.push((k.clone(), v.clone()));
    }

    // options
    let mut options = Vec::new();
    for (name, value) in non_default_options(&setup.recipe_settings) {
        if embedded && NAME_BEARING_OPTIONS.contains(&name) {
            redacted.push(string(name));
        } else {
            options.push((name, value));
        }
    }
    let mut record = Vec::new();
    for (name, value) in record_options(setup) {
        if embedded && name == "record-name" {
            redacted.push(string(name));
        } else {
            record.push((name, value));
        }
    }

    let plan = &setup.plan;
    let utc = first_commit_ts.map_or(setup.sim_settings.utc_offset_minutes, |ts| {
        setup.sim_settings.utc_offset_at(ts)
    });
    let build = ctx.get("build").and_then(Value::as_str).unwrap_or("");
    let mut out: Vec<(String, Value)> = vec![
        ("v".into(), number(f64::from(RECIPE_VERSION))),
        ("app".into(), string("gource-web")),
        ("build".into(), string(build)),
        ("engine".into(), string(ENGINE)),
        ("source".into(), Value::Object(source)),
        ("options".into(), pairs(options)),
        ("record".into(), pairs(record)),
        ("claims".into(), Value::Array(Vec::new())),
        (
            "plan".into(),
            json::object([
                ("fps", number(f64::from(plan.fps))),
                ("ticks_per_frame", number(f64::from(plan.ticks_per_frame))),
                ("fixed_dt", Value::Number(plan.fixed_dt.to_string())),
                ("width", number(f64::from(setup.width))),
                ("height", number(f64::from(setup.height))),
                ("ui_scale", Value::Number(setup.ui_scale.to_string())),
            ]),
        ),
        ("utc_offset_minutes".into(), number(f64::from(utc))),
        ("redacted".into(), Value::Array(redacted)),
    ];
    if !embedded {
        let mut res: Vec<(String, Value)> = ctx
            .get("result")
            .and_then(Value::members)
            .map(<[_]>::to_vec)
            .unwrap_or_default();
        for (k, v) in result.as_ref().and_then(Value::members).unwrap_or(&[]) {
            res.retain(|(key, _)| key != k);
            res.push((k.clone(), v.clone()));
        }
        out.push(("result".into(), Value::Object(res)));
        for key in ["time_zone", "created"] {
            if let Some(v) = ctx.get(key) {
                out.push((key.into(), v.clone()));
            }
        }
    }
    Ok(Value::Object(out))
}
