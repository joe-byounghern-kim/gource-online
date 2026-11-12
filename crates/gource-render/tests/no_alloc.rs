// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Steady-state frame builds must not touch the heap.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use gource_render::gource_core::settings::Settings;
use gource_render::{Frame, FrameBuilder, MonospaceGlyphs};

/// Counts allocations made by the current thread while `COUNTING` is set.
struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

fn note() {
    // `try_with`: the allocator can run while thread locals are torn down.
    let _ = COUNTING.try_with(|c| {
        if c.get() {
            let _ = ALLOCATIONS.try_with(|a| a.set(a.get() + 1));
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

fn allocations_during(f: impl FnOnce()) -> usize {
    ALLOCATIONS.with(|a| a.set(0));
    COUNTING.with(|c| c.set(true));
    f();
    COUNTING.with(|c| c.set(false));
    ALLOCATIONS.with(|a| a.get())
}

#[test]
fn second_and_third_builds_allocate_nothing() {
    let view = common::scene(200, 2000, 30);
    let settings = Settings::default();
    let mut glyphs = MonospaceGlyphs;
    let mut builder = FrameBuilder::new();
    let mut frame = Frame::default();

    let first = allocations_during(|| {
        builder.build(&view, &settings, 1.0 / 60.0, &mut glyphs, &mut frame);
    });
    assert!(first > 0, "the counter must see the warm-up allocations");

    for n in 2..=3 {
        let count = allocations_during(|| {
            builder.build(&view, &settings, 1.0 / 60.0, &mut glyphs, &mut frame);
        });
        assert_eq!(count, 0, "build {n} allocated {count} times");
    }
    assert!(!frame.commands.is_empty());
    assert!(frame.text.len() > 4);
}

#[test]
fn file_key_churn_reuses_entries() {
    // An extension that fades out and comes back reuses its pooled entry.
    let mut view = common::scene(10, 50, 3);
    let settings = Settings::default();
    let mut glyphs = MonospaceGlyphs;
    let mut builder = FrameBuilder::new();
    let mut frame = Frame::default();
    let full = view.hud.extensions.clone();

    let mut run = |view: &gource_render::gource_core::view::SceneView, frames: usize| {
        for _ in 0..frames {
            builder.build(view, &settings, 1.0 / 60.0, &mut glyphs, &mut frame);
        }
    };
    run(&view, 10);
    // Drop "rs" until it has faded out and been deleted (1 s fade at 60 fps).
    view.hud.extensions.remove(0);
    run(&view, 90);
    view.hud.extensions = full;
    let count = allocations_during(|| run(&view, 90));
    assert_eq!(count, 0);
}

#[test]
fn builds_with_decorations_allocate_nothing() {
    use gource_render::gource_core::glam::{Vec2, Vec3};
    use gource_render::gource_core::view::{AgentKeyRow, DecoKind, UserDecor};

    let mut view = common::scene(200, 2000, 30);
    for (i, u) in view.users.iter_mut().enumerate() {
        if i % 3 != 0 {
            u.decor = UserDecor {
                mask: if i % 2 == 0 {
                    0b11_1111
                } else {
                    DecoKind::Cape.bit() | DecoKind::Badge.bit()
                },
                dynamic: 0b11_1111,
                badge: (i % 10) as u8,
                tint: [[200, 100, 50]; DecoKind::COUNT],
                envelope: 0.7,
                scale: 1.0,
                vel: Vec2::new(120.0, -40.0),
                phase: 0.3,
                motion: 1.0,
                ..UserDecor::default()
            };
        }
    }
    view.hud.show_agent_key = true;
    for (name, count) in [("Claude", 1204), ("Codex", 311), ("GitHub Copilot", 87)] {
        let name = view.push_text(name);
        view.hud.agents.push(AgentKeyRow {
            name,
            colour: Vec3::new(0.9, 0.6, 0.0),
            count,
        });
    }
    view.hud.agents_more = 4;
    view.hud.filter_note = Some(view.push_text("AI commits: only assisted"));
    let baseline = view.clone();
    for line in [
        "PR: P open / D draft / M merged / X closed",
        "Review: A approved / ! changes / C comment",
        "Age: replay days / target in hover",
    ] {
        let range = view.push_text(line);
        view.hud.pr_key.push(range);
    }
    for line in ["Lifecycle: complete", "Reviews: partial", "Files: unknown"] {
        let range = view.push_text(line);
        view.hud.pr_coverage.push(range);
    }
    view.hud.pr_flow = Some(view.push_text("PRs (repository): 87 open, oldest 48d"));

    let settings = Settings::default();
    let mut glyphs = MonospaceGlyphs;
    let mut builder = FrameBuilder::new();
    let mut frame = Frame::default();
    builder.build(&view, &settings, 1.0 / 60.0, &mut glyphs, &mut frame);
    for n in 2..=3 {
        let count = allocations_during(|| {
            builder.build(&view, &settings, 1.0 / 60.0, &mut glyphs, &mut frame);
        });
        assert_eq!(count, 0, "build {n} allocated {count} times");
    }
    assert!(
        frame
            .commands
            .iter()
            .any(|c| c.texture == gource_render::TextureId::Deco)
    );
    let mut baseline_frame = Frame::default();
    let mut baseline_builder = FrameBuilder::new();
    for _ in 0..3 {
        baseline_builder.build(
            &baseline,
            &settings,
            1.0 / 60.0,
            &mut glyphs,
            &mut baseline_frame,
        );
    }
    // This preexisting fixture includes extension-key boxes and a text box,
    // in addition to the typical 12-pass decorated scene. PR text must add none.
    assert_eq!(
        frame.commands.len(),
        baseline_frame.commands.len(),
        "rich PR text added rendering passes"
    );
}
