// SPDX-License-Identifier: GPL-3.0-or-later

//! Commit classification: which AI agents a commit is credited to.
//!
//! The classifier reads signals that already exist in git (the author
//! identity and the `Co-authored-by` and `Made-with` trailers) and returns
//! agent ids, kinds and models only; names, emails and subjects are inputs
//! and never leave it. It runs on the server (the git parser calls it for
//! each commit), never in the browser, so no email reaches a page.
//!
//! Rules (registry v1):
//!
//! - **Signals.** The author identity yields [`AttrKind::Agent`]; a
//!   `Co-authored-by` trailer or a `Made-with` trailer yields
//!   [`AttrKind::Assist`]. The committer, the `(aider)` name suffix and the
//!   `aider: ` subject prefix are later signals and are not read yet.
//! - **Matching**, case-insensitive ASCII, trimmed: an exact email of the
//!   agent's matcher; or a GitHub login, either from an email
//!   `<digits>+<login>@users.noreply.github.com` (the login is compared,
//!   never the numeric id) or, for a login ending in `[bot]`, from a name
//!   equal to the login; or a `Made-with` value. A name alone never matches
//!   (a person called "Claude" is a person).
//! - **Precedence.** When the author is agent A, trailers naming A are
//!   dropped and the Agent entry adopts the model of a dropped trailer.
//!   Human co-authors are not recorded, and generic automation (`[bot]`
//!   accounts no matcher claims) gets no attribution.
//! - **Models** are accepted only in the bounded grammar of their agent
//!   (`Claude <Family> [<version>]` or `Claude <version> <Family>`, and
//!   `aider (<litellm id>)`), so a forged trailer name cannot put free text
//!   next to a real person's name. Anything else keeps the agent and drops
//!   the model.
//! - **Output** is canonical ([`crate::canonicalize`]): unique, Agent
//!   first, in registry order, at most 4 entries.
//!
//! The matcher table is versioned with
//! [`gource_core::agents::REGISTRY_VERSION`]; a test pins its digest, so a
//! change to the table is a deliberate one.

use gource_core::agents::{self, AGENTS};
use gource_core::events::AttrKind;

use crate::{ParsedAttribution, attrs, canonicalize};

/// A name and email as git recorded them (not mailmapped).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Identity<'a> {
    pub name: &'a str,
    pub email: &'a str,
}

/// One trailer of the commit message, key and value as written.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Trailer<'a> {
    pub key: &'a str,
    pub value: &'a str,
}

/// Everything the classifier may read about one commit.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct CommitSignals<'a> {
    pub author: Identity<'a>,
    /// Read from a later milestone on; ignored by registry v1.
    pub committer: Identity<'a>,
    /// Read from a later milestone on; ignored by registry v1.
    pub subject: &'a str,
    pub trailers: &'a [Trailer<'a>],
}

/// How the model of a credit is read from a trailer's display name.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ModelGrammar {
    /// The agent has no model in its trailers.
    None,
    /// `Claude Opus 5.5`, `Claude Sonnet 5`, `Claude 3.5 Sonnet`.
    Claude,
    /// `aider (gpt-5.4)`, `aider (gemini/gemini-3.1-pro-preview)`.
    Aider,
}

/// What identifies one agent.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct AgentMatcher {
    /// Id in [`gource_core::agents::AGENTS`].
    pub id: &'static str,
    /// Exact emails (author: Agent, trailer: Assist).
    pub emails: &'static [&'static str],
    /// GitHub logins, matched in noreply emails and, for `[bot]` logins,
    /// in the name.
    pub gh_logins: &'static [&'static str],
    /// `Made-with:` values (Assist).
    pub made_with_values: &'static [&'static str],
    pub model: ModelGrammar,
}

const fn m(
    id: &'static str,
    emails: &'static [&'static str],
    gh_logins: &'static [&'static str],
    made_with_values: &'static [&'static str],
    model: ModelGrammar,
) -> AgentMatcher {
    AgentMatcher {
        id,
        emails,
        gh_logins,
        made_with_values,
        model,
    }
}

/// Registry v1 matchers, in the order of [`AGENTS`]. Every row is backed by
/// a real commit or vendor document (`docs/features/research/ai-trailers.md`).
pub const AGENT_MATCHERS: &[AgentMatcher] = &[
    m(
        "claude",
        &["noreply@anthropic.com"],
        &["claude[bot]"],
        &[],
        ModelGrammar::Claude,
    ),
    m(
        "codex",
        &["noreply@openai.com"],
        &["chatgpt-codex-connector[bot]"],
        &[],
        ModelGrammar::None,
    ),
    m(
        "copilot",
        &["copilot@github.com"],
        &["copilot-swe-agent[bot]", "Copilot"],
        &[],
        ModelGrammar::None,
    ),
    m(
        "cursor",
        &["cursoragent@cursor.com"],
        &[],
        &["Cursor"],
        ModelGrammar::None,
    ),
    m(
        "aider",
        &["aider@aider.chat"],
        &[],
        &[],
        ModelGrammar::Aider,
    ),
    m(
        "jules",
        &[],
        &["google-labs-jules[bot]"],
        &[],
        ModelGrammar::None,
    ),
    m(
        "devin",
        &["devin@cognition.ai"],
        &["devin-ai-integration[bot]"],
        &[],
        ModelGrammar::None,
    ),
    m("amp", &["amp@ampcode.com"], &[], &[], ModelGrammar::None),
    m(
        "openhands",
        &["openhands@all-hands.dev"],
        &[],
        &[],
        ModelGrammar::None,
    ),
    m(
        "amazon-q",
        &[],
        &["amazon-q-developer[bot]"],
        &[],
        ModelGrammar::None,
    ),
];

/// Longest model the classifier emits.
const MAX_MODEL: usize = 64;

/// Output of [`classify`]: at most 4 canonical entries. The storage is
/// kept between calls (see [`AttrBuf::recycle`]), so steady-state
/// classification does not allocate.
#[derive(Debug, Default)]
pub struct AttrBuf<'a> {
    entries: Vec<ParsedAttribution<'a>>,
    truncated: usize,
}

impl<'a> AttrBuf<'a> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn as_slice(&self) -> &[ParsedAttribution<'a>] {
        &self.entries
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Distinct entries the last [`classify`] dropped over the 4-entry cap.
    pub fn truncated(&self) -> usize {
        self.truncated
    }

    /// The same buffer, emptied, for another lifetime.
    pub fn recycle<'b>(self) -> AttrBuf<'b> {
        AttrBuf {
            entries: attrs::recycle(self.entries),
            truncated: 0,
        }
    }

    /// Turn every Agent entry into an Assist entry (an agent identity that a
    /// mailmap renamed into a person: the person is credited, not replaced).
    pub fn downgrade_agents(&mut self) {
        let mut changed = false;
        for e in &mut self.entries {
            if e.kind == AttrKind::Agent {
                e.kind = AttrKind::Assist;
                changed = true;
            }
        }
        if changed {
            canonicalize(&mut self.entries);
        }
    }
}

/// Classify one commit into `out` (cleared first). Pure; allocates nothing
/// when no signal matches.
pub fn classify<'a>(signals: &CommitSignals<'a>, out: &mut AttrBuf<'a>) {
    out.entries.clear();
    out.truncated = 0;

    let author = match_identity(&signals.author);
    if let Some(a) = author {
        out.entries.push(ParsedAttribution {
            kind: AttrKind::Agent,
            agent: a.id,
            model: None,
        });
    }

    for t in signals.trailers {
        let key = t.key.trim();
        let value = t.value.trim();
        let (matcher, model) = if key.eq_ignore_ascii_case("co-authored-by") {
            let identity = split_name_email(value);
            match match_identity(&identity) {
                Some(m) => (m, extract_model(m.model, identity.name)),
                None => continue,
            }
        } else if key.eq_ignore_ascii_case("made-with") {
            match AGENT_MATCHERS.iter().find(|m| {
                m.made_with_values
                    .iter()
                    .any(|v| v.eq_ignore_ascii_case(value))
            }) {
                Some(m) => (m, None),
                None => continue,
            }
        } else {
            continue;
        };

        if author.is_some_and(|a| a.id == matcher.id) {
            // Agent beats Assist: the trailer only lends its model.
            let agent = &mut out.entries[0];
            if model.is_some() && (agent.model.is_none() || model < agent.model) {
                agent.model = model;
            }
            continue;
        }
        out.entries.push(ParsedAttribution {
            kind: AttrKind::Assist,
            agent: matcher.id,
            model,
        });
    }

    out.truncated = canonicalize(&mut out.entries);
}

/// `Name <email>`; also `Name email@host` without brackets (a form VS Code
/// has written). The email is what is between the last `<` and its `>`.
fn split_name_email(value: &str) -> Identity<'_> {
    if let Some(lt) = value.rfind('<') {
        let email = value[lt + 1..].trim();
        let email = email.strip_suffix('>').unwrap_or(email).trim();
        return Identity {
            name: value[..lt].trim(),
            email,
        };
    }
    match value.rsplit_once(' ') {
        Some((name, email)) if email.contains('@') => Identity {
            name: name.trim(),
            email,
        },
        _ => Identity {
            name: value,
            email: "",
        },
    }
}

/// The matcher an identity belongs to (rules 1 and 2).
fn match_identity(id: &Identity<'_>) -> Option<&'static AgentMatcher> {
    let email = id.email.trim();
    let name = id.name.trim();
    if !email.is_empty()
        && let Some(m) = AGENT_MATCHERS
            .iter()
            .find(|m| m.emails.iter().any(|e| e.eq_ignore_ascii_case(email)))
    {
        return Some(m);
    }
    if let Some(login) = noreply_login(email)
        && let Some(m) = AGENT_MATCHERS
            .iter()
            .find(|m| m.gh_logins.iter().any(|l| l.eq_ignore_ascii_case(login)))
    {
        return Some(m);
    }
    AGENT_MATCHERS.iter().find(|m| {
        m.gh_logins
            .iter()
            .any(|l| l.ends_with("[bot]") && l.eq_ignore_ascii_case(name))
    })
}

/// The login of `<digits>+<login>@users.noreply.github.com`.
fn noreply_login(email: &str) -> Option<&str> {
    const DOMAIN: &str = "@users.noreply.github.com";
    let at = email.len().checked_sub(DOMAIN.len())?;
    if !email.is_char_boundary(at) || !email[at..].eq_ignore_ascii_case(DOMAIN) {
        return None;
    }
    let (digits, login) = email[..at].split_once('+')?;
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) && !login.is_empty())
        .then_some(login)
}

fn extract_model(grammar: ModelGrammar, name: &str) -> Option<&str> {
    let name = name.trim();
    if name.len() > MAX_MODEL || !name.is_ascii() {
        return None;
    }
    match grammar {
        ModelGrammar::None => None,
        ModelGrammar::Claude => is_claude_model(name).then_some(name),
        ModelGrammar::Aider => {
            let inner = name
                .get(..7)
                .filter(|p| p.eq_ignore_ascii_case("aider ("))
                .and_then(|_| name[7..].strip_suffix(')'))?;
            is_aider_model(inner).then_some(inner)
        }
    }
}

/// `[A-Z][a-z]{1,15}`, except `Code` (the tool's name, not a model).
fn is_family(word: &str) -> bool {
    let b = word.as_bytes();
    (2..=16).contains(&b.len())
        && b[0].is_ascii_uppercase()
        && b[1..].iter().all(u8::is_ascii_lowercase)
        && word != "Code"
}

/// `[0-9]+(\.[0-9]+)*`
fn is_version(word: &str) -> bool {
    !word.is_empty()
        && word
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
}

/// `^Claude [A-Z][a-z]{1,15}( [0-9]+(\.[0-9]+)*)?$` or
/// `^Claude [0-9]+(\.[0-9]+)* [A-Z][a-z]{1,15}$`.
fn is_claude_model(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("Claude ") else {
        return false;
    };
    let mut words = rest.split(' ');
    match (words.next(), words.next(), words.next()) {
        (Some(family), None, None) => is_family(family),
        (Some(a), Some(b), None) => {
            (is_family(a) && is_version(b)) || (is_version(a) && is_family(b))
        }
        _ => false,
    }
}

/// `^[A-Za-z0-9._-]+(/[A-Za-z0-9._:-]+)?$`
fn is_aider_model(model: &str) -> bool {
    let (provider, rest) = match model.split_once('/') {
        Some((p, r)) => (p, Some(r)),
        None => (model, None),
    };
    let ok = |s: &str, extra: &[u8]| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b) || extra.contains(&b))
    };
    ok(provider, b"") && rest.is_none_or(|r| ok(r, b":"))
}

/// Registry integrity: the digest of [`AGENTS`] and [`AGENT_MATCHERS`]
/// that the test pins. A table change fails it until this is updated; a
/// change to what is detected also bumps
/// [`gource_core::agents::REGISTRY_VERSION`] (which invalidates every
/// server cache), a cosmetic one (name, tint, badge) does not.
pub const REGISTRY_DIGEST: u64 = 0xa693_5d28_ca3e_f6c9;

/// The registry version this digest belongs to.
pub const EXPECTED_REGISTRY_VERSION: u32 = 1;

/// FNV-1a over a serialisation of both tables.
pub fn registry_digest() -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |bytes: &[u8]| {
        for &b in bytes.iter().chain([0xffu8].iter()) {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for a in AGENTS {
        feed(a.id.as_bytes());
        feed(a.name.as_bytes());
        feed(&a.tint);
        feed(&[a.badge as u8]);
    }
    for m in AGENT_MATCHERS {
        feed(m.id.as_bytes());
        for list in [m.emails, m.gh_logins, m.made_with_values] {
            for s in list {
                feed(s.as_bytes());
            }
            feed(b"");
        }
        feed(&[m.model as u8]);
    }
    h
}

/// Whether `id` is a registry agent the classifier can emit.
pub fn is_registry_agent(id: &str) -> bool {
    agents::find(id).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_digest_is_pinned() {
        assert_eq!(
            gource_core::agents::REGISTRY_VERSION,
            EXPECTED_REGISTRY_VERSION,
            "the registry version changed: update EXPECTED_REGISTRY_VERSION and REGISTRY_DIGEST"
        );
        assert_eq!(
            registry_digest(),
            REGISTRY_DIGEST,
            "AGENTS or AGENT_MATCHERS changed: bump REGISTRY_VERSION if detection changed, \
             then update REGISTRY_DIGEST to {:#018x}",
            registry_digest()
        );
    }

    #[test]
    fn every_matcher_names_a_registry_agent() {
        for m in AGENT_MATCHERS {
            assert!(is_registry_agent(m.id), "{}", m.id);
        }
        let ids: Vec<_> = AGENT_MATCHERS.iter().map(|m| m.id).collect();
        let table: Vec<_> = AGENTS.iter().map(|a| a.id).collect();
        assert_eq!(ids, table, "one matcher per agent, in registry order");
    }

    #[test]
    fn model_grammars() {
        for ok in [
            "Claude Opus 5.5",
            "Claude Haiku 4.5",
            "Claude Sonnet 5",
            "Claude Fable 5",
            "Claude Opus",
            "Claude 3.5 Sonnet",
        ] {
            assert!(is_claude_model(ok), "{ok}");
        }
        for bad in [
            "Claude",
            "Claude Code",
            "Claude Code 2",
            "Claude is why Bob broke prod",
            "Claude see http://evil.example",
            "Claude Opus 5.5 <script>",
            "claude opus 5",
            "Claude O 5",
            "Claude Opus 5.",
            "Claude Opus .5",
            "Claude  Opus 5",
            "Claude Opus 5 5",
        ] {
            assert!(!is_claude_model(bad), "{bad}");
        }
        for ok in [
            "gpt-5.4",
            "gemini/gemini-3.1-pro-preview",
            "ollama/llama3:8b",
        ] {
            assert!(is_aider_model(ok), "{ok}");
        }
        for bad in ["", "Bob is to blame", "a/b/c", "a:b", "/x", "x/", "a@b"] {
            assert!(!is_aider_model(bad), "{bad}");
        }
        assert_eq!(
            noreply_login("1+claude[bot]@users.noreply.github.com"),
            Some("claude[bot]")
        );
        assert_eq!(noreply_login("+x@users.noreply.github.com"), None);
        assert_eq!(noreply_login("a1+x@users.noreply.github.com"), None);
        assert_eq!(noreply_login("x@users.noreply.github.com"), None);
        assert_eq!(noreply_login("1+x@example.com"), None);
    }
}
