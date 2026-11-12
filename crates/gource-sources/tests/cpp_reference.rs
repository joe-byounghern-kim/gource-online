// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! The parser against recorded output of Gource's own code.
//!
//! `tests/cpp-reference/cases.txt` lists inputs (the repository's logs,
//! hand-written edge cases for every rule of `custom.cpp`, ISO dates, UTF-8
//! damage, and noisy inputs found by a differential fuzzer).
//! `tests/cpp-reference/expected.txt` is what the C++ reference logic made of
//! them (`harness.cpp`, built by `regen.sh` from the real `core/regex.cpp`,
//! PCRE2, utf8cpp and `parseDateTime`). No C++ is needed to run this test;
//! add a case to `cases.txt` and run `regen.sh` to record one.

mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;

use common::*;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/cpp-reference")
}

/// `\n \r \t \0 \\ \xNN`; everything else is literal.
fn unescape(text: &str) -> Vec<u8> {
    let raw = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        if raw[i] != b'\\' {
            out.push(raw[i]);
            i += 1;
            continue;
        }
        match raw[i + 1] {
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'0' => out.push(0),
            b'\\' => out.push(b'\\'),
            b'x' => {
                let hex = std::str::from_utf8(&raw[i + 2..i + 4]).unwrap();
                out.push(u8::from_str_radix(hex, 16).unwrap());
                i += 2;
            }
            other => panic!("bad escape \\{} in {text:?}", other as char),
        }
        i += 2;
    }
    out
}

fn from_hex(digits: &str) -> Vec<u8> {
    (0..digits.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&digits[i..i + 2], 16).unwrap())
        .collect()
}

fn load(spec: &str) -> Vec<u8> {
    match spec.split_once(':') {
        Some(("file", path)) => read_test_file(path),
        Some(("str", text)) => unescape(text),
        Some(("hex", digits)) => from_hex(digits),
        _ => panic!("bad case spec {spec:?}"),
    }
}

fn cases() -> Vec<(String, Vec<u8>)> {
    let text = std::fs::read_to_string(dir().join("cases.txt")).unwrap();
    text.lines()
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| {
            let (name, spec) = l
                .split_once('\t')
                .unwrap_or_else(|| panic!("bad line {l:?}"));
            (name.to_string(), load(spec))
        })
        .collect()
}

fn expected() -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(dir().join("expected.txt")).unwrap();
    let mut map = BTreeMap::new();
    let mut name = None::<String>;
    let mut body = String::new();
    for line in text.split_inclusive('\n') {
        if let Some(n) = line.strip_prefix("@ ") {
            if let Some(prev) = name.take() {
                map.insert(prev, std::mem::take(&mut body));
            }
            name = Some(n.trim_end().to_string());
        } else {
            body.push_str(line);
        }
    }
    if let Some(prev) = name {
        map.insert(prev, body);
    }
    map
}

#[test]
fn every_case_has_a_recorded_result_and_vice_versa() {
    let cases = cases();
    let expected = expected();
    assert!(cases.len() > 200, "{} cases", cases.len());
    let mut names: Vec<&str> = cases.iter().map(|(n, _)| n.as_str()).collect();
    let listed = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), listed, "duplicate case names");
    assert_eq!(
        names,
        expected.keys().map(String::as_str).collect::<Vec<_>>(),
        "cases.txt and expected.txt are out of step: run regen.sh"
    );
}

#[test]
fn matches_the_cpp_reference_on_every_recorded_case() {
    let expected = expected();
    let mut interesting = 0;
    for (name, input) in cases() {
        let want = &expected[&name];

        let (commits, stats) = parse_whole(&input);
        assert_eq!(&canonical(&commits, &stats), want, "{name}");
        interesting += usize::from(!commits.is_empty());

        // and in pieces
        for sizes in [&[1][..], &[7], &[64], &[3, 1, 11, 2]] {
            let mut sink = Collect::default();
            let stats = parse_chunked(&input, sizes, &mut sink);
            assert_eq!(
                &canonical(&sink.commits, &stats),
                want,
                "{name} / {sizes:?}"
            );
        }
    }
    assert!(interesting > 150, "{interesting} cases produced commits");
}

/// The record without its byte offsets, which a meta field shifts.
fn without_offsets(body: &str) -> String {
    body.lines()
        .map(|l| match l.strip_prefix("C ") {
            Some(rest) => format!("C {}", rest.rsplit_once(' ').unwrap().0),
            None => l.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// C++ Gource reads a log with the AI meta field exactly as the same log
/// without it (F1 AI-19): every `meta-*` case and its `-plain` twin, and
/// the AI fixture and its stripped copy, give the same commits.
#[test]
fn the_cpp_reference_ignores_the_meta_field() {
    let expected = expected();
    let mut pairs: Vec<(String, String)> = expected
        .keys()
        .filter(|n| n.starts_with("meta-") && !n.ends_with("-plain"))
        .map(|n| (n.clone(), format!("{n}-plain")))
        .collect();
    assert!(pairs.len() >= 8, "{pairs:?}");
    pairs.push((
        "file-fixtures-ai-coauthors".into(),
        "file-fixtures-ai-coauthors-stripped".into(),
    ));
    for (with, plain) in pairs {
        let (a, b) = (&expected[&with], &expected[&plain]);
        assert_eq!(without_offsets(a), without_offsets(b), "{with}");
        assert!(a.lines().any(|l| l.starts_with("C ")), "{with} has commits");
    }
}
