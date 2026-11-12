// SPDX-License-Identifier: GPL-3.0-or-later

//! The contributor style system: a rule list that maps a contributor's
//! current attribution state to decorations (AI-37 to AI-47).
//!
//! A rule is `selector => properties`, for example
//! `ai:codex => hat=agent; glow=agent`. Rules apply in order and cascade:
//! for each decoration the last matching rule that sets it wins, and `off`
//! clears an earlier `on`. The effective list is three layers, built-in
//! (from `ai-style`, [`BUILTIN_RULES_V1`] by default), then a claimed layer
//! (reserved for contributor-claimed styles, always empty here), then the
//! viewer's own rules (`user-style`). The built-in rules are parsed by the
//! same parser as the viewer's.
//!
//! Grammar of this milestone (M2):
//!
//! ```text
//! rule     = selector "=>" props            ; split at the last "=>"
//! selector = term *( "&" term )             ; 1..4 terms, AND
//! term     = [ "!" ] ( "human" / "agent" / "assisted" / "ai" / "ai:" agent-id )
//! props    = prop *( ";" prop ) [ ";" ]
//! prop     = ( "cape" / "halo" / "shades" / "hat" / "badge" / "glow" ) [ "=" value ]
//!          / "none"
//! value    = "on" / "off" / "agent" / RRGGBB
//! ```
//!
//! Blank lines and lines starting with `#` are ignored. `bot`, `habitual`,
//! `name:/.../`, `ai:id/glob`, the value `hash` and the properties `colour`
//! and `scale` belong to a later milestone and are rejected; `id:` is
//! reserved ([`codes::RESERVED`]).
//!
//! Polarity: a rule with a positive `agent`, `assisted`, `ai` or `ai:id`
//! term is dynamic (its decorations fade with the assist envelope); any
//! other rule is static (drawn at the avatar's alpha).

use std::fmt;

use crate::agents;
use crate::events::AttrKind;
use crate::settings::AiStyle;
use crate::view::DecoKind;

/// Built-in rules, version 1 (`ai-style=default`).
pub const BUILTIN_RULES_V1: &str = "ai       => badge=on\n\
                                    assisted => cape=agent; shades=on\n\
                                    agent    => halo=agent; glow=agent";

const BUILTIN_CAPE: &str = "ai       => badge=on\n\
                            assisted => cape=agent\n\
                            agent    => halo=agent; glow=agent";

const BUILTIN_SHADES: &str = "ai       => badge=on\n\
                              assisted => shades=on\n\
                              agent    => halo=agent; glow=agent";

const BUILTIN_HAT: &str = "ai       => badge=on\n\
                           assisted => hat=agent; glow=agent\n\
                           agent    => halo=agent; glow=agent";

/// Most rules in one rule list.
pub const MAX_RULES: usize = 32;
/// Longest rule line in bytes.
pub const MAX_LINE_BYTES: usize = 256;
/// Most `&`-joined terms in one selector.
pub const MAX_TERMS: usize = 4;
/// Most non-blank lines in one rule list, comments included.
pub const MAX_LINES: usize = 64;
/// Longest offending line echoed in an option error.
pub const MAX_ECHO_BYTES: usize = 80;

/// The lens colour of `shades=on`.
pub const SHADES_LENS: [u8; 3] = [0x1A, 0x1A, 0x24];

/// Error codes (stable; the page shows them after "Line N:").
pub mod codes {
    pub const MISSING_ARROW: &str = "style_missing_arrow";
    pub const BAD_SELECTOR: &str = "style_bad_selector";
    pub const BAD_PROPERTY: &str = "style_bad_property";
    pub const BAD_VALUE: &str = "style_bad_value";
    pub const TOO_MANY_TERMS: &str = "style_too_many_terms";
    pub const TOO_MANY_RULES: &str = "style_too_many_rules";
    pub const TOO_MANY_LINES: &str = "style_too_many_lines";
    pub const LINE_TOO_LONG: &str = "style_line_too_long";
    pub const RESERVED: &str = "style_reserved";
}

/// A rejected rule list: the first problem found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StyleError {
    /// One of [`codes`].
    pub code: &'static str,
    /// 1-based line number over all lines of the input.
    pub line: u32,
    /// Human-readable text, without code and line.
    pub message: String,
    /// The offending line, trimmed.
    pub text: String,
}

impl fmt::Display for StyleError {
    /// `<code>: <message> (line N)`, the `reason` of the option error.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {} (line {})", self.code, self.message, self.line)
    }
}

impl std::error::Error for StyleError {}

/// `s` cut to at most `max` bytes at a character boundary, with `...`
/// appended when cut.
pub fn cap_echo(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}...", &s[..end])
}

/// A property value.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Value {
    Off,
    On,
    Agent,
    Rgb([u8; 3]),
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Test {
    Human,
    Agent,
    Assisted,
    Ai,
    /// `ai:<agent-id>`.
    AiAgent(Box<str>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Term {
    negated: bool,
    test: Test,
}

/// One parsed rule.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// The rule line, trimmed (what `option_value("user-style")` reports).
    text: Box<str>,
    terms: Vec<Term>,
    /// Per [`DecoKind`], the value this rule sets (properties applied in
    /// order, so `none; badge` leaves only the badge on).
    props: [Option<Value>; DecoKind::COUNT],
    /// Positive polarity: the decorations follow the assist envelope.
    dynamic: bool,
}

impl Rule {
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether the decorations this rule sets fade with the assist state.
    pub fn is_dynamic(&self) -> bool {
        self.dynamic
    }

    fn matches(&self, s: &Subject) -> bool {
        self.terms.iter().all(|t| t.test.holds(s) != t.negated)
    }

    /// The entry `agent` and `on` tints and the badge come from: the entry
    /// of a positive `ai:id` term, else the first AI entry.
    fn entry<'s>(&self, s: Subject<'s>) -> Option<&'s SubjectEntry<'s>> {
        for t in &self.terms {
            if let (false, Test::AiAgent(id)) = (t.negated, &t.test) {
                return s.ai_entries().find(|e| e.agent == &**id);
            }
        }
        s.ai_entries().next()
    }
}

impl Test {
    fn holds(&self, s: &Subject) -> bool {
        match self {
            Test::Human => !s.has(AttrKind::Agent) && !s.has(AttrKind::Bot),
            Test::Agent => s.has(AttrKind::Agent),
            Test::Assisted => s.has(AttrKind::Assist),
            Test::Ai => s.ai_entries().next().is_some(),
            Test::AiAgent(id) => s.ai_entries().any(|e| e.agent == &**id),
        }
    }

    fn is_state(&self) -> bool {
        !matches!(self, Test::Human)
    }
}

/// One entry of a contributor's current attribution set, as the resolver
/// sees it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct SubjectEntry<'a> {
    pub kind: AttrKind,
    /// Agent id.
    pub agent: &'a str,
    /// Agent tint ([`agents::resolve`]).
    pub tint: [u8; 3],
    /// Badge shape index.
    pub badge: u8,
}

impl SubjectEntry<'_> {
    fn is_ai(&self) -> bool {
        matches!(self.kind, AttrKind::Assist | AttrKind::Agent)
    }
}

/// A contributor's current attribution set (empty when not credited).
#[derive(Copy, Clone, Debug, Default)]
pub struct Subject<'a> {
    pub entries: &'a [SubjectEntry<'a>],
}

impl<'a> Subject<'a> {
    fn has(&self, kind: AttrKind) -> bool {
        self.entries.iter().any(|e| e.kind == kind)
    }

    fn ai_entries(self) -> impl Iterator<Item = &'a SubjectEntry<'a>> {
        self.entries.iter().filter(|e| e.is_ai())
    }
}

/// Where a decoration's tint comes from.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub enum Tint {
    /// Not drawn.
    #[default]
    Off,
    Fixed([u8; 3]),
    /// The contributor's own colour, read at extraction (follows
    /// `hash-seed`).
    Contributor,
}

/// The resolved look of one contributor state: small and `Copy`, cached per
/// user until the rules or the state change (AI-47).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct ResolvedStyle {
    pub tints: [Tint; DecoKind::COUNT],
    /// [`DecoKind::bit`]s that follow the assist envelope.
    pub dynamic: u8,
    /// Badge shape index.
    pub badge: u8,
}

impl ResolvedStyle {
    /// Bits of the decorations that are drawn.
    pub fn mask(&self) -> u8 {
        DecoKind::ALL
            .iter()
            .filter(|k| self.tints[k.index()] != Tint::Off)
            .fold(0, |m, k| m | k.bit())
    }
}

/// The text of the built-in layer for `style`.
pub fn builtin_rules(style: AiStyle) -> &'static str {
    match style {
        AiStyle::Default => BUILTIN_RULES_V1,
        AiStyle::Cape => BUILTIN_CAPE,
        AiStyle::Shades => BUILTIN_SHADES,
        AiStyle::Hat => BUILTIN_HAT,
        AiStyle::None => "",
    }
}

/// Parse a rule list (one rule per line). Nothing is applied on error.
pub fn parse_rules(text: &str) -> Result<Vec<Rule>, StyleError> {
    let mut rules = Vec::new();
    let mut lines = 0;
    for (i, raw) in text.split('\n').enumerate() {
        let line_no = (i + 1) as u32;
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let fail = |code: &'static str, message: String| StyleError {
            code,
            line: line_no,
            message,
            text: line.to_string(),
        };
        lines += 1;
        if lines > MAX_LINES {
            return Err(fail(
                codes::TOO_MANY_LINES,
                format!("At most {MAX_LINES} lines, comments included."),
            ));
        }
        if line.starts_with('#') {
            continue;
        }
        if line.len() > MAX_LINE_BYTES {
            return Err(fail(
                codes::LINE_TOO_LONG,
                format!("A rule can be at most {MAX_LINE_BYTES} bytes."),
            ));
        }
        if rules.len() == MAX_RULES {
            return Err(fail(
                codes::TOO_MANY_RULES,
                format!("At most {MAX_RULES} rules."),
            ));
        }
        rules.push(parse_rule(line).map_err(|(code, message)| fail(code, message))?);
    }
    Ok(rules)
}

type Fail = (&'static str, String);

fn missing_arrow() -> Fail {
    (
        codes::MISSING_ARROW,
        "Rule needs a selector, \"=>\" and properties.".to_string(),
    )
}

fn parse_rule(line: &str) -> Result<Rule, Fail> {
    let at = line.rfind("=>").ok_or_else(missing_arrow)?;
    let selector = line[..at].trim();
    let props = line[at + 2..].trim();
    if selector.is_empty() || props.is_empty() {
        return Err(missing_arrow());
    }

    let mut terms = Vec::new();
    for raw in selector.split('&') {
        if terms.len() == MAX_TERMS {
            return Err((
                codes::TOO_MANY_TERMS,
                format!("A selector can combine at most {MAX_TERMS} terms with &."),
            ));
        }
        terms.push(parse_term(raw.trim())?);
    }

    let mut values = [None; DecoKind::COUNT];
    let mut any = false;
    for raw in props.split(';') {
        let prop = raw.trim();
        if prop.is_empty() {
            continue;
        }
        any = true;
        let (name, value) = match prop.split_once('=') {
            Some((n, v)) => (n.trim(), Some(v.trim())),
            None => (prop, None),
        };
        if name == "none" {
            if let Some(v) = value {
                return Err(bad_value(name, v));
            }
            values = [Some(Value::Off); DecoKind::COUNT];
            continue;
        }
        let kind = DecoKind::ALL
            .into_iter()
            .find(|k| k.name() == name)
            .ok_or_else(|| {
                (
                    codes::BAD_PROPERTY,
                    format!(
                        "Unknown property \"{}\". Known: cape, shades, hat, halo, glow, badge, none.",
                        cap_echo(name, MAX_ECHO_BYTES)
                    ),
                )
            })?;
        values[kind.index()] = Some(match value {
            None => Value::On,
            Some(v) => parse_value(v).ok_or_else(|| bad_value(name, v))?,
        });
    }
    if !any {
        return Err(missing_arrow());
    }

    let dynamic = terms.iter().any(|t| !t.negated && t.test.is_state());
    Ok(Rule {
        text: line.into(),
        terms,
        props: values,
        dynamic,
    })
}

fn bad_value(name: &str, value: &str) -> Fail {
    (
        codes::BAD_VALUE,
        format!(
            "Property \"{}\" does not accept \"{}\". Use on, off, agent or RRGGBB.",
            cap_echo(name, MAX_ECHO_BYTES),
            cap_echo(value, MAX_ECHO_BYTES)
        ),
    )
}

fn parse_term(raw: &str) -> Result<Term, Fail> {
    let (negated, body) = match raw.strip_prefix('!') {
        Some(rest) => (true, rest),
        None => (false, raw),
    };
    let test = match body {
        "human" => Test::Human,
        "agent" => Test::Agent,
        "assisted" => Test::Assisted,
        "ai" => Test::Ai,
        _ => {
            if body.starts_with("id:") {
                return Err((
                    codes::RESERVED,
                    "The selector \"id:\" is reserved for a later version.".to_string(),
                ));
            }
            match body.strip_prefix("ai:") {
                Some(id) if agents::is_valid_id(id) => Test::AiAgent(id.into()),
                _ => {
                    return Err((
                        codes::BAD_SELECTOR,
                        format!(
                            "Unknown selector \"{}\". Use human, agent, assisted, ai or ai:AGENT.",
                            cap_echo(raw, MAX_ECHO_BYTES)
                        ),
                    ));
                }
            }
        }
    };
    Ok(Term { negated, test })
}

fn parse_value(v: &str) -> Option<Value> {
    match v {
        "on" => Some(Value::On),
        "off" => Some(Value::Off),
        "agent" => Some(Value::Agent),
        _ if v.len() == 6 && v.bytes().all(|b| b.is_ascii_hexdigit()) => {
            let c = |i: usize| u8::from_str_radix(&v[i..i + 2], 16).ok();
            Some(Value::Rgb([c(0)?, c(2)?, c(4)?]))
        }
        _ => None,
    }
}

/// The compiled layers: built-in, claimed (reserved, empty), viewer.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyleLayers {
    builtin: Vec<Rule>,
    /// Contributor-claimed rules, evaluated between the built-in and the
    /// viewer layer. Reserved for the "claimed styles" spec; always empty.
    claimed: Vec<Rule>,
    viewer: Vec<Rule>,
}

impl StyleLayers {
    /// Compile `ai_style`'s built-in layer and the viewer's rule lines
    /// (already validated by the option; lines that do not parse any more
    /// are dropped).
    pub fn new(ai_style: AiStyle, viewer_lines: &[String]) -> StyleLayers {
        let builtin = parse_rules(builtin_rules(ai_style)).expect("built-in rules parse");
        let viewer = viewer_lines
            .iter()
            .filter_map(|l| parse_rules(l).ok())
            .flatten()
            .collect();
        StyleLayers {
            builtin,
            claimed: Vec::new(),
            viewer,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.builtin.is_empty() && self.claimed.is_empty() && self.viewer.is_empty()
    }

    fn rules(&self) -> impl Iterator<Item = &Rule> {
        self.builtin.iter().chain(&self.claimed).chain(&self.viewer)
    }

    /// The cascade for one contributor state.
    pub fn resolve(&self, subject: &Subject) -> ResolvedStyle {
        // per kind: value, dynamic, entry the tint comes from
        let mut won: [Option<(Value, bool, Option<SubjectEntry>)>; DecoKind::COUNT] =
            [None; DecoKind::COUNT];
        for rule in self.rules() {
            if !rule.matches(subject) {
                continue;
            }
            let entry = rule.entry(*subject).copied();
            for kind in DecoKind::ALL {
                if let Some(v) = rule.props[kind.index()] {
                    won[kind.index()] = Some((v, rule.dynamic, entry));
                }
            }
        }

        let mut out = ResolvedStyle::default();
        for kind in DecoKind::ALL {
            let Some((value, dynamic, entry)) = won[kind.index()] else {
                continue;
            };
            let tint = match value {
                Value::Off => Tint::Off,
                Value::Rgb(c) => Tint::Fixed(c),
                Value::On if kind == DecoKind::Shades => Tint::Fixed(SHADES_LENS),
                // `agent` without an AI entry falls back to the
                // contributor's colour, as `on` does
                Value::On | Value::Agent => match entry {
                    Some(e) => Tint::Fixed(e.tint),
                    None => Tint::Contributor,
                },
            };
            out.tints[kind.index()] = tint;
            if tint != Tint::Off && dynamic {
                out.dynamic |= kind.bit();
            }
            if kind == DecoKind::Badge {
                out.badge = entry.map_or(0, |e| e.badge);
            }
        }
        out
    }
}

#[cfg(test)]
mod tests;
