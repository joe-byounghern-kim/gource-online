// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! The repository's custom-log inputs (`tests/fixtures/custom-log` and
//! `tests/logs`) through [`CustomLogParser`].
//!
//! Every expectation below was produced by running the reference C++ logic
//! (`formats/custom.cpp`, `formats/commitlog.cpp`, `core/seeklog.cpp`, PCRE2
//! and utf8cpp) over the same bytes. The harness is in
//! `crates/gource-sources/tests/cpp-reference/`.

mod common;

use common::*;
use gource_sources::ParseStats;

struct Expect {
    file: &'static str,
    /// Commits in `render` format.
    commits: &'static str,
    /// lines, skipped lines, commits, files
    stats: (u64, u64, u64, u64),
}

const CUSTOM: &[Expect] = &[
    Expect {
        file: "fixtures/custom-log/cancellation.log",
        commits: "\
1700000900 alice
  A /src/before-cancel.rs
1700000901 bob
  A /src/cancel-point.rs
1700000902 carol
  A /src/after-cancel.rs
",
        stats: (3, 0, 3, 3),
    },
    Expect {
        file: "fixtures/custom-log/deep-unicode-unusual.log",
        commits: "\
1700000600 Zoë
  A /src/深い/階層/данные/مرحبا/文件.txt
1700000601 Zoë
  A /src/深い/階層/данные/مرحبا/space name [v1] @.md
1700000602 Miyuki
  M /src/深い/階層/данные/مرحبا/space name [v1] @.md
1700000603 Miyuki
  A /assets/emoji-🌲/café+tea.txt
1700000604 Miyuki
  D /assets/emoji-🌲/café+tea.txt
",
        stats: (5, 0, 5, 5),
    },
    Expect {
        file: "fixtures/custom-log/deletion-recreation.log",
        commits: "\
1700000500 alice
  A /src/config.toml
1700000501 alice
  D /src/config.toml
1700000502 bob
  A /src/config.toml
1700000503 bob
  M /src/config.toml
",
        stats: (4, 0, 4, 4),
    },
    Expect {
        file: "fixtures/custom-log/dense-contributors.log",
        commits: "\
1700000700 alice
  A /src/a.rs
1700000700 bob
  A /src/b.rs
1700000700 carol
  A /src/c.rs
1700000701 dave
  A /src/d.rs
1700000701 erin
  A /src/e.rs
1700000701 frank
  A /src/f.rs
1700000702 alice
  M /src/a.rs
1700000702 bob
  M /src/b.rs
1700000702 carol
  M /src/c.rs
1700000703 dave
  D /src/d.rs
1700000703 erin
  M /src/e.rs
1700000703 frank
  D /src/f.rs
",
        stats: (12, 0, 12, 12),
    },
    // Three identical lines are one commit with three files: nothing is
    // de-duplicated.
    Expect {
        file: "fixtures/custom-log/duplicates.log",
        commits: "\
1700000200 alice
  A /src/a.rs
  A /src/a.rs
  A /src/a.rs
1700000201 bob
  M /src/a.rs
  M /src/a.rs
",
        stats: (5, 0, 2, 5),
    },
    Expect {
        file: "fixtures/custom-log/empty.log",
        commits: "",
        stats: (0, 0, 0, 0),
    },
    Expect {
        file: "fixtures/custom-log/encoder-failure.log",
        commits: "\
1700000950 alice
  A /frames/one.rs
1700000951 alice
  M /frames/one.rs
1700000952 bob
  A /frames/two.rs
",
        stats: (3, 0, 3, 3),
    },
    // Equal timestamps do not merge commits of different users, and the
    // alternating users stay separate commits in source order.
    Expect {
        file: "fixtures/custom-log/equal-timestamps-source-order.log",
        commits: "\
1700000100 zeta
  A /src/zeta.rs
1700000100 alpha
  A /src/alpha.rs
1700000100 zeta
  M /src/zeta.rs
1700000100 alpha
  D /src/alpha.rs
1700000101 alpha
  A /docs/index.md
",
        stats: (5, 0, 5, 5),
    },
    Expect {
        file: "fixtures/custom-log/long-idle.log",
        commits: "\
1700000400 alice
  A /src/start.rs
1700000402 bob
  M /src/start.rs
1700004002 alice
  A /src/resume.rs
1700004003 bob
  D /src/start.rs
",
        stats: (4, 0, 4, 4),
    },
    // Four lines are rejected (bad timestamp, unknown action, empty path,
    // empty timestamp); `src/has|delimiter.rs` is the file `/src/has`; an
    // empty user is `Unknown`; an empty action is `A`; short colours are
    // ignored rather than rejected.
    Expect {
        file: "fixtures/custom-log/malformed-records.log",
        commits: "\
1700000800 valid
  A /ok/file.rs
1700000803 alice
  A /src/has
1700000804 Unknown
  A /src/default-user.rs
1700000805 alice
  A /src/file.rs
1700000806 alice
  A /src/default-action.rs
1700000807 alice
  A /src/file.rs
",
        stats: (10, 4, 6, 6),
    },
    Expect {
        file: "fixtures/custom-log/single-event.log",
        commits: "\
1700000000 alice
  A /README.md
",
        stats: (1, 0, 1, 1),
    },
    // Out-of-order timestamps are kept as they are.
    Expect {
        file: "fixtures/custom-log/unordered-timestamps.log",
        commits: "\
1700000302 carol
  M /app/main.rs
1700000300 alice
  A /app/main.rs
1700000301 bob
  A /app/lib.rs
1700000300 alice
  M /README.md
1700000302 carol
  D /app/lib.rs
",
        stats: (5, 0, 5, 5),
    },
    Expect {
        file: "fixtures/custom-log/visual-hierarchy-activity.log",
        commits: "\
1700001000 Ada Lovelace
  A /src/main.rs
1700001000 Grace Hopper
  A /src/lib/parser.rs
1700001001 Ada Lovelace
  M /src/main.rs
1700001002 Linus Torvalds
  A /tests/unit/parser_test.rs
1700001003 Grace Hopper
  D /src/lib/parser.rs
1700001063 Ada Lovelace
  A /docs/README.md
1700001063 Linus Torvalds
  M /src/main.rs
1700001064 Linus Torvalds
  A /tools/bench/driver.rs
",
        stats: (8, 0, 8, 8),
    },
    // `0` is accepted (but not `00`), negative values are fine.
    Expect {
        file: "fixtures/custom-log/zero-negative-timestamps.log",
        commits: "\
0 zero
  A /zero.txt
-2 negative
  A /negative/two.txt
-1 negative
  M /negative/two.txt
0 zero
  D /zero.txt
1 positive
  A /positive.txt
",
        stats: (5, 0, 5, 5),
    },
    // A directory delete keeps its trailing slash for the simulation.
    Expect {
        file: "logs/custom-dir-delete.log",
        commits: "\
1277787455 bob
  A /path/to/1.txt
1277787457 bob
  A /path/to/blah/1.txt
  A /path/to/blah/2.txt
  A /path/to/blah/3.txt
1277787460 bob
  D /path/to/
",
        stats: (5, 0, 3, 5),
    },
    // The file ends with an empty line, which is a malformed line.
    Expect {
        file: "logs/file-removal.log",
        commits: "\
1277787455 bob
  A /path/to/file.1
  A /path/to/file.2
  A /path/to/file.3
  A /path/to/file.4
  A /path/to/file.5
1277787456 bob
  D /path/to/file.1
  D /path/to/file.2
  D /path/to/file.3
  D /path/to/file.4
1277787466 bob
  D /path/to/file.5
",
        stats: (11, 1, 3, 10),
    },
    Expect {
        file: "logs/file-to-dir.log",
        commits: "\
1277787456 bob
  A /path/to/file/Not
1277787457 bob
  A /path/to/file/Not/Not
  A /path/to/file/Not/Not/Not
1277787459 bob
  A /path/to/file/Not/Not/Not/File
",
        stats: (4, 0, 3, 4),
    },
    Expect {
        file: "logs/utf8.log",
        commits: "\
1277787455 Árvíztűrő tükörfúrógép
  A /Árvíztűrő/tükörfúrógép/Árvíztűrő tükörfúrógép.txt
1277787465 Árvíztűrő tükörfúrógép
  D /Árvíztűrő/tükörfúrógép/Árvíztűrő tükörfúrógép.txt
",
        stats: (3, 1, 2, 2),
    },
];

/// Not custom logs: an SVN XML log and a caption file (`timestamp|text`).
/// The custom format rejects every line of them.
const NOT_CUSTOM: &[&str] = &["logs/svn-dir-delete.log", "logs/utf8-caption.log"];

fn stats_tuple(s: &ParseStats) -> (u64, u64, u64, u64) {
    (s.lines, s.skipped_lines, s.commits, s.files)
}

#[test]
fn every_custom_fixture_matches_the_reference() {
    for case in CUSTOM {
        let bytes = read_test_file(case.file);
        let (commits, stats) = parse_whole(&bytes);
        assert_eq!(render(&commits), case.commits, "{}", case.file);
        assert_eq!(stats_tuple(&stats), case.stats, "{}", case.file);
        assert_eq!(stats.bytes, bytes.len() as u64, "{}", case.file);
    }
}

#[test]
fn listed_files_exist_and_logs_added_later_still_parse_sanely() {
    let covered: Vec<&str> = CUSTOM
        .iter()
        .map(|c| c.file)
        .chain(NOT_CUSTOM.iter().copied())
        .collect();
    for file in &covered {
        assert!(
            repo_root().join("tests").join(file).exists(),
            "{file} is listed but missing"
        );
    }

    // A log somebody adds to these directories without an entry above still
    // has to satisfy the invariants that need no expectation (add an `Expect`
    // to pin its commits down; chunking.rs and batch.rs already include it).
    for file in all_log_files() {
        if covered.contains(&file.as_str()) {
            continue;
        }
        let bytes = read_test_file(&file);
        let (commits, stats) = parse_whole(&bytes);
        assert_eq!(stats.commits as usize, commits.len(), "{file}");
        assert_eq!(stats.bytes, bytes.len() as u64, "{file}");
        let mut previous = None;
        for c in &commits {
            assert!(!c.username.is_empty(), "{file}");
            assert!(!c.files.is_empty(), "{file}");
            assert!(c.files.iter().all(|f| f.1.starts_with('/')), "{file}");
            assert!(c.byte_offset < bytes.len() as u64, "{file}");
            assert!(previous < Some(c.byte_offset), "{file}");
            previous = Some(c.byte_offset);
        }
    }
}

#[test]
fn non_custom_logs_yield_nothing_and_skip_every_line() {
    for file in NOT_CUSTOM {
        let bytes = read_test_file(file);
        let (commits, stats) = parse_whole(&bytes);
        assert!(commits.is_empty(), "{file}");
        assert_eq!(stats.commits, 0, "{file}");
        assert!(stats.lines > 0, "{file}");
        assert_eq!(stats.lines, stats.skipped_lines, "{file}");
    }
}

#[test]
fn byte_offsets_point_at_the_first_line_of_each_commit() {
    for case in CUSTOM {
        let bytes = read_test_file(case.file);
        let (commits, _) = parse_whole(&bytes);
        let mut previous = None;
        for (i, c) in commits.iter().enumerate() {
            let at = c.byte_offset as usize;
            assert!(
                at == 0 || bytes[at - 1] == b'\n',
                "{} commit {i}: offset {at} is not at a line start",
                case.file
            );
            let line_end = bytes[at..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(bytes.len(), |p| at + p);
            let line = std::str::from_utf8(&bytes[at..line_end]).unwrap();
            let (_, first_path) = (&c.files[0].0, &c.files[0].1);
            assert!(
                line.starts_with(&format!("{}|", c.timestamp)),
                "{} commit {i}: line {line:?} does not start with its timestamp",
                case.file
            );
            assert!(
                line.contains(first_path.trim_start_matches('/')),
                "{} commit {i}: line {line:?} lacks {first_path}",
                case.file
            );
            if let Some(prev) = previous {
                assert!(c.byte_offset > prev, "{} commit {i}", case.file);
            }
            previous = Some(c.byte_offset);
        }
    }
}

#[test]
fn byte_offsets_are_exact_across_malformed_lines() {
    // Commits start at lines 0, 4, 5, 6, 7 and 9 of this file.
    let bytes = read_test_file("fixtures/custom-log/malformed-records.log");
    let mut line_starts = vec![0u64];
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\n' {
            line_starts.push(i as u64 + 1);
        }
    }
    let (commits, _) = parse_whole(&bytes);
    let offsets: Vec<u64> = commits.iter().map(|c| c.byte_offset).collect();
    let expected: Vec<u64> = [0, 4, 5, 6, 7, 9].iter().map(|&i| line_starts[i]).collect();
    assert_eq!(offsets, expected);
    assert_eq!(offsets[0], 0);
    assert_eq!(offsets[1], 128);
}
