// SPDX-License-Identifier: GPL-3.0-or-later

//! The classifier's vectors (F1 spec section 8.1, T1 to T27).
//!
//! T1 to T4 and T6 to T16 are taken from real commits and vendor documents
//! (`docs/features/research/ai-trailers.md`); T5, T17 and T18 to T27 are
//! derived or synthetic. T25 and T26 need the git parser and live in
//! `git_meta.rs`; T12b and T17b belong to the later milestone that reads the
//! aider suffix and the Bot kind.

use gource_core::events::AttrKind::{self, Agent, Assist};
use gource_sources::classify::{AttrBuf, CommitSignals, Identity, Trailer, classify};

type Out = Vec<(AttrKind, &'static str, Option<String>)>;

fn id<'a>(name: &'a str, email: &'a str) -> Identity<'a> {
    Identity { name, email }
}

fn human() -> Identity<'static> {
    id("Jane Doe", "jane@example.com")
}

/// Classify with `author` and `Key: value` trailers.
fn run(author: Identity<'_>, trailers: &[&str]) -> (Out, usize) {
    let trailers: Vec<Trailer<'_>> = trailers
        .iter()
        .map(|t| {
            let (key, value) = t.split_once(':').unwrap();
            Trailer { key, value }
        })
        .collect();
    let signals = CommitSignals {
        author,
        committer: author,
        subject: "",
        trailers: &trailers,
    };
    let mut out = AttrBuf::new();
    classify(&signals, &mut out);
    (
        out.as_slice()
            .iter()
            .map(|a| {
                let agent: &'static str = gource_core::agents::find(a.agent).unwrap().id;
                (a.kind, agent, a.model.map(str::to_string))
            })
            .collect(),
        out.truncated(),
    )
}

fn co(trailers: &[&str]) -> Out {
    run(human(), trailers).0
}

fn e(
    kind: AttrKind,
    agent: &'static str,
    model: Option<&str>,
) -> (AttrKind, &'static str, Option<String>) {
    (kind, agent, model.map(str::to_string))
}

#[test]
fn t1_claude_trailer_with_model() {
    assert_eq!(
        co(&["Co-authored-by: Claude Opus 5.5 <noreply@anthropic.com>"]),
        [e(Assist, "claude", Some("Claude Opus 5.5"))]
    );
}

#[test]
fn t2_two_claude_models_ordered_by_model() {
    assert_eq!(
        co(&[
            "Co-Authored-By: Claude Sonnet 5 <noreply@anthropic.com>",
            "Co-Authored-By: Claude Haiku 4.5 <noreply@anthropic.com>",
        ]),
        [
            e(Assist, "claude", Some("Claude Haiku 4.5")),
            e(Assist, "claude", Some("Claude Sonnet 5")),
        ]
    );
}

#[test]
fn t3_bare_claude_and_claude_code_have_no_model() {
    assert_eq!(
        co(&["Co-authored-by: Claude <noreply@anthropic.com>"]),
        [e(Assist, "claude", None)]
    );
    assert_eq!(
        co(&["Co-authored-by: Claude Code <noreply@anthropic.com>"]),
        [e(Assist, "claude", None)]
    );
}

#[test]
fn t4_claude_as_author() {
    assert_eq!(
        run(id("Claude", "noreply@anthropic.com"), &[]).0,
        [e(Agent, "claude", None)]
    );
}

#[test]
fn t5_claude_action_bot_ignores_the_numeric_id_and_the_human_trailer() {
    // derived: the id is github-actions[bot]'s, the login is what counts
    assert_eq!(
        run(
            id(
                "claude[bot]",
                "41898282+claude[bot]@users.noreply.github.com"
            ),
            &["Co-authored-by: Jane Doe <jane@example.com>"]
        )
        .0,
        [e(Agent, "claude", None)]
    );
}

#[test]
fn t6_codex_trailer() {
    assert_eq!(
        co(&["Co-authored-by: Codex <noreply@openai.com>"]),
        [e(Assist, "codex", None)]
    );
}

#[test]
fn t7_codex_connector_author() {
    assert_eq!(
        run(
            id(
                "chatgpt-codex-connector[bot]",
                "199175422+chatgpt-codex-connector[bot]@users.noreply.github.com"
            ),
            &[]
        )
        .0,
        [e(Agent, "codex", None)]
    );
}

#[test]
fn t8_copilot_coding_agent() {
    let signals_author = id(
        "copilot-swe-agent[bot]",
        "198982749+Copilot@users.noreply.github.com",
    );
    let trailers = [Trailer {
        key: "Co-authored-by",
        value: " Jane Doe <123+jane@users.noreply.github.com>",
    }];
    let signals = CommitSignals {
        author: signals_author,
        committer: id("GitHub", "noreply@github.com"),
        subject: "Fix",
        trailers: &trailers,
    };
    let mut out = AttrBuf::new();
    classify(&signals, &mut out);
    assert_eq!(out.as_slice().len(), 1);
    assert_eq!(
        (out.as_slice()[0].kind, out.as_slice()[0].agent),
        (Agent, "copilot")
    );
    // the author name "Copilot" with the same email also matches
    assert_eq!(
        run(
            id("Copilot", "198982749+Copilot@users.noreply.github.com"),
            &[]
        )
        .0,
        [e(Agent, "copilot", None)]
    );
}

#[test]
fn t9_copilot_trailers_are_one_entry() {
    assert_eq!(
        co(&[
            "Co-authored-by: Copilot App <223556219+Copilot@users.noreply.github.com>",
            "Co-authored-by: Copilot <copilot@github.com>",
        ]),
        [e(Assist, "copilot", None)]
    );
    // VS Code once wrote the address without brackets
    assert_eq!(
        co(&["Co-authored-by: Copilot copilot@github.com"]),
        [e(Assist, "copilot", None)]
    );
}

#[test]
fn t10_cursor_trailers_are_one_entry() {
    assert_eq!(
        co(&[
            "Co-authored-by: Cursor <cursoragent@cursor.com>",
            "Made-with: Cursor",
        ]),
        [e(Assist, "cursor", None)]
    );
    assert_eq!(co(&["Made-with: Cursor"]), [e(Assist, "cursor", None)]);
}

#[test]
fn t11_cursor_agent_author() {
    assert_eq!(
        run(id("Cursor Agent", "cursoragent@cursor.com"), &[]).0,
        [e(Agent, "cursor", None)]
    );
}

#[test]
fn t12_aider_trailers_with_models_and_no_name_rewrite() {
    let (out, _) = run(
        id("Jane (aider)", "jane@example.com"),
        &[
            "Co-authored-by: aider (gpt-5.4) <aider@aider.chat>",
            "Co-authored-by: aider (gemini/gemini-3.1-pro-preview) <aider@aider.chat>",
        ],
    );
    assert_eq!(
        out,
        [
            e(Assist, "aider", Some("gemini/gemini-3.1-pro-preview")),
            e(Assist, "aider", Some("gpt-5.4")),
        ]
    );
    // the suffix and the subject prefix alone are not read before M3 (T12b)
    let signals = CommitSignals {
        author: id("Jane (aider)", "jane@example.com"),
        committer: id("Jane (aider)", "jane@example.com"),
        subject: "aider: fix",
        trailers: &[],
    };
    let mut buf = AttrBuf::new();
    classify(&signals, &mut buf);
    assert!(buf.is_empty());
}

#[test]
fn t13_jules_author() {
    assert_eq!(
        run(
            id(
                "google-labs-jules[bot]",
                "161369871+google-labs-jules[bot]@users.noreply.github.com"
            ),
            &[]
        )
        .0,
        [e(Agent, "jules", None)]
    );
}

#[test]
fn t14_devin_as_author_and_as_trailer() {
    assert_eq!(
        run(id("devin", "devin@cognition.ai"), &[]).0,
        [e(Agent, "devin", None)]
    );
    assert_eq!(
        co(&[
            "Co-Authored-By: Devin AI <158243242+devin-ai-integration[bot]@users.noreply.github.com>"
        ]),
        [e(Assist, "devin", None)]
    );
}

#[test]
fn t15_amp_and_openhands() {
    assert_eq!(
        co(&[
            "Co-authored-by: openhands <openhands@all-hands.dev>",
            "Co-authored-by: Amp <amp@ampcode.com>",
        ]),
        [e(Assist, "amp", None), e(Assist, "openhands", None)]
    );
}

#[test]
fn t16_amazon_q_author() {
    assert_eq!(
        run(
            id(
                "amazon-q-developer[bot]",
                "208079219+amazon-q-developer[bot]@users.noreply.github.com"
            ),
            &[]
        )
        .0,
        [e(Agent, "amazon-q", None)]
    );
}

#[test]
fn t17_generic_bots_are_not_ai() {
    for (name, email) in [
        (
            "dependabot[bot]",
            "49699333+dependabot[bot]@users.noreply.github.com",
        ),
        (
            "renovate[bot]",
            "29139614+renovate[bot]@users.noreply.github.com",
        ),
        (
            "github-actions[bot]",
            "41898282+github-actions[bot]@users.noreply.github.com",
        ),
    ] {
        assert!(run(id(name, email), &[]).0.is_empty(), "{name}");
    }
}

#[test]
fn t18_names_alone_and_unknown_tools_do_not_match() {
    assert!(
        run(id("Claude Smith", "claude@smith.example"), &[])
            .0
            .is_empty()
    );
    assert!(co(&["Co-authored-by: Bob <bob@old.example>"]).is_empty());
    assert!(
        run(
            id(
                "gemini-code-assist[bot]",
                "176961590+gemini-code-assist[bot]@users.noreply.github.com"
            ),
            &[]
        )
        .0
        .is_empty()
    );
    // a person whose display name is a bot login of a tool, with their own
    // address, is matched only for [bot] logins (rule 2)
    assert!(run(id("Copilot", "someone@example.com"), &[]).0.is_empty());
    assert!(co(&["Co-authored-by: Claude Opus 5.5 <claude@example.com>"]).is_empty());
}

#[test]
fn t19_squash_duplicates_collapse() {
    assert_eq!(
        co(&[
            "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>",
            "co-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
            "Co-authored-by: Claude <noreply@anthropic.com>",
        ]),
        [e(Assist, "claude", Some("Claude Opus 5.5"))]
    );
}

#[test]
fn t20_an_agent_author_absorbs_its_own_trailer() {
    assert_eq!(
        run(
            id("Claude", "noreply@anthropic.com"),
            &["Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"]
        )
        .0,
        [e(Agent, "claude", Some("Claude Opus 5.5"))]
    );
}

#[test]
fn t21_at_most_four_entries() {
    let (out, truncated) = run(
        human(),
        &[
            "Co-authored-by: Claude Opus 5.5 <noreply@anthropic.com>",
            "Co-authored-by: Codex <noreply@openai.com>",
            "Co-authored-by: Amp <amp@ampcode.com>",
            "Co-authored-by: openhands <openhands@all-hands.dev>",
            "Co-authored-by: Devin AI <158243242+devin-ai-integration[bot]@users.noreply.github.com>",
            "Co-authored-by: aider (gpt-5.4) <aider@aider.chat>",
        ],
    );
    assert_eq!(
        out.iter().map(|o| o.1).collect::<Vec<_>>(),
        ["claude", "codex", "aider", "devin"]
    );
    assert_eq!(truncated, 2);
}

#[test]
fn t22_t23_t24_free_text_never_becomes_a_model() {
    let long = format!("Claude Opus {}", "1".repeat(60));
    assert_eq!(long.len(), 72);
    for name in [
        "Claude Opus 5.5 <script>",
        long.as_str(),
        "Claude is why Bob broke prod",
        "Claude see http://evil.example",
    ] {
        assert_eq!(
            co(&[&format!("Co-authored-by: {name} <noreply@anthropic.com>")]),
            [e(Assist, "claude", None)],
            "{name}"
        );
    }
    assert_eq!(
        co(&["Co-authored-by: aider (Bob is to blame) <aider@aider.chat>"]),
        [e(Assist, "aider", None)]
    );
}

#[test]
fn t27_a_new_claude_family_needs_no_registry_edit() {
    assert_eq!(
        co(&["Co-authored-by: Claude Fable 5 <noreply@anthropic.com>"]),
        [e(Assist, "claude", Some("Claude Fable 5"))]
    );
}

#[test]
fn nothing_matches_without_allocating() {
    // A buffer that never matched has no storage.
    let mut out = AttrBuf::new();
    let trailers = [Trailer {
        key: "Signed-off-by",
        value: "Jane <jane@example.com>",
    }];
    classify(
        &CommitSignals {
            author: human(),
            committer: human(),
            subject: "x",
            trailers: &trailers,
        },
        &mut out,
    );
    assert!(out.is_empty());
}
