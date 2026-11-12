// SPDX-License-Identifier: GPL-3.0-or-later

//! The custom log's meta field: grammar, limits and errors, the parser's
//! union over a commit's lines, and the writer.

mod common;

use common::*;
use gource_core::events::AttrKind::{self, Agent, Assist};
use gource_core::events::ChangeEvent;
use gource_sources::meta::{self, MAX_META_BYTES};
use gource_sources::{
    CustomLogParser, CustomLogWriter, ParsedAttribution, ParsedCommit, ParsedFile,
};

fn a(kind: AttrKind, agent: &str, model: Option<&str>) -> Attr {
    (kind, agent.to_string(), model.map(str::to_string))
}

fn read(field: &str) -> (Vec<Attr>, u64) {
    let (credits, bad) = meta::parse(field.as_bytes());
    (
        credits
            .into_iter()
            .map(|c| (c.kind, c.agent, c.model))
            .collect(),
        bad,
    )
}

#[test]
fn valid_fields() {
    let claude = || a(Assist, "claude", Some("Claude Opus 5.5"));
    for (field, want) in [
        ("", vec![]),
        ("ai=co:claude:Claude Opus 5.5", vec![claude()]),
        ("ai=au:cursor", vec![a(Agent, "cursor", None)]),
        // literal ':' and ',' structure the credits
        (
            "ai=co:claude:Claude Opus 5.5,co:codex",
            vec![claude(), a(Assist, "codex", None)],
        ),
        (
            "ai=co:aider:gemini/gemini-3.1-pro-preview",
            vec![a(Assist, "aider", Some("gemini/gemini-3.1-pro-preview"))],
        ),
        (
            "ai=co:aider:ollama/llama3%3A8b",
            vec![a(Assist, "aider", Some("ollama/llama3:8b"))],
        ),
        // either hex case, any escape, spaces trimmed after decoding
        ("ai=co:cl%61ude: Claude Opus 5.5 ", vec![claude()]),
        ("ai=co:claude:Claude%20Opus%205%2e5", vec![claude()]),
        ("ai=co:claude:a%2cb", vec![a(Assist, "claude", Some("a,b"))]),
        // unknown agents are valid
        ("ai=co:windsurf", vec![a(Assist, "windsurf", None)]),
        // an empty model is no model
        ("ai=co:claude:", vec![a(Assist, "claude", None)]),
        // other keys around it
        ("bot=dependabot;ai=co:codex", vec![a(Assist, "codex", None)]),
        ("x-anything=%%%;ai=co:codex", vec![a(Assist, "codex", None)]),
        (
            "cid=AbCdEfGhIjKlMnOp;pr=12;ai=co:codex",
            vec![a(Assist, "codex", None)],
        ),
        // duplicate key: the first wins
        ("ai=co:codex;ai=au:cursor", vec![a(Assist, "codex", None)]),
        // five credits: the fifth is not read
        (
            "ai=co:a,co:b,co:c,co:d,co:Bad Id",
            vec![
                a(Assist, "a", None),
                a(Assist, "b", None),
                a(Assist, "c", None),
                a(Assist, "d", None),
            ],
        ),
    ] {
        assert_eq!(read(field), (want, 0), "{field:?}");
    }
}

#[test]
fn malformed_fields_cost_only_their_pair() {
    let codex = || vec![a(Assist, "codex", None)];
    for (field, want, bad) in [
        // legacy sixth column text
        ("more", vec![], 1),
        // a pair without '=', a bad key, an empty pair
        ("ai;ai=co:codex", codex(), 1),
        ("Ai=co:codex", vec![], 1),
        ("1ai=x;ai=co:codex", codex(), 1),
        ("ai=co:codex;", codex(), 1),
        (&format!("{}=1;ai=co:codex", "k".repeat(25)), codex(), 1),
        // bad escapes
        ("ai=co:claude:%ZZ", vec![], 1),
        ("ai=co:claude:%4", vec![], 1),
        ("ai=co:claude:%", vec![], 1),
        ("ai=co:claude:%FF", vec![], 1),
        ("pr=%G0;ai=co:codex", codex(), 1),
        // decoded C0, C1, DEL and bidi controls
        ("ai=co:claude:a%0Ab", vec![], 1),
        ("ai=co:claude:a%C2%85b", vec![], 1),
        ("ai=co:claude:a%7Fb", vec![], 1),
        ("ai=co:claude:a%E2%80%AEb", vec![], 1),
        ("ai=co:claude:a%E2%81%A6b", vec![], 1),
        ("ai=co:claude:a%D8%9Cb", vec![], 1),
        // raw controls and bidi characters
        ("ai=co:claude:a\u{202e}b", vec![], 1),
        ("ai=co:claude:a\u{85}b", vec![], 1),
        ("ai=co:claude:a\tb", vec![], 1),
        // a raw '=' or '|' in a value
        ("ai=co:claude:a=b", vec![], 1),
        ("ai=co:codex|x", vec![], 1),
        // bad credits make the whole ai pair go
        ("ai=zz:claude", vec![], 1),
        ("ai=bot:dependabot", vec![], 1),
        ("ai=co:Claude", vec![], 1),
        ("ai=co:", vec![], 1),
        ("ai=", vec![], 1),
        ("ai=co", vec![], 1),
        ("ai=co:claude:a:b", vec![], 1),
        ("ai=co:codex,co:Bad", vec![], 1),
        (&format!("ai=co:{}", "a".repeat(33)), vec![], 1),
        (&format!("ai=co:claude:{}", "m".repeat(65)), vec![], 1),
        ("ai=co:claude:a@b", vec![], 1),
        ("ai=co:claude:Opus\u{e9}", vec![], 1),
        // a bad bot id
        ("bot=Dependabot[bot];ai=co:codex", codex(), 1),
        // the first of two pairs wins even when it is bad
        ("ai=zz:x;ai=co:codex", vec![], 1),
        // a ninth pair is not read
        ("a=1;b=1;c=1;d=1;e=1;f=1;g=1;h=1;ai=co:codex", vec![], 0),
        // invalid UTF-8 is a bad value
        ("pr=\u{fffd}", vec![], 0),
    ] {
        assert_eq!(read(field), (want, bad), "{field:?}");
    }
    let (credits, bad) = meta::parse(b"ai=co:claude:\xff");
    assert_eq!((credits.len(), bad), (0, 1));
}

#[test]
fn the_size_limit_is_512_bytes() {
    let fill = |n: usize| {
        let mut f = "ai=co:codex;x-pad=".to_string();
        f.push_str(&"p".repeat(n - f.len()));
        f
    };
    assert_eq!(
        read(&fill(MAX_META_BYTES)),
        (vec![a(Assist, "codex", None)], 0)
    );
    assert_eq!(read(&fill(MAX_META_BYTES + 1)), (vec![], 1));
    let huge = "ai=co:codex;".repeat(100_000 / 12);
    assert_eq!(read(&huge), (vec![], 1));
}

fn parse_log(text: &str) -> (Vec<Commit>, gource_sources::ParseStats) {
    let mut sink = Collect::default();
    let stats = parse_chunked(text.as_bytes(), &[text.len().max(1)], &mut sink);
    (sink.commits, stats)
}

#[test]
fn the_colour_slot_is_read_as_before() {
    let (commits, stats) = parse_log(
        "1|u|A|/a||ai=au:cursor\n2|u|A|/b|ff0000|ai=co:codex\n3|u|A|/c|ff0000|more\n4|u|A|/d|deadbeef|ai=co:codex\n5|u|A|/e|#00ff00zz|ai=co:codex\n",
    );
    let colours: Vec<_> = commits.iter().map(|c| c.files[0].2).collect();
    assert_eq!(
        colours,
        [
            None,
            Some([0xff, 0, 0]),
            Some([0xff, 0, 0]),
            Some([0xde, 0xad, 0xbe]),
            Some([0, 0xff, 0])
        ]
    );
    let attrs: Vec<usize> = commits.iter().map(|c| c.attrs.len()).collect();
    assert_eq!(attrs, [1, 1, 0, 1, 1]);
    assert_eq!(stats.bad_meta, 1);
    assert_eq!(stats.skipped_lines, 0);
}

#[test]
fn credits_are_the_union_of_a_commits_lines() {
    let (commits, stats) = parse_log(
        "\
1790942378|jane|A|/a||ai=co:codex
1790942378|jane|M|/b
1790942378|jane|M|/c||ai=co:claude:Claude Opus 5.5,co:codex
1790942378|jane|M|/d||ai=co:claude
1790942379|jane|M|/e
1790942379|bob|M|/f||ai=au:cursor
",
    );
    assert_eq!(stats.bad_meta, 0);
    assert_eq!(commits.len(), 3);
    assert_eq!(
        commits[0].attrs,
        [
            a(Assist, "claude", Some("Claude Opus 5.5")),
            a(Assist, "codex", None)
        ]
    );
    assert!(commits[1].attrs.is_empty());
    assert_eq!(commits[2].attrs, [a(Agent, "cursor", None)]);
}

#[test]
fn a_malformed_line_ends_the_commit_and_its_credits() {
    let (commits, _) = parse_log("1|u|A|/a||ai=co:codex\nnonsense\n1|u|A|/b\n");
    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0].attrs.len(), 1);
    assert!(commits[1].attrs.is_empty());
}

#[test]
fn chunking_does_not_change_credits() {
    let text =
        std::fs::read(repo_root().join("tests/fixtures/custom-log/ai-coauthors.log")).unwrap();
    let mut whole = Collect::default();
    let stats = parse_chunked(&text, &[text.len()], &mut whole);
    assert!(whole.commits.iter().any(|c| !c.attrs.is_empty()));
    for sizes in [&[1][..], &[2], &[3], &[5, 1, 9], &[64], &[4096]] {
        let mut sink = Collect::default();
        assert_eq!(parse_chunked(&text, sizes, &mut sink), stats, "{sizes:?}");
        assert_eq!(sink.commits, whole.commits, "{sizes:?}");
    }
}

fn write(commits: &[ParsedCommit<'_>]) -> (String, u64) {
    let mut w = CustomLogWriter::new(Vec::new());
    for c in commits {
        gource_sources::CommitSink::commit(&mut w, c);
    }
    let trimmed = w.stats().meta_trimmed;
    (String::from_utf8(w.finish().unwrap()).unwrap(), trimmed)
}

fn file(path: &str, colour: Option<[u8; 3]>) -> ParsedFile<'_> {
    ParsedFile {
        path,
        kind: gource_core::events::ChangeKind::Add,
        colour,
        event: ChangeEvent::Commit,
    }
}

#[test]
fn the_writer_puts_the_field_on_the_first_line_only() {
    let attrs = [
        ParsedAttribution {
            kind: Assist,
            agent: "claude",
            model: Some("Claude Opus 5.5"),
        },
        ParsedAttribution {
            kind: Assist,
            agent: "aider",
            model: Some("ollama/llama3:8b,x"),
        },
    ];
    let files = [file("/a.rs", None), file("/b.rs", Some([1, 2, 3]))];
    let (out, trimmed) = write(&[
        ParsedCommit {
            timestamp: 1790942378,
            username: "jane",
            files: &files,
            attrs: &attrs,
            byte_offset: 0,
        },
        ParsedCommit {
            timestamp: 1790942500,
            username: "Cursor Agent",
            files: &[file("/x.ts", Some([0xff, 0x88, 0]))],
            attrs: &[ParsedAttribution {
                kind: Agent,
                agent: "cursor",
                model: None,
            }],
            byte_offset: 0,
        },
    ]);
    assert_eq!(trimmed, 0);
    assert_eq!(
        out,
        "1790942378|jane|A|/a.rs||ai=co:claude:Claude Opus 5.5,co:aider:ollama/llama3%3A8b%2Cx\n\
         1790942378|jane|A|/b.rs|010203\n\
         1790942500|Cursor Agent|A|/x.ts|FF8800|ai=au:cursor\n"
    );

    // and the parser reads back what was written
    let (commits, stats) = parse_log(&out);
    assert_eq!(stats.bad_meta, 0);
    assert_eq!(
        commits[0].attrs,
        [
            a(Assist, "claude", Some("Claude Opus 5.5")),
            a(Assist, "aider", Some("ollama/llama3:8b,x")),
        ]
    );
}

#[test]
fn writer_overflow_drops_trailing_credits() {
    let model = ":/".repeat(32);
    let attrs: Vec<ParsedAttribution<'_>> = ["aa", "bb", "cc", "dd"]
        .iter()
        .map(|agent| ParsedAttribution {
            kind: Assist,
            agent,
            model: Some(&model),
        })
        .collect();
    let (out, trimmed) = write(&[ParsedCommit {
        timestamp: 1,
        username: "u",
        files: &[file("/a", None)],
        attrs: &attrs,
        byte_offset: 0,
    }]);
    let field = out.trim_end().split('|').nth(5).unwrap();
    assert!(field.len() <= MAX_META_BYTES, "{}", field.len());
    // each credit is "co:xx:" plus 32 escaped ':' and 32 '/' = 134 bytes
    assert_eq!(trimmed, 1);
    let (commits, stats) = parse_log(&out);
    assert_eq!(stats.bad_meta, 0);
    assert_eq!(commits[0].attrs.len(), 3);
    assert_eq!(commits[0].attrs[0].2.as_deref(), Some(model.as_str()));
}

#[test]
fn round_trip_parser_writer_parser() {
    let text =
        std::fs::read_to_string(repo_root().join("tests/fixtures/custom-log/ai-coauthors.log"))
            .unwrap();
    let (first, _) = parse_log(&text);
    let mut w = CustomLogWriter::new(Vec::new());
    let mut parser = CustomLogParser::new();
    parser.push(text.as_bytes(), &mut w);
    parser.finish(&mut w);
    let written = String::from_utf8(w.finish().unwrap()).unwrap();
    let (second, stats) = parse_log(&written);
    assert_eq!(stats.bad_meta, 0, "{written}");
    let strip = |c: &[Commit]| -> Vec<(i64, String, Vec<Attr>)> {
        c.iter()
            .map(|c| (c.timestamp, c.username.clone(), c.attrs.clone()))
            .collect()
    };
    assert_eq!(strip(&first), strip(&second));
    // fields only on first lines
    let mut prev = None;
    for line in written.lines() {
        let fields: Vec<&str> = line.split('|').collect();
        let key = (fields[0], fields[1]);
        if fields.len() == 6 {
            assert_ne!(prev, Some(key), "{line}");
        }
        prev = Some(key);
    }
}
