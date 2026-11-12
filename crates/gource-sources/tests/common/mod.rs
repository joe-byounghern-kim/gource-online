// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Helpers shared by the integration tests.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use gource_core::events::{AttrKind, ChangeKind, EventLog};
use gource_sources::{CommitSink, CustomLogParser, ParseStats, ParsedCommit};

/// An owned AI credit.
pub type Attr = (AttrKind, String, Option<String>);

/// An owned copy of a [`ParsedCommit`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub timestamp: i64,
    pub username: String,
    pub files: Vec<(ChangeKind, String, Option<[u8; 3]>)>,
    pub attrs: Vec<Attr>,
    pub byte_offset: u64,
}

/// Owned copies of a commit's credits.
pub fn owned_attrs(c: &ParsedCommit<'_>) -> Vec<Attr> {
    c.attrs
        .iter()
        .map(|a| (a.kind, a.agent.to_string(), a.model.map(str::to_string)))
        .collect()
}

/// Sink that keeps every commit.
#[derive(Default)]
pub struct Collect {
    pub commits: Vec<Commit>,
}

impl CommitSink for Collect {
    fn commit(&mut self, c: &ParsedCommit<'_>) {
        self.commits.push(Commit {
            timestamp: c.timestamp,
            username: c.username.to_string(),
            files: c
                .files
                .iter()
                .map(|f| (f.kind, f.path.to_string(), f.colour))
                .collect(),
            attrs: owned_attrs(c),
            byte_offset: c.byte_offset,
        });
    }
}

/// Sink that only counts.
#[derive(Default)]
pub struct Count {
    pub commits: u64,
    pub files: u64,
}

impl CommitSink for Count {
    fn commit(&mut self, c: &ParsedCommit<'_>) {
        self.commits += 1;
        self.files += c.files.len() as u64;
    }
}

/// Parse `bytes` pushing it in pieces of the given sizes (cycled), then
/// `finish`.
pub fn parse_chunked(bytes: &[u8], sizes: &[usize], sink: &mut dyn CommitSink) -> ParseStats {
    let mut parser = CustomLogParser::new();
    let mut at = 0;
    let mut i = 0;
    while at < bytes.len() {
        let n = sizes[i % sizes.len()].max(1).min(bytes.len() - at);
        parser.push(&bytes[at..at + n], sink);
        at += n;
        i += 1;
    }
    parser.finish(sink);
    parser.stats()
}

pub fn parse_whole(bytes: &[u8]) -> (Vec<Commit>, ParseStats) {
    let mut sink = Collect::default();
    let mut parser = CustomLogParser::new();
    parser.push(bytes, &mut sink);
    parser.finish(&mut sink);
    (sink.commits, parser.stats())
}

/// One commit per block, one file per indented line:
///
/// ```text
/// 1700000800 valid
///   A /ok/file.rs
///   M /with/colour #ff0000
/// ```
pub fn render(commits: &[Commit]) -> String {
    let mut out = String::new();
    for c in commits {
        out.push_str(&format!("{} {}\n", c.timestamp, c.username));
        for (kind, path, colour) in &c.files {
            out.push_str(&format!("  {} {}", kind.as_action(), path));
            if let Some([r, g, b]) = colour {
                out.push_str(&format!(" #{r:02x}{g:02x}{b:02x}"));
            }
            out.push('\n');
        }
    }
    out
}

/// The canonical dump shared with the C++ reference harness (hex encoded
/// names so any byte string survives): `C <ts> <user> <files> <offset>` per
/// commit, `F <kind> <path> <colour|->` per file and a closing `S` line.
pub fn canonical(commits: &[Commit], stats: &ParseStats) -> String {
    fn hex(s: &str) -> String {
        s.bytes().map(|b| format!("{b:02x}")).collect()
    }
    let mut out = String::new();
    for c in commits {
        out.push_str(&format!(
            "C {} {} {} {}\n",
            c.timestamp,
            hex(&c.username),
            c.files.len(),
            c.byte_offset
        ));
        for (kind, path, colour) in &c.files {
            let colour = colour.map_or("-".to_string(), |[r, g, b]| {
                format!("{r:02x}{g:02x}{b:02x}")
            });
            out.push_str(&format!(
                "F {} {} {}\n",
                kind.as_action(),
                hex(path),
                colour
            ));
        }
    }
    out.push_str(&format!(
        "S {} {} {} {}\n",
        stats.lines, stats.skipped_lines, stats.commits, stats.files
    ));
    out
}

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Path relative to the repository's `tests/` directory.
pub fn read_test_file(rel: &str) -> Vec<u8> {
    let path = repo_root().join("tests").join(rel);
    std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Every `*.log` under `tests/fixtures/custom-log` and `tests/logs`, as
/// paths relative to `tests/`, sorted.
pub fn all_log_files() -> Vec<String> {
    let mut found = Vec::new();
    for dir in ["fixtures/custom-log", "logs"] {
        let full = repo_root().join("tests").join(dir);
        for entry in std::fs::read_dir(&full).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            if name.ends_with(".log") {
                found.push(format!("{dir}/{name}"));
            }
        }
    }
    found.sort();
    found
}

/// Small deterministic generator (xorshift64*), so tests need no crates.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in `0..n` (n > 0).
    pub fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }

    pub fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }
}

/// Compare two logs structurally: strings by id, commit records, changes.
pub fn assert_logs_equal(a: &EventLog, b: &EventLog) {
    assert_eq!(a.string_count(), b.string_count(), "string count");
    for i in 0..a.string_count() {
        let id = gource_core::events::StrId(i as u32);
        assert_eq!(a.str(id), b.str(id), "string {i}");
    }
    assert_eq!(a.commit_count(), b.commit_count(), "commit count");
    assert_eq!(a.change_count(), b.change_count(), "change count");
    assert_eq!(a.records(), b.records(), "commit records");
    for i in 0..a.commit_count() {
        assert_eq!(
            a.commit(i).changes,
            b.commit(i).changes,
            "changes of commit {i}"
        );
    }
    assert_eq!(a.total_bytes(), b.total_bytes(), "total bytes");
    assert_eq!(a.attr_set_count(), b.attr_set_count(), "attribution sets");
    for ((ia, sa), (ib, sb)) in a.attr_sets().zip(b.attr_sets()) {
        assert_eq!((ia, sa), (ib, sb), "attribution set {}", ia.0);
    }
    assert_eq!(
        a.attr_commit_counts(),
        b.attr_commit_counts(),
        "commits per set"
    );
}

/// Varied custom logs with meta fields: valid credits of known and unknown
/// agents, models with escapes, colours before the field, unions over
/// several lines, malformed and over-long fields.
pub fn generate_ai_log(rng: &mut Rng, lines: usize) -> Vec<u8> {
    const USERS: &[&str] = &["jane", "Cursor Agent", "bob", "Ada"];
    const FIELDS: &[&str] = &[
        "ai=co:claude:Claude Opus 5.5",
        "ai=co:claude:Claude Haiku 4.5,co:claude:Claude Sonnet 5",
        "ai=au:cursor",
        "ai=co:codex",
        "ai=co:aider:ollama/llama3%3A8b",
        "ai=co:windsurf-x:m%2C1",
        "ai=co:claude;bot=dependabot",
        "x-private=1;ai=co:amp",
        "more",
        "ai=zz:claude",
        "ai=co:Claude",
        "ai=co:claude:%ZZ",
        "ai=co:claude:a%E2%80%AEb",
        "bot=dependabot",
        "",
    ];
    let mut out = Vec::new();
    let mut t = 1_700_000_000i64;
    for i in 0..lines {
        if rng.chance(60) {
            t += rng.below(3) as i64;
        }
        let user = USERS[rng.below(USERS.len())];
        let kind = ["A", "M", "D"][rng.below(3)];
        out.extend_from_slice(format!("{t}|{user}|{kind}|/src/f{}.rs", i % 23).as_bytes());
        match rng.below(6) {
            0 => {}
            1 => out.extend_from_slice(b"|ff8800"),
            2 => {
                let field = FIELDS[rng.below(FIELDS.len())];
                out.extend_from_slice(format!("|ff8800|{field}").as_bytes());
            }
            3 if rng.chance(5) => {
                out.extend_from_slice(b"||ai=co:claude:");
                out.extend(std::iter::repeat_n(b'x', 600));
            }
            3 => {
                // a distinct model per line: many sets
                out.extend_from_slice(format!("||ai=co:claude:m{}", rng.below(50)).as_bytes());
            }
            _ => {
                let field = FIELDS[rng.below(FIELDS.len())];
                out.extend_from_slice(format!("||{field}").as_bytes());
            }
        }
        out.push(b'\n');
    }
    out
}

/// Generates varied custom logs for property-style tests: unix and ISO
/// timestamps, grouping, malformed lines, colours, unicode and invalid UTF-8,
/// CRLF, byte order marks, blank lines.
pub fn generate_log(rng: &mut Rng, lines: usize) -> Vec<u8> {
    const USERS: &[&[u8]] = &[
        b"alice",
        b"bob",
        b"Ada Lovelace",
        b"",
        "Zo\u{eb}".as_bytes(),
        "\u{6df1}\u{5733}".as_bytes(),
        b"bad\xffbyte",
        b"cut\xe2\x82",
        b"x y z",
    ];
    const PATHS: &[&[u8]] = &[
        b"src/main.rs",
        b"/abs/path.txt",
        b"README",
        b"dir/",
        b"dir/sub/file.tar.gz",
        "assets/emoji-\u{1f332}/caf\u{e9}.txt".as_bytes(),
        b"bad/\xc0\x80/path",
        b"\xff",
        b"a b/c d",
        b"with#hash",
    ];
    const DATES: &[&[u8]] = &[
        b"2010-01-02",
        b"2010-01-02 03:04:05",
        b"2010-01-02T03:04:05Z",
        b"2010-01-02T03:04:05+05:30",
        b"2010-01-02T03:04:05.678",
        b"2010-13-45T25:61:61Z",
        b"2010-01-01T03:04+530",
    ];

    let mut out = Vec::new();
    let mut ts: i64 = 1_700_000_000;
    let mut user = rng.pick(USERS);
    for _ in 0..lines {
        // stay in the same commit often, so grouping gets exercised
        if rng.chance(35) {
            ts += rng.below(5) as i64 - 1;
            user = rng.pick(USERS);
        }
        match rng.below(40) {
            0 => out.extend_from_slice(b"\n"),
            1 => out.extend_from_slice(b"not a log line\n"),
            2 => out.extend_from_slice(b"1700000000|only|three\n"),
            3 => out.extend_from_slice(b"abc|u|A|/p\n"),
            4 => out.extend_from_slice(b"1|u|X|/p\n"),
            5 => out.extend_from_slice(b"1|u|A|\n"),
            _ => {
                if rng.chance(3) {
                    out.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
                }
                if rng.chance(8) {
                    out.extend_from_slice(rng.pick(DATES));
                } else if rng.chance(3) {
                    out.extend_from_slice(b"0");
                } else {
                    out.extend_from_slice(ts.to_string().as_bytes());
                }
                out.push(b'|');
                out.extend_from_slice(user);
                out.push(b'|');
                out.extend_from_slice(rng.pick(&[&b"A"[..], b"M", b"D", b""]));
                out.push(b'|');
                out.extend_from_slice(rng.pick(PATHS));
                match rng.below(8) {
                    0 => out.extend_from_slice(b"|ff8800"),
                    1 => out.extend_from_slice(b"|#00AAbb"),
                    2 => out.extend_from_slice(b"|12345"),
                    3 => out.extend_from_slice(b"|zz|more"),
                    _ => {}
                }
                if rng.chance(10) {
                    out.push(b'\r');
                }
                out.push(b'\n');
            }
        }
    }
    // sometimes no newline at the end
    if rng.chance(30) && out.last() == Some(&b'\n') {
        out.pop();
    }
    out
}

/// Every log in the repository plus a few generated ones.
pub fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut inputs: Vec<(String, Vec<u8>)> = all_log_files()
        .into_iter()
        .map(|f| {
            let bytes = read_test_file(&f);
            (f, bytes)
        })
        .collect();
    for seed in 1..=6u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        inputs.push((format!("generated-{seed}"), generate_log(&mut rng, 400)));
    }
    for seed in 1..=3u64 {
        let mut rng = Rng(seed.wrapping_mul(0xD1B5_4A32_D192_ED03));
        inputs.push((
            format!("generated-ai-{seed}"),
            generate_ai_log(&mut rng, 300),
        ));
    }
    inputs
}
