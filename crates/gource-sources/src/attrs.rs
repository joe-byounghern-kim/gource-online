// SPDX-License-Identifier: GPL-3.0-or-later

//! Attribution helpers shared by the parsers: the canonical order of a
//! commit's credits, owned storage for credits collected over several
//! lines, and the cap on distinct sets.

use std::cmp::Ordering;
use std::collections::HashSet;

use gource_core::agents;
use gource_core::events::{AttrKind, MAX_ATTR_SETS, MAX_SET_ENTRIES};

use crate::ParsedAttribution;

fn kind_rank(kind: AttrKind) -> u8 {
    match kind {
        AttrKind::Agent => 0,
        AttrKind::Assist => 1,
        AttrKind::Bot => 2,
    }
}

/// `(kind: Agent first, registry index, model)`; agents that are not in the
/// registry come after the known ones, ordered by id. A missing model sorts
/// before any model.
fn canonical_cmp(a: &ParsedAttribution<'_>, b: &ParsedAttribution<'_>) -> Ordering {
    let index = |id: &str| agents::agent_index(id).unwrap_or(agents::AGENTS.len());
    kind_rank(a.kind)
        .cmp(&kind_rank(b.kind))
        .then_with(|| index(a.agent).cmp(&index(b.agent)))
        .then_with(|| a.agent.cmp(b.agent))
        .then_with(|| a.model.cmp(&b.model))
}

/// Bring a commit's credits into the canonical form that is part of the
/// wire format: unique by `(kind, agent, model)`, an entry without a model
/// dropped when the same agent and kind has one with a model, ordered by
/// `(kind: Agent first, registry index, model)`, at most 4 entries.
/// Returns how many distinct entries were dropped by the 4-entry cap.
pub fn canonicalize(v: &mut Vec<ParsedAttribution<'_>>) -> usize {
    if v.len() > 1 {
        v.sort_unstable_by(canonical_cmp);
        v.dedup();
        // Bare entries sort first within their (kind, agent): drop one when
        // the next entry is the same agent and kind with a model.
        let mut i = 0;
        while i + 1 < v.len() {
            let (a, b) = (v[i], v[i + 1]);
            if a.model.is_none() && a.kind == b.kind && a.agent == b.agent {
                v.remove(i);
            } else {
                i += 1;
            }
        }
    }
    let extra = v.len().saturating_sub(MAX_SET_ENTRIES);
    v.truncate(MAX_SET_ENTRIES);
    extra
}

/// Credits of a commit in progress, owned, because the lines they came from
/// are gone by the time the commit is emitted. Buffers are reused.
#[derive(Debug, Default)]
pub(crate) struct OwnedAttrs {
    text: String,
    /// `(kind, agent end, model end or None)`; spans are back to back in
    /// `text`, each agent followed by its model.
    entries: Vec<(AttrKind, usize, Option<usize>)>,
}

impl OwnedAttrs {
    pub(crate) fn clear(&mut self) {
        self.text.clear();
        self.entries.clear();
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(crate) fn push(&mut self, kind: AttrKind, agent: &str, model: Option<&str>) {
        self.text.push_str(agent);
        let agent_end = self.text.len();
        let model_end = model.map(|m| {
            self.text.push_str(m);
            self.text.len()
        });
        self.entries.push((kind, agent_end, model_end));
    }

    /// Append borrowed views of every entry to `out`.
    pub(crate) fn extend_into<'a>(&'a self, out: &mut Vec<ParsedAttribution<'a>>) {
        let mut start = 0;
        for &(kind, agent_end, model_end) in &self.entries {
            let agent = &self.text[start..agent_end];
            let model = model_end.map(|end| &self.text[agent_end..end]);
            start = model_end.unwrap_or(agent_end);
            out.push(ParsedAttribution { kind, agent, model });
        }
    }
}

/// The distinct canonical sets a parser has handed out, up to
/// [`MAX_ATTR_SETS`].
#[derive(Debug, Default)]
pub(crate) struct SetCap {
    seen: HashSet<Box<str>>,
    key: String,
}

impl SetCap {
    /// Whether a commit may carry `set` (canonical): always for an empty or
    /// already seen set, and for a new one while there is room.
    pub(crate) fn admit(&mut self, set: &[ParsedAttribution<'_>]) -> bool {
        if set.is_empty() {
            return true;
        }
        // Agent ids and models never contain control characters.
        self.key.clear();
        for a in set {
            self.key.push(char::from(b'0' + a.kind.code()));
            self.key.push_str(a.agent);
            match a.model {
                Some(m) => {
                    self.key.push('\x1f');
                    self.key.push_str(m);
                }
                None => self.key.push('\x1d'),
            }
            self.key.push('\x1e');
        }
        if self.seen.contains(self.key.as_str()) {
            return true;
        }
        if self.seen.len() >= MAX_ATTR_SETS {
            return false;
        }
        self.seen.insert(self.key.as_str().into());
        true
    }
}

/// Reuse a vector's allocation for a different lifetime (the in-place
/// `collect` specialisation keeps the buffer; the vector is empty).
pub(crate) fn recycle<'b>(mut v: Vec<ParsedAttribution<'_>>) -> Vec<ParsedAttribution<'b>> {
    v.clear();
    v.into_iter().map_while(|_| None).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a<'a>(kind: AttrKind, agent: &'a str, model: Option<&'a str>) -> ParsedAttribution<'a> {
        ParsedAttribution { kind, agent, model }
    }

    #[test]
    fn canonical_order_dedupe_and_cap() {
        use AttrKind::*;
        let mut v = vec![
            a(Assist, "zz-unknown", None),
            a(Assist, "codex", None),
            a(Assist, "claude", Some("Claude Sonnet 5")),
            a(Assist, "claude", None),
            a(Agent, "cursor", None),
            a(Assist, "claude", Some("Claude Haiku 4.5")),
            a(Assist, "claude", Some("Claude Sonnet 5")),
        ];
        assert_eq!(canonicalize(&mut v), 1);
        assert_eq!(
            v,
            [
                a(Agent, "cursor", None),
                a(Assist, "claude", Some("Claude Haiku 4.5")),
                a(Assist, "claude", Some("Claude Sonnet 5")),
                a(Assist, "codex", None),
            ]
        );
        // a bare entry stays when the same agent has a model under another kind
        let mut v = vec![a(Assist, "claude", Some("m")), a(Agent, "claude", None)];
        assert_eq!(canonicalize(&mut v), 0);
        assert_eq!(
            v,
            [a(Agent, "claude", None), a(Assist, "claude", Some("m"))]
        );
        // unknown agents after known ones, by id
        let mut v = vec![
            a(Assist, "b", None),
            a(Assist, "a", None),
            a(Assist, "amp", None),
        ];
        canonicalize(&mut v);
        assert_eq!(
            v.iter().map(|e| e.agent).collect::<Vec<_>>(),
            ["amp", "a", "b"]
        );
    }

    #[test]
    fn owned_attrs_round_trip() {
        let mut o = OwnedAttrs::default();
        o.push(AttrKind::Assist, "claude", Some("Claude Opus 5.5"));
        o.push(AttrKind::Agent, "cursor", None);
        o.push(AttrKind::Assist, "aider", Some("gpt-5.4"));
        let mut out = Vec::new();
        o.extend_into(&mut out);
        assert_eq!(
            out,
            [
                a(AttrKind::Assist, "claude", Some("Claude Opus 5.5")),
                a(AttrKind::Agent, "cursor", None),
                a(AttrKind::Assist, "aider", Some("gpt-5.4")),
            ]
        );
    }

    #[test]
    fn set_cap_admits_seen_sets_after_the_cap() {
        let mut cap = SetCap::default();
        let models: Vec<String> = (0..MAX_ATTR_SETS + 1).map(|i| i.to_string()).collect();
        for m in &models[..MAX_ATTR_SETS] {
            assert!(cap.admit(&[a(AttrKind::Assist, "x", Some(m))]));
        }
        assert!(!cap.admit(&[a(AttrKind::Assist, "x", Some(&models[MAX_ATTR_SETS]))]));
        assert!(cap.admit(&[a(AttrKind::Assist, "x", Some(&models[7]))]));
        assert!(cap.admit(&[]));
    }
}
