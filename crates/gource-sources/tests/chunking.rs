// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! The parser's output must not depend on how the input is cut into chunks:
//! one byte at a time, seven at a time, random sizes, in two pieces at every
//! possible position, or all at once.

mod common;

use common::*;
use gource_sources::{CustomLogParser, ParseStats};

type Parsed = (Vec<Commit>, ParseStats);

fn parse_with(bytes: &[u8], sizes: &[usize]) -> Parsed {
    let mut sink = Collect::default();
    let stats = parse_chunked(bytes, sizes, &mut sink);
    (sink.commits, stats)
}

/// Inputs with every awkward thing the format allows.
fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut inputs: Vec<(String, Vec<u8>)> = all_log_files()
        .into_iter()
        .map(|f| {
            let bytes = read_test_file(&f);
            (f, bytes)
        })
        .collect();

    let multibyte = "1700000001|Zo\u{eb} \u{6df1}\u{5733}|A|src/\u{6df1}\u{5733}/\u{1f332}.txt\n\
                     1700000001|Zo\u{eb} \u{6df1}\u{5733}|M|caf\u{e9}/\u{4f60}\u{597d}.rs|FF8800\n\
                     1700000002|\u{1f332}|D|\u{1f332}/\n";
    let unix = read_test_file("fixtures/custom-log/malformed-records.log");
    let crlf = String::from_utf8(unix.clone())
        .unwrap()
        .replace('\n', "\r\n");
    let mut bom = vec![0xEF, 0xBB, 0xBF];
    bom.extend_from_slice(&unix);

    let mut every_line_bom = Vec::new();
    for line in unix.split_inclusive(|&b| b == b'\n') {
        every_line_bom.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
        every_line_bom.extend_from_slice(line);
    }

    let mut cut = unix.clone();
    cut.pop(); // no newline at the end

    inputs.extend([
        ("multibyte".to_string(), multibyte.as_bytes().to_vec()),
        ("crlf".to_string(), crlf.into_bytes()),
        ("bom-first-line".to_string(), bom),
        ("bom-every-line".to_string(), every_line_bom),
        ("no-final-newline".to_string(), cut),
        (
            "invalid-utf8".to_string(),
            b"1|bad\xffuser|A|/p\xc0\x80q\n1|bad\xffuser|M|/\xe2\x82\n2|cut\xf0\x9f|A|/x\x80\x80\n"
                .to_vec(),
        ),
        (
            "iso-dates".to_string(),
            b"2010-01-02|a|A|/x\n2010-01-02 03:04:05|a|M|/x\n2010-01-02T03:04:05+05:30|b|D|/x\n\
              2010-01-02T03:04:05Z|b|A|/y\n2010-13-45T25:61:61Z|c|A|/z\n"
                .to_vec(),
        ),
        (
            "cr-oddities".to_string(),
            b"1|a|A|/x\r\r\n2|a|A|/y\r\n\r\n3|a|A|/z\r".to_vec(),
        ),
        ("only-newlines".to_string(), b"\n\n\n".to_vec()),
        (
            "one-partial-line".to_string(),
            b"1700000000|alice|A|/no-newline".to_vec(),
        ),
        ("lone-cr".to_string(), b"\r".to_vec()),
    ]);

    for seed in 1..=4u64 {
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        inputs.push((format!("generated-{seed}"), generate_log(&mut rng, 250)));
    }
    inputs
}

#[test]
fn one_byte_chunks_equal_the_whole_input() {
    for (name, bytes) in corpus() {
        assert_eq!(parse_with(&bytes, &[1]), parse_whole(&bytes), "{name}");
    }
}

#[test]
fn seven_byte_chunks_equal_the_whole_input() {
    for (name, bytes) in corpus() {
        assert_eq!(parse_with(&bytes, &[7]), parse_whole(&bytes), "{name}");
    }
}

#[test]
fn small_odd_chunks_equal_the_whole_input() {
    for (name, bytes) in corpus() {
        let whole = parse_whole(&bytes);
        for size in [2, 3, 5, 8, 9, 16, 63, 64, 65, 1000] {
            assert_eq!(parse_with(&bytes, &[size]), whole, "{name} / {size}");
        }
    }
}

#[test]
fn random_chunk_sizes_equal_the_whole_input() {
    let mut rng = Rng(0xC0FFEE);
    for (name, bytes) in corpus() {
        let whole = parse_whole(&bytes);
        for _ in 0..12 {
            let sizes: Vec<usize> = (0..32)
                .map(|_| match rng.below(4) {
                    0 => 1,
                    1 => 1 + rng.below(8),
                    2 => 1 + rng.below(100),
                    _ => 1 + rng.below(5000),
                })
                .collect();
            assert_eq!(parse_with(&bytes, &sizes), whole, "{name} / {sizes:?}");
        }
    }
}

#[test]
fn every_two_piece_split_equals_the_whole_input() {
    for (name, bytes) in corpus() {
        if bytes.len() > 700 {
            continue;
        }
        let whole = parse_whole(&bytes);
        for at in 0..=bytes.len() {
            let mut sink = Collect::default();
            let mut parser = CustomLogParser::new();
            parser.push(&bytes[..at], &mut sink);
            parser.push(&bytes[at..], &mut sink);
            parser.finish(&mut sink);
            assert_eq!(
                (sink.commits, parser.stats()),
                whole,
                "{name} split at {at}"
            );
        }
    }
}

#[test]
fn every_three_piece_split_of_awkward_lines_equals_the_whole_input() {
    // CR/LF, a BOM, and multi-byte sequences, cut at every pair of positions.
    let inputs: [&[u8]; 3] = [
        b"\xef\xbb\xbf1|\xc3\xab\xe6\xb7\xb1|A|/\xf0\x9f\x8c\xb2\r\n2|b|M|/x\r\n",
        b"1|a|A|/x\r\r\n2|a|A|/y\n\n3|a|A|/z\r",
        b"1|bad\xffuser|A|/p\xe2\x82|ff00AA\n",
    ];
    for bytes in inputs {
        let whole = parse_whole(bytes);
        for a in 0..=bytes.len() {
            for b in a..=bytes.len() {
                let mut sink = Collect::default();
                let mut parser = CustomLogParser::new();
                parser.push(&bytes[..a], &mut sink);
                parser.push(&bytes[a..b], &mut sink);
                parser.push(&bytes[b..], &mut sink);
                parser.finish(&mut sink);
                assert_eq!(
                    (sink.commits, parser.stats()),
                    whole,
                    "{:?} split at {a}/{b}",
                    String::from_utf8_lossy(bytes)
                );
            }
        }
    }
}

#[test]
fn empty_chunks_are_harmless() {
    for (name, bytes) in corpus() {
        let whole = parse_whole(&bytes);
        let mut sink = Collect::default();
        let mut parser = CustomLogParser::new();
        parser.push(&[], &mut sink);
        for chunk in bytes.chunks(11) {
            parser.push(chunk, &mut sink);
            parser.push(&[], &mut sink);
        }
        parser.finish(&mut sink);
        parser.finish(&mut sink);
        assert_eq!((sink.commits, parser.stats()), whole, "{name}");
    }
}

#[test]
fn commits_become_available_before_the_input_ends() {
    // A commit is emitted once the next line shows it is complete, so a
    // streaming consumer sees commits while the rest of the log is still in
    // flight (the web client starts playing before the download finishes).
    let log = b"1|a|A|/x\n1|a|A|/y\n2|b|A|/z\n3|c|A|/w\n";
    let mut sink = Collect::default();
    let mut parser = CustomLogParser::new();
    parser.push(&log[..19], &mut sink); // through "1|a|A|/y\n" plus nothing more
    assert!(
        sink.commits.is_empty(),
        "the last line may still extend the commit"
    );
    parser.push(&log[19..28], &mut sink); // "2|b|A|/z\n" completes commit 1
    assert_eq!(sink.commits.len(), 1);
    assert_eq!(sink.commits[0].files.len(), 2);
    parser.push(&log[28..], &mut sink);
    assert_eq!(sink.commits.len(), 2);
    parser.finish(&mut sink);
    assert_eq!(sink.commits.len(), 3);
}
