// SPDX-License-Identifier: GPL-3.0-or-later

//! `GitRawParser` with the clone service's AI detection format: the `m1:`
//! meta line, classification, and the hostile-byte rules.

use std::path::Path;
use std::process::Command;

use gource_core::events::AttrKind::{self, Agent, Assist};
use gource_sources::{CommitSink, CustomLogWriter, GitRawParser, ParsedCommit};

/// The `--pretty` format of the clone service with detection on.
const FORMAT: &str = "--pretty=format:user:%aN%n%ct%nm1:%an%x1f%ae%x1f%cn%x1f%ce%x1f%(trailers:key=Co-authored-by,key=Made-with,unfold,separator=%x1f)%x1e%s";

type Attr = (AttrKind, String, Option<String>);

#[derive(Clone, Debug, PartialEq, Eq)]
struct Commit {
    timestamp: i64,
    username: String,
    files: Vec<String>,
    attrs: Vec<Attr>,
}

#[derive(Default)]
struct Collect(Vec<Commit>);

impl CommitSink for Collect {
    fn commit(&mut self, c: &ParsedCommit<'_>) {
        self.0.push(Commit {
            timestamp: c.timestamp,
            username: c.username.to_string(),
            files: c.files.iter().map(|f| f.path.to_string()).collect(),
            attrs: c
                .attrs
                .iter()
                .map(|a| (a.kind, a.agent.to_string(), a.model.map(str::to_string)))
                .collect(),
        });
    }
}

fn parse_chunked(input: &[u8], chunk: usize) -> (Vec<Commit>, GitRawParser) {
    let mut parser = GitRawParser::new();
    let mut sink = Collect::default();
    for part in input.chunks(chunk.max(1)) {
        parser.push(part, &mut sink);
    }
    parser.finish(&mut sink);
    (sink.0, parser)
}

/// Parse whole, and check that every chunking gives the same result.
fn parse(input: &[u8]) -> (Vec<Commit>, u64) {
    let (whole, parser) = parse_chunked(input, input.len());
    for chunk in [1, 2, 3, 7, 64] {
        let (pieces, p) = parse_chunked(input, chunk);
        assert_eq!(pieces, whole, "chunk {chunk}");
        assert_eq!(p.stats(), parser.stats(), "chunk {chunk}");
    }
    (whole, parser.stats().bad_meta)
}

fn a(kind: AttrKind, agent: &str, model: Option<&str>) -> Attr {
    (kind, agent.to_string(), model.map(str::to_string))
}

/// `m1:` line from its pieces; `\x1f` and `\x1e` are inserted.
fn m1(an: &str, ae: &str, cn: &str, ce: &str, trailers: &[&str], subject: &str) -> String {
    format!(
        "m1:{an}\x1f{ae}\x1f{cn}\x1f{ce}\x1f{}\x1e{subject}",
        trailers.join("\x1f")
    )
}

fn commit(user: &str, ts: i64, meta: &str, files: &[&str]) -> String {
    let mut s = format!("user:{user}\n{ts}\n{meta}\n");
    for f in files {
        s.push_str(&format!(":000000 100644 0000000 1111111 A\t{f}\n"));
    }
    s
}

const CLAUDE: &str = "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>";

#[test]
fn credits_reach_their_commit_only() {
    let jane = m1(
        "Jane",
        "jane@example.com",
        "Jane",
        "jane@example.com",
        &[CLAUDE],
        "feat:\tx",
    );
    let cursor = m1(
        "Cursor Agent",
        "cursoragent@cursor.com",
        "Cursor Agent",
        "cursoragent@cursor.com",
        &[""],
        "agent",
    );
    let merge = m1(
        "Bob",
        "bob@example.com",
        "Bob",
        "bob@example.com",
        &["Co-authored-by: Codex <noreply@openai.com>"],
        "merge",
    );
    let bob = m1(
        "Bob",
        "bob@example.com",
        "Bob",
        "bob@example.com",
        &[""],
        "plain",
    );
    // a merge header is followed directly by the next commit's user: line
    let input = [
        commit("Jane", 10, &jane, &["a"]),
        "\n".into(),
        commit("Cursor Agent", 11, &cursor, &["b"]),
        "\n".into(),
        format!("user:Bob\n12\n{merge}\n"),
        commit("Bob", 13, &bob, &["c"]),
        "\n".into(),
        // a merge whose next commit has no m1: line (old producer)
        format!("user:Bob\n14\n{merge}\n"),
        commit("Bob", 15, "", &["d"]).replace("15\n\n", "15\n"),
        "\n".into(),
        // ... or a malformed one
        format!("user:Bob\n16\n{merge}\n"),
        commit("Bob", 17, "m1:Bob\x1fbob@example.com", &["e"]),
        "\n".into(),
        // an agent-authored merge followed by a human commit
        format!("user:Cursor Agent\n18\n{cursor}\n"),
        commit("Bob", 19, &bob, &["f"]),
    ]
    .concat();
    let (commits, bad) = parse(input.as_bytes());
    let attrs: Vec<(&str, Vec<Attr>)> = commits
        .iter()
        .map(|c| (c.files[0].as_str(), c.attrs.clone()))
        .collect();
    assert_eq!(
        attrs,
        [
            ("/a", vec![a(Assist, "claude", Some("Claude Opus 5.5"))]),
            ("/b", vec![a(Agent, "cursor", None)]),
            ("/c", vec![]),
            ("/d", vec![]),
            ("/e", vec![]),
            ("/f", vec![]),
        ]
    );
    assert_eq!(bad, 1, "the truncated meta line");
    assert_eq!(commits[0].username, "Jane");
}

#[test]
fn a_stale_meta_line_is_skipped_and_never_a_file() {
    let jane = m1(
        "Jane",
        "j@x.org",
        "Jane",
        "j@x.org",
        &[CLAUDE],
        "subject\twith tab",
    );
    // m1: after the files, then the next header
    let input = format!(
        "user:Jane\n10\n:000000 100644 0000000 1111111 A\ta\n{jane}\nuser:Jane\n11\n{jane}\n:000000 100644 0000000 1111111 A\tb\n"
    );
    let (commits, _) = parse(input.as_bytes());
    assert_eq!(commits.len(), 1, "{commits:?}");
    assert_eq!(commits[0].files, ["/a", "/b"]);
    // the second header replaced the first; its m1: line applies
    assert_eq!(
        commits[0].attrs,
        [a(Assist, "claude", Some("Claude Opus 5.5"))]
    );
    let (_, parser) = parse_chunked(input.as_bytes(), input.len());
    assert_eq!(parser.stats().skipped_lines, 1);
}

#[test]
fn a_meta_line_without_files_and_tab_in_subject() {
    let line = m1("Jane", "j@x.org", "Jane", "j@x.org", &[CLAUDE], "a\tb\tc");
    let input = format!("user:Jane\n10\n{line}\n:000000 100644 0000000 1111111 A\tx\n");
    let (commits, bad) = parse(input.as_bytes());
    assert_eq!(bad, 0);
    assert_eq!(commits[0].files, ["/x"]);
    assert_eq!(commits[0].attrs.len(), 1);
}

#[test]
fn hostile_bytes_leave_the_commit_unattributed() {
    // T26 (synthetic): separators inside names shift the fields
    let cases = [
        // author name containing 0x1f
        m1("Ja\x1fne", "j@x.org", "Jane", "j@x.org", &[CLAUDE], "s"),
        // author name containing 0x1e
        m1("Ja\x1ene", "j@x.org", "Jane", "j@x.org", &[CLAUDE], "s"),
        // an email piece with a space
        m1("Jane", "j @x.org", "Jane", "j@x.org", &[CLAUDE], "s"),
        m1("Jane", "j@x.org", "Jane", "j@x.org\t", &[CLAUDE], "s"),
        // a trailer piece without ':'
        m1(
            "Jane",
            "j@x.org",
            "Jane",
            "j@x.org",
            &[CLAUDE, "no colon"],
            "s",
        ),
        // a control character in a name
        m1("Ja\x07ne", "j@x.org", "Jane", "j@x.org", &[CLAUDE], "s"),
        // no record separator: a truncated line
        format!("m1:Jane\x1fj@x.org\x1fJane\x1fj@x.org\x1f{CLAUDE}"),
        // fewer than five pieces
        "m1:Jane\x1fj@x.org\x1fJane\x1fj@x.org\x1e".to_string(),
        "m1:".to_string(),
    ];
    for meta in &cases {
        let input = commit("Jane", 10, meta, &["a"]);
        let (commits, bad) = parse(input.as_bytes());
        assert_eq!(commits.len(), 1, "{meta:?}");
        assert!(commits[0].attrs.is_empty(), "{meta:?}");
        assert_eq!(bad, 1, "{meta:?}");
    }
    // a 0x1e or 0x1f in the subject is harmless
    let input = commit(
        "Jane",
        10,
        &m1(
            "Jane",
            "j@x.org",
            "Jane",
            "j@x.org",
            &[CLAUDE],
            "x\x1fy\x1ez",
        ),
        &["a"],
    );
    let (commits, bad) = parse(input.as_bytes());
    assert_eq!((commits[0].attrs.len(), bad), (1, 0));
}

#[test]
fn a_mailmapped_agent_identity_is_downgraded() {
    // T25 (synthetic): raw author claude[bot], mailmapped name Jane Doe
    let meta = m1(
        "claude[bot]",
        "41898282+claude[bot]@users.noreply.github.com",
        "claude[bot]",
        "41898282+claude[bot]@users.noreply.github.com",
        &[""],
        "fix",
    );
    let (commits, _) = parse(commit("Jane Doe", 10, &meta, &["a"]).as_bytes());
    assert_eq!(commits[0].attrs, [a(Assist, "claude", None)]);
    let (commits, _) = parse(commit("claude[bot]", 10, &meta, &["a"]).as_bytes());
    assert_eq!(commits[0].attrs, [a(Agent, "claude", None)]);
}

#[test]
fn the_old_format_parses_unchanged() {
    let input = b"user:A\n10\n:000000 100644 0000000 1111111 A\ta\n\nuser:M\n11\nuser:B\n12\n:000000 100644 0000000 1111111 M\ta\n";
    let (commits, bad) = parse(input);
    assert_eq!(bad, 0);
    assert_eq!(commits.len(), 2);
    assert!(commits.iter().all(|c| c.attrs.is_empty()));
    assert_eq!(commits[1].username, "B");
}

#[test]
fn credits_are_canonical_and_capped() {
    let trailers = [
        "Co-authored-by: Codex <noreply@openai.com>",
        "Co-authored-by: Claude Sonnet 5 <noreply@anthropic.com>",
        "Co-authored-by: Amp <amp@ampcode.com>",
        "Co-authored-by: openhands <openhands@all-hands.dev>",
        "Co-authored-by: Claude Haiku 4.5 <noreply@anthropic.com>",
        "Made-with: Cursor",
    ];
    let meta = m1("Jane", "j@x.org", "Jane", "j@x.org", &trailers, "s");
    let input = commit("Jane", 10, &meta, &["a"]);
    let (commits, _) = parse(input.as_bytes());
    assert_eq!(
        commits[0].attrs,
        [
            a(Assist, "claude", Some("Claude Haiku 4.5")),
            a(Assist, "claude", Some("Claude Sonnet 5")),
            a(Assist, "codex", None),
            a(Assist, "cursor", None),
        ]
    );
    let (_, parser) = parse_chunked(input.as_bytes(), input.len());
    assert_eq!(parser.attrs_truncated(), 2);
}

fn git_version() -> Option<(u32, u32)> {
    let out = Command::new("git").arg("--version").output().ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    let mut parts = text.split_whitespace().nth(2)?.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}

fn git(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Vec<u8> {
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .envs(env.iter().copied())
        .args([
            "-c",
            "init.defaultBranch=main",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

fn git_commit(dir: &Path, name: &str, email: &str, time: i64, msg: &str) {
    let date = format!("@{time} +0000");
    git(dir, &["add", "-A"], &[]);
    git(
        dir,
        &["commit", "-q", "-m", msg],
        &[
            ("GIT_AUTHOR_NAME", name),
            ("GIT_AUTHOR_EMAIL", email),
            ("GIT_COMMITTER_NAME", name),
            ("GIT_COMMITTER_EMAIL", email),
            ("GIT_AUTHOR_DATE", &date),
            ("GIT_COMMITTER_DATE", &date),
        ],
    );
}

/// The command of the clone service against a scratch repository: the
/// parser and writer produce the documented custom-log lines, and no email
/// or trailer text survives.
#[test]
fn real_git_output_of_the_detection_format() {
    if git_version().is_none_or(|v| v < (2, 22)) {
        eprintln!("skipped: needs git 2.22 or newer");
        return;
    }
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("git_meta_scratch");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q"], &[]);

    std::fs::write(dir.join("a.rs"), "a").unwrap();
    git_commit(
        &dir,
        "jane",
        "jane@example.com",
        1_790_942_378,
        &format!("feat:\twith a tab\n\nbody\n\n{CLAUDE}\nCo-authored-by: Bob <bob@old.example>"),
    );
    git(&dir, &["checkout", "-q", "-b", "dev"], &[]);
    std::fs::write(dir.join("x.ts"), "x").unwrap();
    git_commit(
        &dir,
        "Cursor Agent",
        "cursoragent@cursor.com",
        1_790_942_500,
        "agent work",
    );
    git(&dir, &["checkout", "-q", "main"], &[]);
    std::fs::write(dir.join("b.rs"), "b").unwrap();
    git_commit(
        &dir,
        "jane",
        "jane@example.com",
        1_790_942_700,
        "two\n\nCo-authored-by: Claude Opus 5.5 <noreply@anthropic.com>\nCo-authored-by: Codex <noreply@openai.com>",
    );
    git(
        &dir,
        &[
            "merge",
            "-q",
            "--no-ff",
            "dev",
            "-m",
            "merge\n\nMade-with: Cursor",
        ],
        &[
            ("GIT_AUTHOR_NAME", "jane"),
            ("GIT_AUTHOR_EMAIL", "jane@example.com"),
            ("GIT_COMMITTER_NAME", "jane"),
            ("GIT_COMMITTER_EMAIL", "jane@example.com"),
            ("GIT_AUTHOR_DATE", "@1790942800 +0000"),
            ("GIT_COMMITTER_DATE", "@1790942800 +0000"),
        ],
    );
    std::fs::write(dir.join("c.rs"), "c").unwrap();
    git_commit(&dir, "bob", "bob@example.com", 1_790_942_900, "plain");

    let raw = git(
        &dir,
        &[
            "log",
            "--reverse",
            "--raw",
            "--encoding=UTF-8",
            "--no-renames",
            "--no-show-signature",
            FORMAT,
        ],
        &[],
    );
    assert!(raw.windows(3).any(|w| w == b"m1:"));

    let mut parser = GitRawParser::new();
    let mut writer = CustomLogWriter::new(Vec::new());
    parser.push(&raw, &mut writer);
    parser.finish(&mut writer);
    assert_eq!(parser.stats().bad_meta, 0);
    let text = String::from_utf8(writer.finish().unwrap()).unwrap();
    // the merge has no files and is dropped; its credits go nowhere
    assert_eq!(
        text,
        "1790942378|jane|A|/a.rs||ai=co:claude:Claude Opus 5.5\n\
         1790942500|Cursor Agent|A|/x.ts||ai=au:cursor\n\
         1790942700|jane|A|/b.rs||ai=co:claude:Claude Opus 5.5,co:codex\n\
         1790942900|bob|A|/c.rs\n"
    );
    for leak in ["@", "Bob", "Co-authored", "with a tab"] {
        assert!(!text.contains(leak), "{leak} in {text}");
    }
}
