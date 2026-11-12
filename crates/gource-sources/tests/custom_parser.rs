// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Behaviour of [`CustomLogParser`] on small hand-written inputs. The
//! expectations follow `formats/custom.cpp`, `formats/commitlog.cpp` and
//! `core/seeklog.cpp`; the same rules were also cross-checked against the
//! reference code on millions of generated inputs (harness in
//! `crates/gource-sources/tests/cpp-reference/`).

mod common;

use common::*;
use gource_core::events::ChangeKind::{self, Add, Delete, Modify};
use gource_sources::CustomLogParser;

fn commits(input: &[u8]) -> Vec<Commit> {
    parse_whole(input).0
}

type Shape = (i64, String, Vec<(ChangeKind, String)>);

fn shape(input: &[u8]) -> Vec<Shape> {
    commits(input)
        .into_iter()
        .map(|c| {
            (
                c.timestamp,
                c.username,
                c.files.into_iter().map(|(k, p, _)| (k, p)).collect(),
            )
        })
        .collect()
}

#[test]
fn consecutive_lines_with_same_timestamp_and_user_form_one_commit() {
    let got = shape(b"10|a|A|/x\n10|a|M|/y\n10|a|D|/z\n11|a|A|/w\n");
    assert_eq!(
        got,
        vec![
            (
                10,
                "a".to_string(),
                vec![
                    (Add, "/x".to_string()),
                    (Modify, "/y".to_string()),
                    (Delete, "/z".to_string())
                ]
            ),
            (11, "a".to_string(), vec![(Add, "/w".to_string())]),
        ]
    );
}

#[test]
fn grouping_is_only_between_neighbours() {
    // a, b, a with equal timestamps: three commits, in source order
    let got = shape(b"5|a|A|/1\n5|b|A|/2\n5|a|A|/3\n");
    assert_eq!(got.len(), 3);
    assert_eq!(got[0].1, "a");
    assert_eq!(got[1].1, "b");
    assert_eq!(got[2].1, "a");
    // and the same user with a different timestamp in between likewise
    assert_eq!(shape(b"5|a|A|/1\n6|a|A|/2\n5|a|A|/3\n").len(), 3);
}

#[test]
fn a_malformed_line_ends_the_commit_in_progress() {
    // Same timestamp and user on both sides of the bad line: two commits.
    let (cs, stats) = parse_whole(b"7|a|A|/x\nnonsense\n7|a|A|/y\n");
    assert_eq!(cs.len(), 2);
    assert_eq!(cs[0].files.len(), 1);
    assert_eq!(cs[1].files.len(), 1);
    assert_eq!(stats.skipped_lines, 1);
    assert_eq!(stats.lines, 3);

    // Each way a line can be malformed does that.
    for bad in [
        &b""[..],
        b"no pipes",
        b"7|a|X|/x",
        b"7|a|A|",
        b"|a|A|/x",
        b"7|a|A",
        b"abc|a|A|/x",
        b"00|a|A|/x",
        b"2010-1-1|a|A|/x",
        b"not-a-timestamp|a|A|/x",
    ] {
        let mut input = b"7|a|A|/x\n".to_vec();
        input.extend_from_slice(bad);
        input.extend_from_slice(b"\n7|a|A|/y\n");
        let (cs, stats) = parse_whole(&input);
        assert_eq!(cs.len(), 2, "{:?}", String::from_utf8_lossy(bad));
        assert_eq!(stats.skipped_lines, 1, "{:?}", String::from_utf8_lossy(bad));
    }
}

#[test]
fn a_blank_line_is_malformed_but_the_missing_last_line_is_not() {
    let (cs, stats) = parse_whole(b"7|a|A|/x\n\n7|a|A|/y\n");
    assert_eq!(cs.len(), 2);
    assert_eq!((stats.lines, stats.skipped_lines), (3, 1));

    // "x\n" is one line; there is no empty line after the final newline.
    let (_, stats) = parse_whole(b"7|a|A|/x\n");
    assert_eq!((stats.lines, stats.skipped_lines), (1, 0));
    // "x\n\n" has a real empty line after the first
    let (_, stats) = parse_whole(b"7|a|A|/x\n\n");
    assert_eq!((stats.lines, stats.skipped_lines), (2, 1));
}

#[test]
fn leading_garbage_does_not_stop_the_parse() {
    let got = shape(b"garbage\nmore garbage\n1|a|A|/x\n");
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].0, 1);
}

#[test]
fn defaults_for_empty_username_and_action() {
    let got = shape(b"1||A|/x\n1|Unknown||/y\n");
    // empty user is "Unknown", so both lines are the same commit
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].1, "Unknown");
    assert_eq!(
        got[0].2,
        vec![(Add, "/x".to_string()), (Add, "/y".to_string())]
    );
}

#[test]
fn paths_are_rooted_and_directories_keep_their_trailing_slash() {
    let got = shape(b"1|a|A|rel/file\n1|a|A|/abs/file\n1|a|D|dir/\n1|a|D|/\n1|a|A|x\n");
    let paths: Vec<&str> = got[0].2.iter().map(|(_, p)| p.as_str()).collect();
    assert_eq!(paths, ["/rel/file", "/abs/file", "/dir/", "/", "/x"]);
}

#[test]
fn text_is_filtered_like_utf8cpp_with_a_question_mark() {
    let (cs, _) = parse_whole(b"1|bad\xffuser|A|\xff\n1|bad\xffuser|A|a\xc0\x80b\n1|bad\xffuser|A|/\x80\x80\x80\n1|bad\xffuser|A|\xe2\x82\n");
    assert_eq!(cs.len(), 1);
    assert_eq!(cs[0].username, "bad?user");
    let paths: Vec<&str> = cs[0].files.iter().map(|(_, p, _)| p.as_str()).collect();
    // lone invalid byte -> '?', rooted; an overlong pair is one '?';
    // stray continuation bytes are one '?' each; a truncated tail is one '?'
    assert_eq!(paths, ["/?", "/a?b", "/???", "/?"]);
}

#[test]
fn commits_are_grouped_on_the_raw_username_not_the_filtered_one() {
    // Both names filter to "a?", but they differ byte for byte.
    let (cs, _) = parse_whole(b"1|a\xff|A|/x\n1|a\xfe|A|/y\n1|a\xfe|A|/z\n");
    assert_eq!(cs.len(), 2);
    assert_eq!(cs[0].username, "a?");
    assert_eq!(cs[1].username, "a?");
    assert_eq!(cs[1].files.len(), 2);
}

#[test]
fn nul_bytes_are_ordinary_bytes() {
    let (cs, stats) = parse_whole(b"1|a\0b|A|/p\0q\n");
    assert_eq!(stats.skipped_lines, 0);
    assert_eq!(cs[0].username, "a\0b");
    assert_eq!(cs[0].files[0].1, "/p\0q");
}

#[test]
fn colour_column() {
    let (cs, _) = parse_whole(
        b"1|a|A|/1|ff8800\n\
          1|a|A|/2|#00AAbb\n\
          1|a|A|/3|12345\n\
          1|a|A|/4|#12345\n\
          1|a|A|/5|zzzzzz\n\
          1|a|A|/6|\n\
          1|a|A|/7|ABCDEFGHIJ\n\
          1|a|A|/8|##112233\n\
          1|a|A|/9|0A0b0C|ignored|columns\n",
    );
    let colours: Vec<Option<[u8; 3]>> = cs[0].files.iter().map(|f| f.2).collect();
    assert_eq!(
        colours,
        [
            Some([0xff, 0x88, 0x00]),
            Some([0x00, 0xaa, 0xbb]),
            None,
            None,
            None,
            None,
            Some([0xab, 0xcd, 0xef]),
            None,
            Some([0x0a, 0x0b, 0x0c]),
        ]
    );
}

#[test]
fn pipes_in_the_path_cut_it_short() {
    let (cs, stats) = parse_whole(b"1|a|A|src/has|delimiter.rs\n");
    assert_eq!(stats.skipped_lines, 0);
    assert_eq!(cs[0].files[0].1, "/src/has");
}

#[test]
fn timestamps_follow_atoll() {
    let ts = |field: &str| {
        let (cs, _) = parse_whole(format!("{field}|a|A|/x\n").as_bytes());
        cs.first().map(|c| c.timestamp)
    };
    assert_eq!(ts("0"), Some(0));
    assert_eq!(ts("00"), None);
    assert_eq!(ts("-7"), Some(-7));
    assert_eq!(ts("+7"), Some(7));
    assert_eq!(ts(" 7"), Some(7));
    assert_eq!(ts("7x"), Some(7));
    assert_eq!(ts("x7"), None);
    assert_eq!(ts("9223372036854775807"), Some(i64::MAX));
    assert_eq!(ts("99999999999999999999"), Some(i64::MAX));
    assert_eq!(ts("-99999999999999999999"), Some(i64::MIN));
    assert_eq!(ts("1700000000"), Some(1_700_000_000));
}

#[test]
fn iso_8601_timestamps() {
    let ts = |field: &str| {
        let (cs, _) = parse_whole(format!("{field}|a|A|/x\n").as_bytes());
        cs.first().map(|c| c.timestamp)
    };
    // values from running parseDateTime with TZ=UTC
    assert_eq!(ts("2010-01-02"), Some(1_262_390_400));
    assert_eq!(ts("2010-01-02Z"), Some(1_262_390_400));
    assert_eq!(ts("2010-01-02 03:04"), Some(1_262_401_440));
    assert_eq!(ts("2010-01-02 03:04:05"), Some(1_262_401_445));
    assert_eq!(ts("2010-01-01T03:04:05Z"), Some(1_262_315_045));
    assert_eq!(ts("2010-01-01T03:04:05.6789Z"), Some(1_262_315_045));
    assert_eq!(ts("2010-01-01T03:04:05+12"), Some(1_262_271_845));
    assert_eq!(ts("2010-01-01T00:04:05+5:30"), Some(1_262_284_445));
    assert_eq!(ts("2010-01-01T03:04+530"), Some(1_262_295_240));
    assert_eq!(ts("1970-01-01T00:00:00Z"), Some(0));
    assert_eq!(ts("1969-12-31T23:59:59Z"), Some(-1));
    // out-of-range fields carry, as with mktime
    assert_eq!(ts("2010-13-45T25:61:61Z"), Some(1_297_735_321));
    // not matching the date grammar
    assert_eq!(ts("2010-1-1"), None);
    assert_eq!(ts("2010-01-01T03"), None);
    assert_eq!(ts("2010-01-01t03:04"), None);
    assert_eq!(ts("5-"), None);
}

#[test]
fn dates_without_a_zone_use_the_configured_local_offset() {
    let parse = |offset: Option<i32>| {
        let mut parser = match offset {
            Some(o) => CustomLogParser::new().with_local_utc_offset(o),
            None => CustomLogParser::new(),
        };
        let mut sink = Collect::default();
        parser.push(
            b"2010-01-02 03:04:05|a|A|/x\n2010-01-02 03:04:05Z|b|A|/y\n",
            &mut sink,
        );
        parser.finish(&mut sink);
        sink.commits.iter().map(|c| c.timestamp).collect::<Vec<_>>()
    };
    // UTC unless told otherwise
    assert_eq!(parse(None), [1_262_401_445, 1_262_401_445]);
    // Seoul, as the reference produces it with TZ=Asia/Seoul: 03:04:05 local
    // is nine hours earlier in UTC. The explicit-Z line does not move.
    assert_eq!(
        parse(Some(9 * 3600)),
        [1_262_401_445 - 9 * 3600, 1_262_401_445]
    );
    assert_eq!(
        parse(Some(-5 * 3600)),
        [1_262_401_445 + 5 * 3600, 1_262_401_445]
    );
}

#[test]
fn byte_order_marks_are_skipped_on_every_line() {
    let (cs, stats) = parse_whole(b"\xef\xbb\xbf1|a|A|/x\n\xef\xbb\xbf2|b|A|/y\n3|c|A|/z\n");
    assert_eq!(cs.len(), 3);
    assert_eq!(stats.skipped_lines, 0);
    assert_eq!(
        cs.iter().map(|c| c.byte_offset).collect::<Vec<_>>(),
        [0, 3 + 9, 3 + 9 + 3 + 9]
    );
    // BOM then '|' has no timestamp
    let (cs, stats) = parse_whole(b"\xef\xbb\xbf|a|A|/x\n");
    assert!(cs.is_empty());
    assert_eq!(stats.skipped_lines, 1);
}

#[test]
fn carriage_returns() {
    // CRLF
    let (cs, _) = parse_whole(b"1|a|A|/x\r\n1|a|M|/y\r\n");
    assert_eq!(cs.len(), 1);
    assert_eq!(cs[0].files[0].1, "/x");
    assert_eq!(cs[0].files[1].1, "/y");
    // only one CR is removed
    let (cs, _) = parse_whole(b"1|a|A|/x\r\r\n");
    assert_eq!(cs[0].files[0].1, "/x\r");
    // a CR at the very end of the input counts as the line ending too
    let (cs, _) = parse_whole(b"1|a|A|/x\r");
    assert_eq!(cs[0].files[0].1, "/x");
    // a lone CR is not a line break: this is one line, whose path column
    // runs to the next '|'
    let (cs, stats) = parse_whole(b"1|a|A|/x\r2|a|A|/y\n");
    assert_eq!(stats.lines, 1);
    assert_eq!(cs[0].files[0].1, "/x\r2");
}

#[test]
fn the_last_line_needs_finish_but_not_a_newline() {
    let mut sink = Collect::default();
    let mut parser = CustomLogParser::new();
    parser.push(b"1|a|A|/x\n2|b|A|/y", &mut sink);
    // "2|b|A|/y" is still a partial line and the first commit is waiting
    // for a line that ends it.
    assert_eq!(sink.commits.len(), 0);
    parser.finish(&mut sink);
    assert_eq!(sink.commits.len(), 2);
    assert_eq!(sink.commits[1].timestamp, 2);
    assert_eq!(sink.commits[1].byte_offset, 9);
    assert_eq!(parser.stats().bytes, 17);
}

#[test]
fn byte_offsets_with_crlf_and_multibyte_text() {
    let log = "1|\u{e9}|A|/x\r\n2|\u{e9}|A|/y\r\n3|b|A|/z";
    let (cs, _) = parse_whole(log.as_bytes());
    let first_line = "1|\u{e9}|A|/x\r\n".len() as u64;
    let second_line = "2|\u{e9}|A|/y\r\n".len() as u64;
    assert_eq!(
        cs.iter().map(|c| c.byte_offset).collect::<Vec<_>>(),
        [0, first_line, first_line + second_line]
    );
}

#[test]
fn commit_offset_is_its_first_valid_line() {
    // lines 0 and 1 are junk, the commit starts at line 2
    let log = b"junk\nmore junk\n5|a|A|/x\n5|a|A|/y\n";
    let (cs, _) = parse_whole(log);
    assert_eq!(cs.len(), 1);
    assert_eq!(cs[0].byte_offset, 5 + 10);
}

#[test]
fn stats_add_up() {
    let log = b"1|a|A|/x\n1|a|M|/y\nbad\n2|b|D|/z\n\n3|c|A|/w";
    let mut sink = Collect::default();
    let mut parser = CustomLogParser::new();
    parser.push(log, &mut sink);
    parser.finish(&mut sink);
    let s = parser.stats();
    assert_eq!(s.bytes, log.len() as u64);
    assert_eq!(s.lines, 6);
    assert_eq!(s.skipped_lines, 2);
    assert_eq!(s.commits, 3);
    assert_eq!(s.files, 4);
    assert_eq!(s.commits as usize, sink.commits.len());
}

#[test]
fn the_parser_can_continue_after_finish() {
    let mut sink = Collect::default();
    let mut parser = CustomLogParser::new();
    parser.push(b"1|a|A|/x\n", &mut sink);
    parser.finish(&mut sink);
    parser.push(b"2|b|A|/y\n", &mut sink);
    parser.finish(&mut sink);
    assert_eq!(sink.commits.len(), 2);
    assert_eq!(sink.commits[1].byte_offset, 9);
}

#[test]
fn empty_input_and_finish_without_input() {
    let mut sink = Collect::default();
    let mut parser = CustomLogParser::new();
    parser.finish(&mut sink);
    parser.push(b"", &mut sink);
    parser.finish(&mut sink);
    assert!(sink.commits.is_empty());
    assert_eq!(parser.stats(), Default::default());
}

#[test]
fn never_panics_on_arbitrary_bytes() {
    let mut rng = Rng(0xDEAD_BEEF);
    const ALPHABET: &[u8] = b"|\n\r AMD0123-+#fF:.TZ/\xEF\xBB\xBF\x80\xFF\xC3\xA9\0";
    for _ in 0..3000 {
        let len = rng.below(200);
        let bytes: Vec<u8> = (0..len).map(|_| *rng.pick(ALPHABET)).collect();
        let (cs, stats) = parse_whole(&bytes);
        assert_eq!(stats.commits as usize, cs.len());
        for c in &cs {
            assert!(!c.files.is_empty());
            for (_, path, _) in &c.files {
                assert!(path.starts_with('/'));
            }
        }
    }
}
