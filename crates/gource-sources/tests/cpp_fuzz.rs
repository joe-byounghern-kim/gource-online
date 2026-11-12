// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Differential fuzzing against the C++ reference logic.
//!
//! Generates noisy custom logs (random bytes, damaged UTF-8, odd timestamps
//! and dates, byte order marks, carriage returns, malformed lines, colour
//! columns) and checks that [`CustomLogParser`] produces exactly what the
//! reference harness in `tests/cpp-reference/` produces for them, commit by
//! commit, byte offset included. The parser is fed whole and in random
//! chunks.
//!
//! Needs the compiled harness, so it is ignored by default:
//!
//! ```text
//! GOURCE_SRC=/path/to/Gource/src crates/gource-sources/tests/cpp-reference/regen.sh build
//! CPP_HARNESS=target/cpp-reference/harness \
//!     cargo test -p gource-sources --test cpp_fuzz -- --ignored --nocapture
//! ```
//!
//! `FUZZ_CASES` (default 20000) and `FUZZ_SEED` (default 1) tune the run. The
//! harness runs with `TZ=UTC`, matching the parser's default for dates
//! without a zone.

mod common;

use std::io::Write;
use std::process::{Command, Stdio};

use common::*;

const NASTY_BYTES: &[u8] = &[
    b'|', b'|', b'|', b'\n', b'\r', b'A', b'D', b'M', b'X', b'a', b'1', b'0', b'9', b'-', b'+',
    b'#', b'f', b'F', b'g', b'Z', b'T', b' ', b':', b'.', b'/', 0xEF, 0xBB, 0xBF, 0x80, 0xBF, 0xC0,
    0xC3, 0xA9, 0xE2, 0x82, 0xAC, 0xF0, 0x9F, 0xFF, 0xF4, 0x90, 0xED, 0xA0, 0, b't', b'e',
];

fn without_separators(mut bytes: Vec<u8>) -> Vec<u8> {
    bytes.retain(|&b| b != b'|' && b != b'\n');
    bytes
}

fn noise(r: &mut Rng) -> Vec<u8> {
    let n = r.below(60);
    (0..n).map(|_| *r.pick(NASTY_BYTES)).collect()
}

fn username(r: &mut Rng) -> Vec<u8> {
    const POOL: &[&[u8]] = &[
        b"alice",
        b"bob",
        b"",
        b"Ada Lovelace",
        b"Zo\xc3\xab",
        b"bad\xffname",
        b"cut\xe2\x82",
        b"x\x80y\x80\x80z",
        b"\xc0\x80",
        b"\xed\xa0\x80",
        b"\xf4\x90\x80\x80",
        b"\xf0\x9f\x8c\xb2",
        b"a\x00b",
        b"Unknown",
        b"\xef\xbb\xbfbom",
        b"tab\there",
        b" lead",
        b"trail ",
        b"\xc3",
        b"\xe2\x82\xe2\x82\xac",
        b"\xf8\x88\x80\x80\x80",
    ];
    if r.below(4) == 0 {
        without_separators(noise(r))
    } else {
        r.pick(POOL).to_vec()
    }
}

fn path(r: &mut Rng) -> Vec<u8> {
    const POOL: &[&[u8]] = &[
        b"src/main.rs",
        b"/abs/path",
        b"x",
        b"/",
        b"//",
        b"dir/",
        b"a.b.c",
        b"/.hidden",
        b"bad/\xffpath",
        b"\xff",
        b"\xc0\x80/x",
        b"caf\xc3\xa9/\xe6\xb7\xb1",
        b"\x80\x80",
        b"a\x00b",
        b"\xef\xbb\xbf/bom",
        b"sp ace/file name",
        b"trail\r",
        b"\xe2\x82",
        b"#hash",
        b" lead",
    ];
    if r.below(4) == 0 {
        let v = without_separators(noise(r));
        if v.is_empty() { b"p".to_vec() } else { v }
    } else {
        r.pick(POOL).to_vec()
    }
}

fn digits(r: &mut Rng, n: usize) -> String {
    (0..n).map(|_| (b'0' + r.below(10) as u8) as char).collect()
}

/// A date string near the grammar of `parseDateTime`, often broken.
fn date(r: &mut Rng) -> Vec<u8> {
    const ZONES: &[&str] = &[
        "",
        "",
        "",
        "Z",
        "z",
        "+12",
        "-05",
        "+5:30",
        "+0530",
        "+530",
        "+123",
        "+1234",
        "+12345",
        "-1:2",
        "+",
        "-",
        " Z",
        " +12",
        "  +12",
        "+12:30",
        "-12:3",
        "+1:30",
        "+01:3",
        "+5",
        "-5",
        "+00",
        "+0",
        " -0500",
        "-5:30",
        "+99:99",
        "+12:",
        "Z ",
        "ZZ",
        "+1 2",
        "+12:30:00",
        "+123:30",
        "+0:0",
        "+00:0",
    ];

    let mut s = String::new();
    let year_len = if r.chance(90) { 4 } else { 1 + r.below(6) };
    s.push_str(&digits(r, year_len));
    s.push(if r.chance(95) {
        '-'
    } else {
        *r.pick(&['/', '.', ' ', '\u{2212}'])
    });
    let month_len = if r.chance(90) { 2 } else { r.below(4) };
    if month_len == 2 && r.chance(80) {
        s.push_str(&format!("{:02}", 1 + r.below(12)));
    } else {
        s.push_str(&digits(r, month_len));
    }
    s.push('-');
    let day_len = if r.chance(90) { 2 } else { r.below(4) };
    if day_len == 2 && r.chance(80) {
        s.push_str(&format!("{:02}", 1 + r.below(31)));
    } else {
        s.push_str(&digits(r, day_len));
    }
    if r.chance(70) {
        s.push(*r.pick(&['T', 'T', ' ', ' ', 't', '_']));
        let hour_len = *r.pick(&[1, 2, 2, 2, 2, 3, 0]);
        s.push_str(&digits(r, hour_len));
        s.push(if r.chance(95) { ':' } else { ';' });
        let minute_len = if r.chance(90) { 2 } else { r.below(4) };
        s.push_str(&digits(r, minute_len));
        if r.chance(70) {
            s.push(':');
            let second_len = if r.chance(90) { 2 } else { r.below(4) };
            s.push_str(&digits(r, second_len));
            if r.chance(30) {
                s.push('.');
                let fraction_len = r.below(5);
                s.push_str(&digits(r, fraction_len));
            }
        }
    }
    s.push_str(r.pick(ZONES));

    let mut bytes = s.into_bytes();
    if r.chance(30) && !bytes.is_empty() {
        let i = r.below(bytes.len());
        match r.below(3) {
            0 => {
                bytes.remove(i);
            }
            1 => bytes.insert(i, *r.pick(NASTY_BYTES)),
            _ => bytes[i] = *r.pick(NASTY_BYTES),
        }
    }
    without_separators(bytes)
}

fn timestamp(r: &mut Rng, ts: i64) -> Vec<u8> {
    match r.below(30) {
        0..=17 => ts.to_string().into_bytes(),
        18 => b"0".to_vec(),
        19 => b"00".to_vec(),
        20 => b"-5".to_vec(),
        21 => b"+7".to_vec(),
        22 => format!(" {ts}").into_bytes(),
        23 => format!("{ts}abc").into_bytes(),
        24 => b"abc".to_vec(),
        25 => b"99999999999999999999".to_vec(),
        26 => b"-99999999999999999999".to_vec(),
        27 => b"1-2".to_vec(),
        28 => format!("{ts}\0{ts}").into_bytes(),
        _ => date(r),
    }
}

fn line(r: &mut Rng, ts: i64, user: &[u8]) -> Vec<u8> {
    let mut line = Vec::new();
    if r.chance(4) {
        line.extend_from_slice(&[0xEF, 0xBB, 0xBF]);
    }
    line.extend_from_slice(&timestamp(r, ts));
    line.push(b'|');
    line.extend_from_slice(user);
    line.push(b'|');
    line.extend_from_slice(r.pick(&[&b"A"[..], b"A", b"M", b"D", b"", b"X", b"a", b"AD", b" "]));
    line.push(b'|');
    line.extend_from_slice(&path(r));
    match r.below(14) {
        0 => line.extend_from_slice(b"|ff8800"),
        1 => line.extend_from_slice(b"|#00AAbb"),
        2 => line.extend_from_slice(b"|12345"),
        3 => line.extend_from_slice(b"|zz|more"),
        4 => line.extend_from_slice(b"|abcdefg"),
        5 => line.extend_from_slice(b"|##aabbcc"),
        6 => line.extend_from_slice(b"|"),
        7 => line.extend_from_slice(b"|0x1234"),
        8 => line.extend_from_slice(b"| aabbcc"),
        9 => line.extend_from_slice(b"|AaBbCc|ffffff"),
        _ => {}
    }
    if r.chance(8) && !line.is_empty() {
        let i = r.below(line.len());
        match r.below(3) {
            0 => {
                line.remove(i);
            }
            1 => line.insert(i, *r.pick(NASTY_BYTES)),
            _ => line[i] = *r.pick(NASTY_BYTES),
        }
    }
    line.retain(|&b| b != b'\n');
    line
}

fn case(r: &mut Rng) -> Vec<u8> {
    match r.below(10) {
        // pure noise
        0 => {
            let n = r.below(300);
            (0..n).map(|_| *r.pick(NASTY_BYTES)).collect()
        }
        // date strings only
        1 => {
            let mut out = Vec::new();
            for _ in 0..1 + r.below(12) {
                out.extend_from_slice(&date(r));
                out.extend_from_slice(b"|u|A|/p\n");
            }
            out
        }
        // commits made of lines
        _ => {
            let mut out = Vec::new();
            let mut ts = (r.next() % 2_000_000_000) as i64;
            let mut user = username(r);
            for _ in 0..1 + r.below(25) {
                if r.chance(40) {
                    ts += r.below(4) as i64 - 1;
                    user = username(r);
                }
                out.extend_from_slice(&line(r, ts, &user));
                if r.chance(10) {
                    out.push(b'\r');
                }
                if r.chance(3) {
                    out.extend_from_slice(b"\r\r");
                }
                out.push(b'\n');
                if r.chance(3) {
                    out.push(b'\n');
                }
            }
            if r.chance(25) {
                while out.last() == Some(&b'\n') {
                    out.pop();
                }
            }
            out
        }
    }
}

/// What the harness prints for each case, without the closing `END`.
fn run_harness(harness: &str, cases: &[Vec<u8>]) -> Vec<String> {
    let mut input = Vec::new();
    for c in cases {
        input.extend_from_slice(format!("{}\n", c.len()).as_bytes());
        input.extend_from_slice(c);
    }
    let mut child = Command::new(harness)
        .env("TZ", "UTC")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot run {harness}: {e}"));
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || stdin.write_all(&input).unwrap());
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap();
    assert!(out.status.success(), "the harness failed");

    String::from_utf8(out.stdout)
        .unwrap()
        .split("END\n")
        .map(str::to_string)
        .collect()
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn env_number(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[test]
#[ignore = "needs the compiled C++ harness: see the module documentation"]
fn random_inputs_match_the_cpp_reference() {
    let harness = std::env::var("CPP_HARNESS")
        .expect("set CPP_HARNESS to the harness built by `regen.sh build`");
    let count = env_number("FUZZ_CASES", 20_000) as usize;
    let seed = env_number("FUZZ_SEED", 1);

    let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
    let cases: Vec<Vec<u8>> = (0..count).map(|_| case(&mut rng)).collect();
    let mut expected = run_harness(&harness, &cases);
    assert_eq!(
        expected.pop().as_deref(),
        Some(""),
        "harness output ends after the last END"
    );
    assert_eq!(expected.len(), cases.len());

    let mut commits = 0;
    let mut mismatches = 0;
    for (i, (input, want)) in cases.iter().zip(&expected).enumerate() {
        // whole, or in random pieces
        let (got_commits, stats) = if i % 2 == 0 {
            parse_whole(input)
        } else {
            let sizes: Vec<usize> = (0..16)
                .map(|_| match rng.below(4) {
                    0 => 1,
                    1 => 1 + rng.below(5),
                    2 => 1 + rng.below(40),
                    _ => 1 + rng.below(400),
                })
                .collect();
            let mut sink = Collect::default();
            let stats = parse_chunked(input, &sizes, &mut sink);
            (sink.commits, stats)
        };
        commits += got_commits.len();
        let got = canonical(&got_commits, &stats);
        if &got != want {
            mismatches += 1;
            if mismatches <= 3 {
                eprintln!(
                    "MISMATCH in case {i}\ninput hex: {}\n--- C++ ---\n{want}--- Rust ---\n{got}",
                    hex(input)
                );
            }
        }
    }
    println!("seed {seed}: {count} cases, {commits} commits compared, {mismatches} mismatches");
    assert_eq!(mismatches, 0);
}
