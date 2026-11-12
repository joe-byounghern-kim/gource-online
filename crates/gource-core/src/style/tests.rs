// SPDX-License-Identifier: GPL-3.0-or-later

//! Grammar, error and cascade tests of the style rules.

use super::*;

fn err(text: &str) -> StyleError {
    parse_rules(text).expect_err(text)
}

const CLAUDE: [u8; 3] = [0xE6, 0x9F, 0x00];
const CODEX: [u8; 3] = [0x56, 0xB4, 0xE9];
const CURSOR: [u8; 3] = [0xCC, 0x79, 0xA7];

fn entry(kind: AttrKind, agent: &str) -> SubjectEntry<'_> {
    let a = agents::resolve(agent);
    SubjectEntry {
        kind,
        agent,
        tint: a.tint,
        badge: a.badge as u8,
    }
}

fn resolve(layers: &StyleLayers, entries: &[SubjectEntry]) -> ResolvedStyle {
    layers.resolve(&Subject { entries })
}

fn viewer(text: &str) -> StyleLayers {
    let lines: Vec<String> = parse_rules(text)
        .unwrap()
        .iter()
        .map(|r| r.text().to_string())
        .collect();
    StyleLayers::new(AiStyle::None, &lines)
}

#[test]
fn builtin_layers_parse() {
    for style in [
        AiStyle::Default,
        AiStyle::Cape,
        AiStyle::Shades,
        AiStyle::Hat,
        AiStyle::None,
    ] {
        let rules = parse_rules(builtin_rules(style)).unwrap();
        assert!(rules.iter().all(Rule::is_dynamic), "{style:?}");
        assert_eq!(rules.len(), if style == AiStyle::None { 0 } else { 3 });
    }
    assert!(StyleLayers::new(AiStyle::None, &[]).is_empty());
}

#[test]
fn every_error_code_with_its_line() {
    let cases: &[(&str, &str, u32)] = &[
        ("ai badge", codes::MISSING_ARROW, 1),
        ("# c\n\n => badge", codes::MISSING_ARROW, 3),
        ("ai =>", codes::MISSING_ARROW, 1),
        ("ai => ;;", codes::MISSING_ARROW, 1),
        ("ai => badge\nrobot => cape", codes::BAD_SELECTOR, 2),
        ("bot => cape", codes::BAD_SELECTOR, 1),
        ("habitual => cape", codes::BAD_SELECTOR, 1),
        ("name:/x/ => cape", codes::BAD_SELECTOR, 1),
        ("ai:claude/* => cape", codes::BAD_SELECTOR, 1),
        ("ai:Claude => cape", codes::BAD_SELECTOR, 1),
        ("ai & => cape", codes::BAD_SELECTOR, 1),
        ("ai => capes=on", codes::BAD_PROPERTY, 1),
        ("ai => colour=agent", codes::BAD_PROPERTY, 1),
        ("ai => scale=2", codes::BAD_PROPERTY, 1),
        ("ai => cape=hash", codes::BAD_VALUE, 1),
        ("ai => cape=FFF", codes::BAD_VALUE, 1),
        ("ai => none=on", codes::BAD_VALUE, 1),
        (
            "ai & agent & assisted & human & ai => cape",
            codes::TOO_MANY_TERMS,
            1,
        ),
        ("id:abc => cape", codes::RESERVED, 1),
    ];
    for &(text, code, line) in cases {
        let e = err(text);
        assert_eq!((e.code, e.line), (code, line), "{text}: {e}");
        assert_eq!(
            e.to_string(),
            format!("{code}: {} (line {line})", e.message)
        );
    }

    let rules = vec!["ai => badge"; MAX_RULES + 1].join("\n");
    let e = err(&rules);
    assert_eq!((e.code, e.line), (codes::TOO_MANY_RULES, 33));
    assert!(parse_rules(&vec!["ai => badge"; MAX_RULES].join("\n")).is_ok());

    let mut lines = vec!["# comment"; MAX_LINES];
    lines.push("ai => badge");
    let e = err(&lines.join("\n"));
    assert_eq!((e.code, e.line), (codes::TOO_MANY_LINES, 65));
    // blank lines do not count
    let text = format!("{}\n\n\n", vec!["# c"; MAX_LINES].join("\n"));
    assert!(parse_rules(&text).is_ok());

    let long = format!("ai => badge; {}", "badge;".repeat(50));
    assert!(long.len() > MAX_LINE_BYTES);
    let e = err(&long);
    assert_eq!(e.code, codes::LINE_TOO_LONG);
    let ok = format!("ai =>{}", " ".repeat(MAX_LINE_BYTES - 10) + "badge");
    assert_eq!(ok.trim().len(), MAX_LINE_BYTES);
    assert!(parse_rules(&ok).is_ok());
}

#[test]
fn messages_name_the_problem() {
    assert_eq!(
        err("ai => capes").message,
        "Unknown property \"capes\". Known: cape, shades, hat, halo, glow, badge, none."
    );
    assert_eq!(
        err("ai => cape=pink").message,
        "Property \"cape\" does not accept \"pink\". Use on, off, agent or RRGGBB."
    );
    assert_eq!(
        err("robot => cape").message,
        "Unknown selector \"robot\". Use human, agent, assisted, ai or ai:AGENT."
    );
    let e = err(&format!("{} => cape", "x".repeat(200)));
    assert!(
        e.message
            .contains(&format!("\"{}...\"", "x".repeat(MAX_ECHO_BYTES)))
    );
}

#[test]
fn rules_keep_their_trimmed_text_and_the_last_arrow_splits() {
    let rules =
        parse_rules("  ai:codex  =>  hat=agent ; glow=agent;  \r\n# x\n!human=>badge").unwrap();
    let texts: Vec<&str> = rules.iter().map(Rule::text).collect();
    assert_eq!(
        texts,
        ["ai:codex  =>  hat=agent ; glow=agent;", "!human=>badge"]
    );
    // the value side may not hold "=>", so the selector is everything before
    // the last one
    assert_eq!(err("ai => cape => badge").code, codes::BAD_SELECTOR);
}

#[test]
fn polarity_follows_the_terms() {
    let rules =
        parse_rules("ai => badge\n!ai => badge\nhuman => hat\nhuman & assisted => hat").unwrap();
    let dynamic: Vec<bool> = rules.iter().map(Rule::is_dynamic).collect();
    assert_eq!(dynamic, [true, false, false, true]);
}

#[test]
fn default_look_by_state() {
    let layers = StyleLayers::new(AiStyle::Default, &[]);
    // no attribution: nothing
    assert_eq!(resolve(&layers, &[]), ResolvedStyle::default());

    let assisted = resolve(&layers, &[entry(AttrKind::Assist, "claude")]);
    let bits = DecoKind::Cape.bit() | DecoKind::Shades.bit() | DecoKind::Badge.bit();
    assert_eq!(assisted.mask(), bits);
    assert_eq!(assisted.dynamic, bits);
    assert_eq!(assisted.tints[DecoKind::Cape.index()], Tint::Fixed(CLAUDE));
    assert_eq!(
        assisted.tints[DecoKind::Shades.index()],
        Tint::Fixed(SHADES_LENS)
    );
    assert_eq!(assisted.tints[DecoKind::Badge.index()], Tint::Fixed(CLAUDE));
    assert_eq!(assisted.badge, agents::BadgeShape::Circle as u8);

    let agent = resolve(&layers, &[entry(AttrKind::Agent, "cursor")]);
    assert_eq!(
        agent.mask(),
        DecoKind::Halo.bit() | DecoKind::Glow.bit() | DecoKind::Badge.bit()
    );
    assert_eq!(agent.tints[DecoKind::Glow.index()], Tint::Fixed(CURSOR));
    assert_eq!(agent.badge, agents::BadgeShape::Triangle as u8);
}

#[test]
fn ai_style_variants_swap_the_builtin_look() {
    let s = [entry(AttrKind::Assist, "codex")];
    let mask = |style| resolve(&StyleLayers::new(style, &[]), &s).mask();
    assert_eq!(
        mask(AiStyle::Cape),
        DecoKind::Cape.bit() | DecoKind::Badge.bit()
    );
    assert_eq!(
        mask(AiStyle::Shades),
        DecoKind::Shades.bit() | DecoKind::Badge.bit()
    );
    assert_eq!(
        mask(AiStyle::Hat),
        DecoKind::Hat.bit() | DecoKind::Glow.bit() | DecoKind::Badge.bit()
    );
    assert_eq!(mask(AiStyle::None), 0);
    // `none` drops only the built-in layer
    let lines = vec!["assisted => hat".to_string()];
    let own = resolve(&StyleLayers::new(AiStyle::None, &lines), &s);
    assert_eq!(own.mask(), DecoKind::Hat.bit());
}

#[test]
fn cascade_later_wins_off_clears_none_clears_all() {
    let s = [entry(AttrKind::Assist, "codex")];
    let lines: Vec<String> = [
        "ai:codex => hat=agent; glow=agent",
        "assisted => cape=off",
        "ai => shades=3366CC",
    ]
    .iter()
    .map(|l| l.to_string())
    .collect();
    let r = resolve(&StyleLayers::new(AiStyle::Default, &lines), &s);
    assert_eq!(r.tints[DecoKind::Cape.index()], Tint::Off);
    assert_eq!(r.tints[DecoKind::Hat.index()], Tint::Fixed(CODEX));
    assert_eq!(r.tints[DecoKind::Glow.index()], Tint::Fixed(CODEX));
    assert_eq!(
        r.tints[DecoKind::Shades.index()],
        Tint::Fixed([0x33, 0x66, 0xCC])
    );
    assert_eq!(r.tints[DecoKind::Badge.index()], Tint::Fixed(CODEX));

    let r = resolve(&viewer("ai => cape; hat\nai => none; badge"), &s);
    assert_eq!(r.mask(), DecoKind::Badge.bit());

    // a rule for another agent does not match
    let r = resolve(&viewer("ai:claude => hat"), &s);
    assert_eq!(r.mask(), 0);
}

#[test]
fn ai_agent_term_picks_its_entry_for_tint_and_badge() {
    let s = [
        entry(AttrKind::Assist, "claude"),
        entry(AttrKind::Assist, "codex"),
    ];
    let r = resolve(&viewer("ai:codex => badge=agent; cape=on"), &s);
    assert_eq!(r.tints[DecoKind::Badge.index()], Tint::Fixed(CODEX));
    assert_eq!(r.badge, agents::BadgeShape::Diamond as u8);
    assert_eq!(r.tints[DecoKind::Cape.index()], Tint::Fixed(CODEX));
    // without an ai:id term: the first AI entry
    let r = resolve(&viewer("ai => badge"), &s);
    assert_eq!(r.badge, agents::BadgeShape::Circle as u8);
}

#[test]
fn static_rules_and_selector_semantics() {
    // a static rule draws for plain contributors, in their own colour
    let r = resolve(&viewer("human => hat=on"), &[]);
    assert_eq!(r.tints[DecoKind::Hat.index()], Tint::Contributor);
    assert_eq!(r.dynamic, 0);
    // assisted humans are still human; agent authors are not
    let s = [entry(AttrKind::Assist, "claude")];
    assert_eq!(
        resolve(&viewer("human => hat"), &s).mask(),
        DecoKind::Hat.bit()
    );
    let s = [entry(AttrKind::Agent, "cursor")];
    assert_eq!(resolve(&viewer("human => hat"), &s).mask(), 0);
    assert_eq!(
        resolve(&viewer("!assisted & agent => hat"), &s).mask(),
        DecoKind::Hat.bit()
    );
    // bot entries are neither human nor AI
    let s = [entry(AttrKind::Bot, "dependabot")];
    assert_eq!(resolve(&viewer("human => hat\nai => cape"), &s).mask(), 0);
    // `agent` without an AI entry uses the contributor's colour
    let r = resolve(&viewer("!ai => cape=agent"), &[]);
    assert_eq!(r.tints[DecoKind::Cape.index()], Tint::Contributor);
}

#[test]
fn echo_is_cut_at_a_character_boundary() {
    assert_eq!(cap_echo("abc", 3), "abc");
    assert_eq!(cap_echo("abcd", 3), "abc...");
    assert_eq!(cap_echo("aé", 2), "a...");
}
