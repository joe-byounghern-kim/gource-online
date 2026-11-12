// SPDX-License-Identifier: GPL-3.0-or-later

//! REC-113: once a recording is warmed up, `step_frame` (ticks, frame
//! build, draw, stop rules, progress) does not allocate.
//!
//! A counting global allocator records allocations made on the test thread
//! while counting is switched on. The log keeps a fixed set of users busy
//! modifying a fixed set of files, so after warm-up no entity, string or
//! buffer has to grow.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use common::*;

struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCS: Cell<u64> = const { Cell::new(0) };
}

fn note() {
    // `try_with`: the allocator can run while thread locals are torn down.
    let _ = COUNTING.try_with(|c| {
        if c.get() {
            let _ = ALLOCS.try_with(|a| a.set(a.get() + 1));
        }
    });
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc(layout) }
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc_zeroed(layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note();
        unsafe { System.realloc(ptr, layout, new_size) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// 30 files added at once, then 4 users modifying them every 10 minutes.
fn busy_log() -> gource_core::events::EventLog {
    let path = |i: usize| {
        format!(
            "/dir{}/sub{}/file{i}.{}",
            i % 5,
            i % 3,
            ["c", "h", "rs"][i % 3]
        )
    };
    let mut text = String::new();
    for i in 0..30 {
        text.push_str(&format!("1000000|user0|A|{}\n", path(i)));
    }
    for c in 1..2000 {
        let ts = 1_000_000 + c * 600;
        for f in 0..3 {
            text.push_str(&format!(
                "{ts}|user{}|M|{}\n",
                c % 4,
                path((c * 7 + f) % 30)
            ));
        }
    }
    parse_log(text.as_bytes())
}

#[test]
fn step_frame_does_not_allocate_after_warm_up() {
    let log = busy_log();
    for fps in ["60", "30", "24"] {
        let mut d = driver(
            &log,
            &[
                ("record-framerate", fps),
                ("record-viewport", "1280x720"),
                ("seconds-per-day", "20"),
                ("key", "true"),
            ],
        )
        .unwrap();
        d.begin_recording();
        // long enough for the user quadtree to reach its high-water mark
        // (users drifting apart deepen it now and then early in a run)
        for _ in 0..4000 {
            d.step_frame().unwrap();
        }
        COUNTING.with(|c| c.set(true));
        for _ in 0..600 {
            d.step_frame().unwrap();
            let _ = (d.pts_us(), d.duration_us(), d.progress(), d.frame_hash());
        }
        COUNTING.with(|c| c.set(false));
        let allocs = ALLOCS.with(|a| a.replace(0));
        assert_eq!(
            allocs, 0,
            "{fps} fps: {allocs} allocations in 600 steady frames"
        );
        assert!(d.engine().simulation().status(d.engine().log()).users > 0);
    }
}
