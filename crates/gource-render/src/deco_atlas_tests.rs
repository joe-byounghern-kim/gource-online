// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Tests for the generated decoration atlas table.

use super::*;

fn overlaps(a: [u32; 4], b: [u32; 4]) -> bool {
    a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

/// Gap in pixels between two rectangles along the axis they are closest on.
fn gap(a: [u32; 4], b: [u32; 4]) -> u32 {
    let dx = a[0].saturating_sub(b[2]).max(b[0].saturating_sub(a[2]));
    let dy = a[1].saturating_sub(b[3]).max(b[1].saturating_sub(a[3]));
    dx.max(dy)
}

#[test]
fn counts_match() {
    assert_eq!(DecoCell::ALL.len(), USED_CELLS);
    assert_eq!(USED_CELLS, 14);
    const { assert!(USED_CELLS <= CELL_COUNT) };
    assert_eq!(CELL_COUNT as u32, (ATLAS_SIZE / CELL_SIZE).pow(2));
    assert_eq!(ART_SIZE + 2 * GUTTER, CELL_SIZE);
    for (i, cell) in DecoCell::ALL.iter().enumerate() {
        assert_eq!(cell.index(), i);
    }
    assert_eq!(DecoCell::BADGES[0], DecoCell::BadgeCircle);
    assert_eq!(DecoCell::BADGES[9], DecoCell::BadgeBolt);
}

#[test]
fn rectangles_are_inside_the_atlas() {
    for cell in DecoCell::ALL {
        let [x0, y0, x1, y1] = cell.px();
        assert!(x0 < x1 && y0 < y1, "{cell:?}");
        assert_eq!((x1 - x0, y1 - y0), (ART_SIZE, ART_SIZE), "{cell:?}");
        assert!(x1 <= ATLAS_SIZE && y1 <= ATLAS_SIZE, "{cell:?}");
        let [u0, v0, u1, v1] = cell.uv();
        for v in [u0, v0, u1, v1] {
            assert!((0.0..=1.0).contains(&v), "{cell:?}");
        }
        let s = ATLAS_SIZE as f32;
        assert_eq!(
            [u0 * s, v0 * s, u1 * s, v1 * s],
            px_f32(cell.px()),
            "{cell:?}"
        );
    }
}

fn px_f32(p: [u32; 4]) -> [f32; 4] {
    p.map(|v| v as f32)
}

#[test]
fn cells_do_not_overlap_and_keep_the_gutter() {
    for (i, a) in DecoCell::ALL.iter().enumerate() {
        let [x0, y0, x1, y1] = a.px();
        // Gutter to the atlas border.
        assert!(x0 >= GUTTER && y0 >= GUTTER, "{a:?}");
        assert!(
            ATLAS_SIZE - x1 >= GUTTER && ATLAS_SIZE - y1 >= GUTTER,
            "{a:?}"
        );
        for b in &DecoCell::ALL[i + 1..] {
            assert!(!overlaps(a.px(), b.px()), "{a:?} overlaps {b:?}");
            assert!(
                gap(a.px(), b.px()) >= GUTTER,
                "{a:?} and {b:?} are closer than the gutter"
            );
        }
    }
}

#[test]
fn aspects_are_the_anchor_aspects() {
    // Each cell is drawn for the pixel aspect of its AI-55 anchor on the
    // 384x512 avatar, so the generator's anchors must match `deco.rs`.
    use crate::deco::{BADGE_RECT, CAPE_RECT, HALO_RECT, HAT_RECT, SHADES_RECT};
    let aspect = |r: [f32; 4]| (r[2] - r[0]) * 384.0 / ((r[3] - r[1]) * 512.0);
    for (cell, rect) in [
        (DecoCell::Cape, CAPE_RECT),
        (DecoCell::Halo, HALO_RECT),
        (DecoCell::Shades, SHADES_RECT),
        (DecoCell::Hat, HAT_RECT),
        (DecoCell::BadgeCircle, BADGE_RECT),
    ] {
        assert!(
            (cell.aspect() - aspect(rect)).abs() < 1e-3,
            "{cell:?} {} != {}",
            cell.aspect(),
            aspect(rect)
        );
    }
    // cape 1.74, halo 2.68, shades 2.75, hat 1.40
    assert!((DecoCell::Cape.aspect() - 1.74).abs() < 0.01);
}

#[test]
fn png_is_the_declared_size() {
    // IHDR: 8 byte signature, 4 byte length, "IHDR", then width and height.
    let png = include_bytes!("../../../data/decorations.png");
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert_eq!(&png[12..16], b"IHDR");
    let w = u32::from_be_bytes(png[16..20].try_into().unwrap());
    let h = u32::from_be_bytes(png[20..24].try_into().unwrap());
    assert_eq!((w, h), (ATLAS_SIZE, ATLAS_SIZE));
    assert_eq!(png[25], 6, "colour type RGBA");
}
