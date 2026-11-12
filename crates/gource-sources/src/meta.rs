// SPDX-License-Identifier: GPL-3.0-or-later

//! The custom log's optional meta field: `key=value` pairs after an
//! explicit colour slot.
//!
//! ```text
//! timestamp|username|A/M/D|/path|<colour or empty>|<meta>
//! 1790942378|jane|A|/src/a.rs||ai=co:claude:Claude Opus 5.5
//! ```
//!
//! C++ Gource stops reading after the colour slot, so it reads these lines
//! unchanged. Grammar (unknown keys are ignored, so later features can add
//! keys without a format change):
//!
//! ```text
//! meta   = [ pair *( ";" pair ) ]
//! pair   = key "=" value
//! key    = lower *( lower / DIGIT / "-" )          ; 1..24 bytes
//! value  = *( vchar / pct )                        ; every key except `ai`
//! vchar  = any UTF-8 scalar except C0 and C1 controls, DEL, bidi controls
//!          (U+061C, U+200E, U+200F, U+202A-202E, U+2066-2069), "%", ";", "|", "="
//! ai     = credit *( "," credit )
//! credit = kind ":" comp [ ":" comp ]              ; kind, agent, model
//! comp   = *( cchar / pct )                        ; cchar = vchar except ":" and ","
//! pct    = "%" HEXDIG HEXDIG                       ; UTF-8 bytes, either hex case
//! ```
//!
//! Keys: `ai` (credits, `co` = assisted, `au` = agent author), `ev` (what
//! the line stands for when it is not a commit's change: `pr-opened`,
//! `pr-merged`, `pr-closed`; [`ChangeEvent`]; an unknown value, from a
//! newer writer, reads as a commit change), `bot` (reserved for generic
//! automation, read and validated but not used yet), `cid` (reserved for
//! claimed styles), `x-*` (private, ignored unread).
//!
//! Limits: the whole field at most [`MAX_META_BYTES`] bytes, at most
//! [`MAX_PAIRS`] pairs and [`MAX_CREDITS`] credits per `ai` (more are not
//! read), agent ids `[a-z0-9][a-z0-9-]{0,31}`, models at most
//! [`MAX_MODEL_BYTES`] bytes of `[A-Za-z0-9 ._:/+,-]` after decoding.
//! Leading and trailing spaces of values are trimmed.
//!
//! Errors never cost the line: a field over the size limit is ignored as a
//! whole, a malformed pair (no `=`, bad key, bad escape, a control or bidi
//! character, a bad credit) is ignored on its own. Each counts once in
//! [`crate::ParseStats::bad_meta`]. An unknown key is ignored silently, and
//! of two pairs with the same key the first wins.
//!
//! Rich PR records additionally require `pr`, `pr-author`, `pr-created`,
//! `pr-state=open|draft` and `pr-initial=0|1`; requested-review events require
//! `pr-target`. Their entire encoded field must fit 512 bytes/eight pairs.
//! Missing/conflicting mandatory data rejects the rich record, not the ordinary
//! code records around it. Unknown `ev` and free L0 ev-only records retain their
//! older fallback. An optional malformed `pr-log` reduces frozen coverage to
//! sticky Unknown without discarding valid mandatory lifecycle data.

use std::fmt::Write as _;

use gource_core::agents;
use gource_core::events::{
    AttrKind, ChangeEvent, ChangeKind, PrCompleteness, PrLogMetadata, PrLogUpdate, PrState,
    is_pr_label,
};

use crate::attrs::OwnedAttrs;
use crate::{ParsedAttribution, ParsedPrLifecycle};

/// Longest meta field, in bytes as written (escaped).
pub const MAX_META_BYTES: usize = 512;
/// Pairs read from one field.
pub const MAX_PAIRS: usize = 8;
/// Longest key.
pub const MAX_KEY_BYTES: usize = 24;
/// Credits read from one `ai` value.
pub const MAX_CREDITS: usize = 4;
/// Longest model, decoded.
pub const MAX_MODEL_BYTES: usize = 64;

/// A credit read from a meta field, owned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Credit {
    pub kind: AttrKind,
    pub agent: String,
    pub model: Option<String>,
}

/// Read a meta field (the bytes after the colour slot's `|`). Returns the
/// credits of its `ai` pair, in field order and not yet canonical, and the
/// number of problems found (each one a `bad_meta`).
pub fn parse(field: &[u8]) -> (Vec<Credit>, u64) {
    let mut reader = MetaReader::default();
    let mut owned = OwnedAttrs::default();
    let bad = reader.read(field, &mut owned);
    let mut views = Vec::new();
    owned.extend_into(&mut views);
    let credits = views
        .iter()
        .map(|a| Credit {
            kind: a.kind,
            agent: a.agent.to_string(),
            model: a.model.map(str::to_string),
        })
        .collect();
    (credits, bad)
}

/// The `ev` value of a meta field ([`ChangeEvent::Commit`] without one).
pub fn parse_event(field: &[u8]) -> ChangeEvent {
    let mut reader = MetaReader::default();
    reader.read(field, &mut OwnedAttrs::default());
    reader.event()
}

/// Reusable state for reading meta fields without allocating per line.
#[derive(Debug, Default)]
pub(crate) struct MetaReader {
    /// Decoded components of the `ai` pair being read.
    decoded: String,
    /// Scratch for percent-decoding one component.
    bytes: Vec<u8>,
    /// The `ev` value of the field read last.
    event: ChangeEvent,
    pr: Option<OwnedPrLifecycle>,
    rejected_pr: bool,
    coverage: Option<PrLogUpdate>,
}

#[derive(Clone, Debug)]
pub(crate) struct OwnedPrLifecycle {
    pub number: u64,
    pub owner: String,
    pub target: Option<String>,
    pub created: i64,
    pub state: PrState,
    pub initial: bool,
}

impl OwnedPrLifecycle {
    pub(crate) fn borrowed(&self) -> ParsedPrLifecycle<'_> {
        ParsedPrLifecycle {
            number: self.number,
            owner: &self.owner,
            target: self.target.as_deref(),
            created: self.created,
            state: self.state,
            initial: self.initial,
        }
    }
}

/// One credit of the `ai` pair being read: spans into `MetaReader::decoded`.
#[derive(Copy, Clone)]
struct Span {
    kind: AttrKind,
    agent: (usize, usize),
    model: Option<(usize, usize)>,
}

const NO_SPAN: Span = Span {
    kind: AttrKind::Assist,
    agent: (0, 0),
    model: None,
};

impl MetaReader {
    /// Read `field`, append the credits of its `ai` pair to `out`, and
    /// return the number of problems.
    pub(crate) fn read(&mut self, field: &[u8], out: &mut OwnedAttrs) -> u64 {
        self.read_line(field, out, None)
    }

    pub(crate) fn read_line(
        &mut self,
        field: &[u8],
        out: &mut OwnedAttrs,
        kind: Option<ChangeKind>,
    ) -> u64 {
        self.event = ChangeEvent::Commit;
        self.pr = None;
        self.rejected_pr = false;
        self.coverage = None;
        if field.is_empty() {
            return 0;
        }
        let mut pr_bad = self.read_pr(field);
        if self.pr.is_some() && kind.is_some_and(|kind| self.event.pr_kind() != Some(kind)) {
            self.rejected_pr = true;
            self.coverage = Some(PrLogUpdate::Unknown);
            pr_bad += 1;
        }
        if self.rejected_pr {
            self.event = ChangeEvent::Commit;
            return pr_bad.max(1);
        }
        if field.len() > MAX_META_BYTES {
            return 1;
        }

        self.decoded.clear();
        let mut bad = pr_bad;
        let mut seen: [&[u8]; MAX_PAIRS] = [&[]; MAX_PAIRS];
        let mut seen_count = 0;
        for pair in field.split(|&b| b == b';').take(MAX_PAIRS) {
            let Some(eq) = pair.iter().position(|&b| b == b'=') else {
                bad += 1;
                continue;
            };
            let (key, value) = (&pair[..eq], &pair[eq + 1..]);
            if !is_key(key) {
                bad += 1;
                continue;
            }
            if key.starts_with(b"x-") {
                continue;
            }
            if seen[..seen_count].contains(&key) {
                continue;
            }
            seen[seen_count] = key;
            seen_count += 1;
            if key == b"pr-log" && pr_bad != 0 {
                // Rich preflight already counted this optional carrier problem.
                continue;
            }

            let ok = match key {
                b"ai" => self.read_ai(value, out),
                b"ev" => self.decode(value, false).is_some_and(|(s, e)| {
                    if let Some(ev) = ChangeEvent::from_meta(&self.decoded[s..e]) {
                        self.event = ev;
                    }
                    true
                }),
                b"bot" => self.decode(value, false).is_some_and(|(s, e)| {
                    // Reserved for generic automation (a later milestone):
                    // validated, not used.
                    agents::is_valid_id(&self.decoded[s..e])
                }),
                _ => self.decode(value, false).is_some(),
            };
            if !ok {
                bad += 1;
            }
        }
        bad
    }

    /// The `ev` value of the field read last.
    pub(crate) fn event(&self) -> ChangeEvent {
        self.event
    }

    pub(crate) fn take_pr(&mut self) -> Option<OwnedPrLifecycle> {
        self.pr.take()
    }

    pub(crate) fn rejected_pr(&self) -> bool {
        self.rejected_pr
    }

    pub(crate) fn coverage(&self) -> Option<PrLogUpdate> {
        self.coverage
    }

    /// A rich rejection is not an ordinary file mutation. Old ev-only L0 and
    /// unknown ev retain their original fallback semantics separately.
    fn read_pr(&mut self, field: &[u8]) -> u64 {
        let mut event = None;
        let mut seen_event = false;
        let mut rich_keys = false;
        for pair in field.split(|&b| b == b';') {
            let Some(eq) = pair.iter().position(|&b| b == b'=') else {
                continue;
            };
            let key = &pair[..eq];
            if matches!(
                key,
                b"pr"
                    | b"pr-author"
                    | b"pr-target"
                    | b"pr-created"
                    | b"pr-state"
                    | b"pr-initial"
                    | b"pr-log"
            ) {
                rich_keys = true;
            }
            if key == b"ev" && !seen_event && pair.len() <= MAX_META_BYTES {
                seen_event = true;
                self.decoded.clear();
                if let Some((s, e)) = self.decode(&pair[eq + 1..], false) {
                    event = ChangeEvent::from_meta(&self.decoded[s..e]);
                }
            }
        }
        let recognized_pr = event.is_some_and(|ev| ev != ChangeEvent::Commit);
        let rich_event = event.is_some_and(|ev| ev.code() > 3);
        if !recognized_pr || (!rich_keys && !rich_event) {
            // An optional bare commit pr never establishes repository scope.
            if rich_keys
                && field
                    .split(|&b| b == b';')
                    .any(|p| p.starts_with(b"pr-log="))
            {
                self.coverage = Some(PrLogUpdate::Unknown);
            }
            return 0;
        }
        let result = (|| {
            if field.len() > MAX_META_BYTES || field.split(|&b| b == b';').count() > MAX_PAIRS {
                return None;
            }
            let mut values = std::collections::HashMap::new();
            let mut bad_carrier = false;
            for pair in field.split(|&b| b == b';') {
                let eq = pair.iter().position(|&b| b == b'=')?;
                let key = &pair[..eq];
                if !is_key(key) {
                    return None;
                }
                if key.starts_with(b"x-") || key == b"ai" {
                    continue;
                }
                self.decoded.clear();
                let decoded = self.decode(&pair[eq + 1..], false);
                let Some((s, e)) = decoded else {
                    if key == b"pr-log" {
                        bad_carrier = true;
                        continue;
                    }
                    return None;
                };
                let value = self.decoded[s..e].to_owned();
                if values.insert(key, value).is_some() {
                    if key == b"pr-log" {
                        bad_carrier = true;
                    } else {
                        return None;
                    }
                }
            }
            let get = |key: &[u8]| values.get(key).map(String::as_str);
            let number = strict_u64(get(b"pr")?)?;
            let owner = get(b"pr-author")?;
            if number == 0 || !is_pr_label(owner) {
                return None;
            }
            let target = get(b"pr-target");
            if target.is_some_and(|t| !is_pr_label(t)) {
                return None;
            }
            let created = strict_i64(get(b"pr-created")?)?;
            let state = match get(b"pr-state")? {
                "open" => PrState::Open,
                "draft" => PrState::Draft,
                _ => return None,
            };
            let initial = match get(b"pr-initial")? {
                "0" => false,
                "1" => true,
                _ => return None,
            };
            let ev = event?;
            if (ev == ChangeEvent::PrReviewRequested) != target.is_some()
                || (initial && ev != ChangeEvent::PrOpened)
            {
                return None;
            }
            let coverage = if bad_carrier {
                Some(PrLogUpdate::Unknown)
            } else {
                get(b"pr-log").map(|value| {
                    if ev != ChangeEvent::PrOpened {
                        PrLogUpdate::Unknown
                    } else {
                        parse_pr_log(value).map_or(PrLogUpdate::Unknown, PrLogUpdate::Metadata)
                    }
                })
            };
            Some((
                OwnedPrLifecycle {
                    number,
                    owner: owner.to_owned(),
                    target: target.map(str::to_owned),
                    created,
                    state,
                    initial,
                },
                coverage,
            ))
        })();
        match result {
            Some((pr, coverage)) => {
                self.event = event.unwrap();
                self.pr = Some(pr);
                self.coverage = coverage;
                u64::from(coverage == Some(PrLogUpdate::Unknown))
            }
            None => {
                self.rejected_pr = true;
                self.coverage = Some(PrLogUpdate::Unknown);
                1
            }
        }
    }

    /// The `ai` value: all of it or nothing.
    fn read_ai(&mut self, value: &[u8], out: &mut OwnedAttrs) -> bool {
        self.decoded.clear();
        let mut spans = [NO_SPAN; MAX_CREDITS];
        let mut n = 0;
        for credit in value.split(|&b| b == b',').take(MAX_CREDITS) {
            let mut comps = credit.split(|&b| b == b':');
            let (Some(kind), Some(agent)) = (comps.next(), comps.next()) else {
                return false;
            };
            let model = comps.next();
            if comps.next().is_some() {
                return false;
            }

            let Some(k) = self.decode(kind, true) else {
                return false;
            };
            let kind = match &self.decoded[k.0..k.1] {
                "co" => AttrKind::Assist,
                "au" => AttrKind::Agent,
                _ => return false,
            };
            let Some(agent) = self.decode(agent, true) else {
                return false;
            };
            if !agents::is_valid_id(&self.decoded[agent.0..agent.1]) {
                return false;
            }
            let model = match model {
                None => None,
                Some(raw) => {
                    let Some(m) = self.decode(raw, true) else {
                        return false;
                    };
                    if m.0 == m.1 {
                        None
                    } else if is_model(&self.decoded[m.0..m.1]) {
                        Some(m)
                    } else {
                        return false;
                    }
                }
            };
            spans[n] = Span { kind, agent, model };
            n += 1;
        }

        for s in &spans[..n] {
            let agent = &self.decoded[s.agent.0..s.agent.1];
            let model = s.model.map(|(a, b)| &self.decoded[a..b]);
            out.push(s.kind, agent, model);
        }
        true
    }

    /// Percent-decode and validate one value or `ai` component, trim its
    /// spaces and append it to `self.decoded`; returns its span there.
    fn decode(&mut self, raw: &[u8], ai_component: bool) -> Option<(usize, usize)> {
        let text = std::str::from_utf8(raw).ok()?;
        self.bytes.clear();
        let mut chars = text.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '%' => {
                    let hex = raw.get(i + 1..i + 3)?;
                    let hi = hex_value(hex[0])?;
                    let lo = hex_value(hex[1])?;
                    self.bytes.push(hi << 4 | lo);
                    chars.next();
                    chars.next();
                }
                ';' | '|' | '=' => return None,
                ':' | ',' if ai_component => return None,
                c if is_forbidden(c) => return None,
                c => {
                    let mut b = [0; 4];
                    self.bytes
                        .extend_from_slice(c.encode_utf8(&mut b).as_bytes());
                }
            }
        }
        let decoded = std::str::from_utf8(&self.bytes).ok()?;
        if decoded.chars().any(is_forbidden) {
            return None;
        }
        let trimmed = decoded.trim_matches(' ');
        let start = self.decoded.len();
        self.decoded.push_str(trimmed);
        Some((start, self.decoded.len()))
    }
}

fn hex_value(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// C0 and C1 controls, DEL and the bidi controls.
fn is_forbidden(c: char) -> bool {
    matches!(c,
        '\u{0}'..='\u{1f}'
        | '\u{7f}'..='\u{9f}'
        | '\u{61c}'
        | '\u{200e}'
        | '\u{200f}'
        | '\u{202a}'..='\u{202e}'
        | '\u{2066}'..='\u{2069}')
}

/// `lower *( lower / DIGIT / "-" )`, 1 to 24 bytes.
fn is_key(key: &[u8]) -> bool {
    !key.is_empty()
        && key.len() <= MAX_KEY_BYTES
        && key[0].is_ascii_lowercase()
        && key
            .iter()
            .all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// At most 64 bytes of `[A-Za-z0-9 ._:/+,-]`.
pub fn is_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= MAX_MODEL_BYTES
        && model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b" ._:/+,-".contains(&b))
}

/// Append `s` percent-encoded for a meta value: `%`, `;`, `|`, `=`, C0 and
/// C1 controls and DEL always, and `:` and `,` inside an `ai` component.
pub(crate) fn escape_into(out: &mut String, s: &str, ai_component: bool) {
    for c in s.chars() {
        let escape = matches!(c, '%' | ';' | '|' | '=')
            || (ai_component && matches!(c, ':' | ','))
            || matches!(c, '\u{0}'..='\u{1f}' | '\u{7f}'..='\u{9f}');
        if escape {
            let mut b = [0; 4];
            for byte in c.encode_utf8(&mut b).bytes() {
                let _ = write!(out, "%{byte:02X}");
            }
        } else {
            out.push(c);
        }
    }
}

fn strict_u64(value: &str) -> Option<u64> {
    (!value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

fn strict_i64(value: &str) -> Option<i64> {
    let digits = value.strip_prefix('-').unwrap_or(value);
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

pub(crate) fn parse_pr_log(value: &str) -> Option<PrLogMetadata> {
    let mut parts = value.split(',');
    if parts.next()? != "1" {
        return None;
    }
    let from = strict_i64(parts.next()?)?;
    let to = strict_i64(parts.next()?)?;
    let as_of = strict_i64(parts.next()?)?;
    let flag = |v| match v {
        "c" => Some(PrCompleteness::Complete),
        "p" => Some(PrCompleteness::Partial),
        "u" => Some(PrCompleteness::Unknown),
        _ => None,
    };
    let lifecycle = flag(parts.next()?)?;
    let reviews = flag(parts.next()?)?;
    let files = flag(parts.next()?)?;
    let meta = PrLogMetadata {
        version: 1,
        from,
        to,
        as_of,
        lifecycle,
        reviews,
        files,
    };
    (parts.next().is_none() && meta.is_valid()).then_some(meta)
}

pub(crate) fn write_pr_log(out: &mut String, metadata: PrLogMetadata) {
    let flag = |v| match v {
        PrCompleteness::Complete => "c",
        PrCompleteness::Partial => "p",
        PrCompleteness::Unknown => "u",
    };
    let _ = write!(
        out,
        "1,{},{},{},{},{},{}",
        metadata.from,
        metadata.to,
        metadata.as_of,
        flag(metadata.lifecycle),
        flag(metadata.reviews),
        flag(metadata.files)
    );
}

/// Write the meta field for `credits` (canonical order) into `out`, which
/// is cleared first: `ai=co:claude:Claude Opus 5.5,au:cursor`. Credits a
/// reader would reject (bad agent id or model) and Bot credits are left
/// out, and trailing credits are dropped until the field fits
/// [`MAX_META_BYTES`]. Returns the number of credits not written; `out`
/// stays empty when none is left.
pub fn write_ai_field(out: &mut String, credits: &[ParsedAttribution<'_>]) -> usize {
    out.clear();
    let mut written = 0;
    for c in credits.iter().take(MAX_CREDITS) {
        let agent = c.agent.trim_matches(' ');
        let model = c
            .model
            .map(|m| m.trim_matches(' '))
            .filter(|m| !m.is_empty());
        let kind = match c.kind {
            AttrKind::Assist => "co",
            AttrKind::Agent => "au",
            // Generic automation has no `ai` form; its `bot` key is
            // reserved for a later milestone.
            AttrKind::Bot => continue,
        };
        if !agents::is_valid_id(agent) || model.is_some_and(|m| !is_model(m)) {
            continue;
        }
        let before = out.len();
        out.push_str(if written == 0 { "ai=" } else { "," });
        out.push_str(kind);
        out.push(':');
        escape_into(out, agent, true);
        if let Some(m) = model {
            out.push(':');
            escape_into(out, m, true);
        }
        if out.len() > MAX_META_BYTES {
            // Canonical order puts the most important credits first; the
            // later ones are the ones to give up.
            out.truncate(before);
            break;
        }
        written += 1;
    }
    credits.len() - written
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(field: &str) -> (Vec<(AttrKind, String, Option<String>)>, u64) {
        let (credits, bad) = parse(field.as_bytes());
        (
            credits
                .into_iter()
                .map(|c| (c.kind, c.agent, c.model))
                .collect(),
            bad,
        )
    }

    #[test]
    fn hex_and_keys() {
        assert!(is_key(b"ai") && is_key(b"x-a") && is_key(b"a1-"));
        assert!(!is_key(b"") && !is_key(b"1a") && !is_key(b"Ai") && !is_key(&[b'a'; 25]));
        assert!(is_key(&[b'a'; 24]));
        assert_eq!(read("ai=co:claude:a%3ab").0[0].2.as_deref(), Some("a:b"));
    }
}
