// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Throughput of the parser and of the batch codec on a generated 50 MB log.
//!
//! Ignored by default (it needs an optimised build to mean anything):
//!
//! ```text
//! cargo test --release -p gource-sources --test perf -- --ignored --nocapture
//! ```

mod common;

use std::time::Instant;

use common::*;
use gource_core::events::EventLog;
use gource_sources::{BatchEncoder, CommitSink, CustomLogParser, EventLogSink, apply_batch};

const TARGET_BYTES: usize = 50 * 1024 * 1024;
const CHUNK: usize = 64 * 1024;

fn push_num(out: &mut Vec<u8>, mut n: u64) {
    let mut digits = [0u8; 20];
    let mut i = digits.len();
    loop {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(&digits[i..]);
}

/// A log shaped like a large repository's history: a few hundred
/// contributors, a few hundred thousand distinct paths with a skewed
/// popularity, commits of 1 to ~30 files, mostly modifications, a few
/// colours and some non-ASCII names.
fn generate(target: usize) -> Vec<u8> {
    let mut rng = Rng(0x5EED_1234_ABCD_EF01);

    let users: Vec<Vec<u8>> = (0..400)
        .map(|i| match i % 5 {
            0 => format!("Contributor Number {i}").into_bytes(),
            1 => format!("dev{i}").into_bytes(),
            2 => format!("J\u{fc}rgen M\u{fc}ller {i}").into_bytes(),
            3 => format!("\u{6df1}\u{5733}{i}").into_bytes(),
            _ => format!("user-{i}@example.org").into_bytes(),
        })
        .collect();

    let paths: Vec<Vec<u8>> = (0..200_000)
        .map(|i| {
            let (a, b, c) = (i % 37, (i / 37) % 91, i / 3367);
            let ext = ["rs", "c", "h", "md", "toml", "json", "py", "ts"][i % 8];
            if i % 997 == 0 {
                format!("src/m\u{f3}dulo{a}/sub{b}/\u{6587}\u{4ef6}{i}.{ext}").into_bytes()
            } else {
                format!("src/module{a}/sub{b}/dir{c}/file_{i}.{ext}").into_bytes()
            }
        })
        .collect();

    let mut out = Vec::with_capacity(target + 4096);
    let mut ts = 1_262_304_000u64;
    while out.len() < target {
        ts += rng.below(3600) as u64;
        let user = &users[rng.below(users.len())];
        let files = 1 + (rng.below(100) * rng.below(100)) / 400; // mostly small, a long tail
        for _ in 0..files {
            // square the uniform draw: popular paths come up much more often
            let r = rng.below(paths.len());
            let path = &paths[r * r / paths.len()];
            push_num(&mut out, ts);
            out.push(b'|');
            out.extend_from_slice(user);
            out.push(b'|');
            out.push(match rng.below(10) {
                0 | 1 => b'A',
                2 => b'D',
                _ => b'M',
            });
            out.push(b'|');
            if rng.below(7) != 0 {
                out.push(b'/');
            }
            out.extend_from_slice(path);
            if rng.below(50) == 0 {
                out.extend_from_slice(b"|ff8800");
            }
            out.push(b'\n');
        }
    }
    out
}

fn mb_per_s(bytes: usize, seconds: f64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0) / seconds
}

fn feed(bytes: &[u8], sink: &mut dyn CommitSink) -> gource_sources::ParseStats {
    let mut parser = CustomLogParser::new();
    for chunk in bytes.chunks(CHUNK) {
        parser.push(chunk, sink);
    }
    parser.finish(sink);
    parser.stats()
}

#[test]
#[ignore = "benchmark; run with --release --ignored --nocapture"]
fn fifty_megabyte_log() {
    let started = Instant::now();
    let log = generate(TARGET_BYTES);
    println!(
        "generated {:.1} MB in {:.2}s",
        log.len() as f64 / 1e6,
        started.elapsed().as_secs_f64()
    );

    // parse only
    let mut count = Count::default();
    let started = Instant::now();
    let stats = feed(&log, &mut count);
    let parse = started.elapsed().as_secs_f64();
    println!(
        "parse            : {:6.3}s  {:7.1} MB/s  ({} lines, {} commits, {} skipped)",
        parse,
        mb_per_s(log.len(), parse),
        stats.lines,
        stats.commits,
        stats.skipped_lines
    );
    assert_eq!(stats.skipped_lines, 0);
    assert_eq!(count.commits, stats.commits);

    // parse into an EventLog
    let mut event_log = EventLog::new();
    let started = Instant::now();
    feed(&log, &mut EventLogSink::new(&mut event_log));
    let sink = started.elapsed().as_secs_f64();
    println!(
        "parse + sink     : {:6.3}s  {:7.1} MB/s  ({} strings)",
        sink,
        mb_per_s(log.len(), sink),
        event_log.string_count()
    );

    // parse + encode batches, then apply them (what the worker and the main
    // thread do), timed separately
    struct Batches {
        encoder: BatchEncoder,
        batches: Vec<Vec<u8>>,
    }
    impl CommitSink for Batches {
        fn commit(&mut self, c: &gource_sources::ParsedCommit<'_>) {
            self.encoder.commit(c);
            if self.encoder.pending_bytes() >= CHUNK {
                self.batches.push(self.encoder.take_batch());
            }
        }
    }
    let mut sink = Batches {
        encoder: BatchEncoder::new(),
        batches: Vec::new(),
    };
    let started = Instant::now();
    feed(&log, &mut sink);
    sink.batches.push(sink.encoder.take_batch());
    let encode = started.elapsed().as_secs_f64();
    let encoded: usize = sink.batches.iter().map(Vec::len).sum();
    println!(
        "parse + encode   : {:6.3}s  {:7.1} MB/s  ({} batches, {:.1} MB encoded = {:.0}% of the log)",
        encode,
        mb_per_s(log.len(), encode),
        sink.batches.len(),
        encoded as f64 / 1e6,
        100.0 * encoded as f64 / log.len() as f64
    );

    let mut applied = EventLog::new();
    let started = Instant::now();
    for batch in &sink.batches {
        apply_batch(&mut applied, batch).unwrap();
    }
    let apply = started.elapsed().as_secs_f64();
    println!(
        "apply_batch      : {:6.3}s  {:7.1} MB/s of batches ({:.1} MB/s of log)",
        apply,
        mb_per_s(encoded, apply),
        mb_per_s(log.len(), apply)
    );
    assert_logs_equal(&event_log, &applied);

    if !cfg!(debug_assertions) {
        assert!(parse < 1.0, "parsing 50 MB took {parse:.2}s");
    }
}
