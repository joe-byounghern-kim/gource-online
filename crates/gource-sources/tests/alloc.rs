// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Allocation behaviour. In steady state the parser must not allocate at
//! all, and the batch encoder only for the batches it returns. A counting
//! global allocator (per thread, so parallel tests do not disturb each
//! other) checks that.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use common::*;
use gource_core::events::EventLog;
use gource_sources::{BatchEncoder, CommitSink, CustomLogParser, ParsedCommit, apply_batch};

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

struct Counting;

fn count() {
    // `try_with`: the slot may be gone while a thread shuts down.
    let _ = ALLOCATIONS.try_with(|c| c.set(c.get() + 1));
}

// SAFETY: every method forwards to `System` unchanged; counting has no
// effect on the memory handed out.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn allocations() -> usize {
    ALLOCATIONS.with(Cell::get)
}

/// Allocations made while `f` runs on this thread.
fn measure<R>(f: impl FnOnce() -> R) -> (R, usize) {
    let before = allocations();
    let r = f();
    (r, allocations() - before)
}

fn log() -> Vec<u8> {
    // includes invalid UTF-8, BOMs, colours, dates, malformed lines and
    // commits of many sizes
    let mut rng = Rng(0xA110C);
    generate_log(&mut rng, 20_000)
}

#[test]
fn parser_reaches_a_steady_state_without_allocations() {
    let bytes = log();
    let mut parser = CustomLogParser::new();
    let mut sink = Count::default();

    // Warm-up: buffers grow to the sizes this input needs.
    for chunk in bytes.chunks(777) {
        parser.push(chunk, &mut sink);
    }
    parser.finish(&mut sink);
    let commits_first_pass = sink.commits;
    assert!(commits_first_pass > 1000);

    // Same input again, chunks that split lines at awkward places.
    let ((), allocations) = measure(|| {
        for chunk in bytes.chunks(777) {
            parser.push(chunk, &mut sink);
        }
        parser.finish(&mut sink);
    });
    assert_eq!(sink.commits, 2 * commits_first_pass);
    assert_eq!(
        allocations, 0,
        "the parser allocated while parsing a known-size input"
    );
}

#[test]
fn parsing_whole_input_in_one_push_also_allocates_nothing_after_warm_up() {
    let bytes = log();
    let mut parser = CustomLogParser::new();
    let mut sink = Count::default();
    parser.push(&bytes, &mut sink);
    parser.finish(&mut sink);
    let ((), allocations) = measure(|| {
        parser.push(&bytes, &mut sink);
        parser.finish(&mut sink);
    });
    assert_eq!(allocations, 0);
}

#[test]
fn encoder_allocates_once_per_batch_once_its_strings_are_known() {
    struct Encode<'a> {
        encoder: &'a mut BatchEncoder,
    }
    impl CommitSink for Encode<'_> {
        fn commit(&mut self, c: &ParsedCommit<'_>) {
            self.encoder.commit(c);
        }
    }

    let bytes = log();
    let mut parser = CustomLogParser::new();
    let mut encoder = BatchEncoder::new();

    // Warm-up: every string gets interned, buffers reach their sizes.
    let mut first_pass = Vec::new();
    for chunk in bytes.chunks(4096) {
        parser.push(
            chunk,
            &mut Encode {
                encoder: &mut encoder,
            },
        );
        first_pass.push(encoder.take_batch());
    }
    parser.finish(&mut Encode {
        encoder: &mut encoder,
    });
    first_pass.push(encoder.take_batch());

    // Second pass: nothing new to intern. Collect the batches in a Vec with
    // room reserved up front so only the batches themselves allocate.
    let pushes = bytes.chunks(4096).count() + 1;
    let mut second_pass: Vec<Vec<u8>> = Vec::with_capacity(pushes);
    let ((), allocations) = measure(|| {
        for chunk in bytes.chunks(4096) {
            parser.push(
                chunk,
                &mut Encode {
                    encoder: &mut encoder,
                },
            );
            second_pass.push(encoder.take_batch());
        }
        parser.finish(&mut Encode {
            encoder: &mut encoder,
        });
        second_pass.push(encoder.take_batch());
    });
    let non_empty = second_pass.iter().filter(|b| !b.is_empty()).count();
    assert!(non_empty > 10);
    assert_eq!(
        allocations, non_empty,
        "one allocation per returned batch and nothing else"
    );

    // And the whole stream still decodes (the second pass repeats the first,
    // so it is a valid continuation only for the strings: apply the first).
    let mut log = EventLog::new();
    for batch in first_pass.iter().filter(|b| !b.is_empty()) {
        apply_batch(&mut log, batch).unwrap();
    }
    assert!(log.commit_count() > 1000);
}

#[test]
fn validating_a_batch_does_not_allocate() {
    let bytes = log();
    let mut parser = CustomLogParser::new();
    let mut encoder = BatchEncoder::new();
    parser.push(&bytes[..bytes.len() / 4], &mut encoder);
    let mut batch = encoder.take_batch();
    assert!(batch.len() > 1000);

    // Break the final consistency check, so all of the validation pass runs
    // and nothing is applied: only the error message may allocate.
    batch.push(0);
    let mut log = EventLog::new();
    let (result, allocations) = measure(|| apply_batch(&mut log, &batch));
    assert!(result.is_err());
    assert_eq!(allocations, 1, "just the error text");
}
