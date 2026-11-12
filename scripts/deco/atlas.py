#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate the decoration atlas for AI co-author markers (docs/features/ai-coauthors.md,
AI-54 and AI-55).

Writes
  data/decorations.png                       256x256 RGBA, 4x4 cells of 64x64 px
  crates/gource-render/src/deco_atlas.rs     the cell table (normalised UV rectangles); its
                                             tests live in deco_atlas_tests.rs

Layout: each cell holds its art in the central 56x56 px; the 4 px gutter around it carries
the extruded edge colour of the art at alpha 0, so bilinear filtering and mip levels 0..2
never pull in a neighbour. The art is neutral greyscale with alpha because the `Textured`
program is `texture * vertex colour`, so the vertex colour is the tint.

Every cell is stretched over an anchor rectangle of the avatar quad (AI-55), so each is
drawn in a logical space of height 1 and width `aspect` (the anchor's pixel aspect on the
384x512 avatar) and resampled to the square cell; the stretch then cancels on screen.

The art is original: polygons, ellipses and gradients drawn with Pillow, no brand marks.
Pillow only; deterministic (no randomness, no timestamps).

  python3 scripts/deco/atlas.py              write both files
  python3 scripts/deco/atlas.py --preview P  also write a composite of the decorations on
                                             data/user.png at the AI-55 anchors to P
"""

import argparse
import math
import os
import sys

from PIL import Image, ImageChops, ImageDraw, ImageFilter

ATLAS = 256
CELL = 64
GUTTER = 4
ART = CELL - 2 * GUTTER  # 56
SS = 8  # supersampling factor for the art

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
PNG_PATH = os.path.join(ROOT, "data", "decorations.png")
RS_PATH = os.path.join(ROOT, "crates", "gource-render", "src", "deco_atlas.rs")
AVATAR_PATH = os.path.join(ROOT, "data", "user.png")

# AI-55 anchor rectangles (u0, v0, u1, v1) of the 384x512 avatar quad.
AVATAR_W, AVATAR_H = 384, 512
ANCHORS = {
    "cape": (-0.16, 0.43, 1.16, 1.00),
    "halo": (0.24, -0.04, 0.74, 0.10),
    "shades": (0.27, 0.24, 0.71, 0.36),
    "hat": (0.20, -0.14, 0.78, 0.17),
    "badge": (0.70, 0.68, 0.92, 0.84),
}


def aspect_of(name):
    u0, v0, u1, v1 = ANCHORS[name]
    return ((u1 - u0) * AVATAR_W) / ((v1 - v0) * AVATAR_H)


class Art:
    """One cell's art area: an RGBA canvas at SS x resolution in a logical space of
    height 1 and width `aspect`."""

    def __init__(self, aspect):
        self.aspect = aspect
        self.w = self.h = ART * SS
        self.img = Image.new("RGBA", (self.w, self.h), (0, 0, 0, 0))

    def px(self, x, y):
        return (x / self.aspect * self.w, y * self.h)

    def pts(self, points):
        return [self.px(x, y) for x, y in points]

    def mask_poly(self, points):
        m = Image.new("L", (self.w, self.h), 0)
        ImageDraw.Draw(m).polygon(self.pts(points), fill=255)
        return m

    def mask_ellipse(self, cx, cy, rx, ry):
        m = Image.new("L", (self.w, self.h), 0)
        x0, y0 = self.px(cx - rx, cy - ry)
        x1, y1 = self.px(cx + rx, cy + ry)
        ImageDraw.Draw(m).ellipse([x0, y0, x1, y1], fill=255)
        return m

    def fill_image(self, fill):
        """`fill` is a grey level, ("v", y0, y1, g0, g1) for a vertical gradient between
        logical rows y0 and y1, or ("r", cx, cy, r, g_in, g_out) for a radial one."""
        if isinstance(fill, int):
            return Image.new("L", (self.w, self.h), fill)
        if fill[0] == "v":
            _, y0, y1, g0, g1 = fill
            col = Image.new("L", (1, self.h))
            data = []
            for j in range(self.h):
                t = min(1.0, max(0.0, ((j + 0.5) / self.h - y0) / (y1 - y0)))
                data.append(int(round(g0 + (g1 - g0) * t)))
            col.putdata(data)
            return col.resize((self.w, self.h), Image.NEAREST)
        _, cx, cy, r, g_in, g_out = fill
        lo = 64
        small = Image.new("L", (lo, lo))
        data = []
        for j in range(lo):
            for i in range(lo):
                x = (i + 0.5) / lo * self.aspect
                y = (j + 0.5) / lo
                t = min(1.0, math.hypot(x - cx, y - cy) / r)
                data.append(int(round(g_in + (g_out - g_in) * t)))
        small.putdata(data)
        return small.resize((self.w, self.h), Image.BILINEAR)

    def paint(self, mask, fill, alpha=255):
        if alpha != 255:
            mask = mask.point(lambda p: p * alpha // 255)
        g = self.fill_image(fill)
        layer = Image.merge("RGBA", (g, g, g, mask))
        self.img = Image.alpha_composite(self.img, layer)

    def poly(self, points, fill, alpha=255):
        self.paint(self.mask_poly(points), fill, alpha)

    def ellipse(self, cx, cy, rx, ry, fill, alpha=255):
        self.paint(self.mask_ellipse(cx, cy, rx, ry), fill, alpha)

    def blur(self, mask, sigma):
        return mask.filter(ImageFilter.GaussianBlur(sigma * self.h))

    def finish(self):
        """Downsample to 56x56 (premultiplied by Pillow) and return it."""
        return self.img.resize((ART, ART), Image.LANCZOS)


# ---------------------------------------------------------------- curves

def bezier(p0, p1, p2, n=24):
    out = []
    for i in range(n + 1):
        t = i / n
        a, b, c = (1 - t) ** 2, 2 * t * (1 - t), t * t
        out.append((a * p0[0] + b * p1[0] + c * p2[0], a * p0[1] + b * p1[1] + c * p2[1]))
    return out


def circle_pts(cx, cy, r, n=64, a0=0.0, a1=360.0):
    return [
        (cx + r * math.cos(math.radians(a0 + (a1 - a0) * i / n)),
         cy + r * math.sin(math.radians(a0 + (a1 - a0) * i / n)))
        for i in range(n + 1)
    ]


def scaled(points, cx, cy, sx, sy):
    return [(cx + (x - cx) * sx, cy + (y - cy) * sy) for x, y in points]


def mirror(points, a):
    return [(a - x, y) for x, y in points]


# ---------------------------------------------------------------- cells

def draw_cape():
    """A superhero cape seen from the front, behind the avatar: square shoulders a little
    wider than the torso (so both shoulder tips and flanks show beside it), flaring out to
    a softly waved hem just below the avatar's base. Drawn in avatar (u, v) coordinates."""
    a = aspect_of("cape")
    art = Art(a)
    u0, v0, u1, v1 = ANCHORS["cape"]

    def uv(u, v):
        return ((u - u0) / (u1 - u0) * a, (v - v0) / (v1 - v0))

    def uvs(points):
        return [uv(u, v) for u, v in points]

    hem_v = 0.935
    corner = (-0.14, 0.975)
    # left half, from the neck (hidden behind the head) over the rounded shoulder, then
    # flaring out to the hem corner, which hangs lower than the middle of the hem
    shoulder = bezier((0.40, 0.465), (0.18, 0.455), (0.135, 0.52), 16)
    flank = bezier((0.135, 0.52), (0.07, 0.70), corner, 24)[1:]
    left = shoulder + flank
    n = 48
    hem = []
    for i in range(1, n):
        t = i / n
        u = corner[0] + (1.0 - 2 * corner[0]) * t
        sag = (corner[1] - hem_v) * (2 * t - 1) ** 2
        # three soft waves over a hem that rises a little toward the middle
        hem.append((u, hem_v + sag + 0.014 * math.sin(2 * math.pi * 3 * t)))
    right = mirror(left, 1.0)[::-1]
    outline = uvs(left + hem + right)
    art.poly(outline, ("v", 0.0, 1.0, 250, 165))
    # the inside of the cape shows along both flanks: a darker band under the outer edge
    inner_l = bezier((0.17, 0.54), (0.12, 0.72), (0.0, hem_v + 0.01), 24)
    band = uvs(flank + inner_l[::-1])
    art.poly(band, 70, 150)
    art.poly(uvs(mirror(flank, 1.0) + mirror(inner_l, 1.0)[::-1]), 70, 150)
    # shoulder highlight: the fabric catches light where it folds over the shoulder
    gleam = (bezier((0.36, 0.47), (0.21, 0.465), (0.165, 0.515), 12)
             + bezier((0.19, 0.535), (0.24, 0.49), (0.36, 0.485), 12))
    art.poly(uvs(gleam), 255, 150)
    art.poly(uvs(mirror(gleam, 1.0)), 255, 150)
    # folds: dark wedges from the shoulders to the hem troughs, mirrored
    for fu, tu, w in ((0.30, 0.08, 0.025), (0.42, 0.30, 0.03)):
        for m in (False, True):
            pts = [(fu - 0.01, 0.50), (fu + 0.01, 0.50), (tu + w, hem_v), (tu - w, hem_v)]
            art.poly(uvs(mirror(pts, 1.0) if m else pts), 70, 70)
    return art.finish()


def draw_halo():
    a = aspect_of("halo")
    art = Art(a)
    cx, cy = a / 2, 0.5
    rx, ry, t = 1.20, 0.40, 0.14
    ring = art.mask_ellipse(cx, cy, rx, ry)
    inner = art.mask_ellipse(cx, cy, rx - t * 1.5, ry - t)
    ring_mask = Image.eval(inner, lambda p: 255 - p)
    ring_mask = Image.composite(ring_mask, Image.new("L", ring.size, 0), ring)
    glow = art.blur(ring_mask, 0.05)
    art.paint(glow, 255, 110)
    # the far (upper) arc is a little dimmer than the near (lower) one
    art.paint(art.blur(ring_mask, 0.012), ("v", cy - ry, cy + ry, 205, 255))
    return art.finish()


def draw_shades():
    a = aspect_of("shades")
    art = Art(a)
    # left lens: wide top, rounded narrower bottom (wayfarer-like)
    lens = (
        [(0.14, 0.30), (1.20, 0.30)]
        + bezier((1.20, 0.30), (1.18, 0.62), (1.02, 0.80))
        + bezier((1.02, 0.80), (0.70, 0.94), (0.34, 0.86))[1:]
        + bezier((0.34, 0.86), (0.14, 0.78), (0.14, 0.30))[1:]
    )
    lcx, lcy = 0.67, 0.58
    frame = scaled(lens, lcx, lcy, 1.10, 1.22)
    frame_r = mirror(frame, a)
    # temples
    art.poly([(0.00, 0.20), (0.14, 0.20), (0.18, 0.34), (0.02, 0.34)], 255)
    art.poly(mirror([(0.00, 0.20), (0.14, 0.20), (0.18, 0.34), (0.02, 0.34)], a), 255)
    # brow bar and bridge
    art.poly([(0.06, 0.14), (a - 0.06, 0.14), (a - 0.06, 0.30), (0.06, 0.30)], 255)
    art.poly([(1.10, 0.28), (a - 1.10, 0.28), (a - 1.10, 0.46), (1.10, 0.46)], 255)
    art.poly(frame, 255)
    art.poly(frame_r, 255)
    art.poly(lens, ("v", 0.30, 0.88, 95, 55))
    art.poly(mirror(lens, a), ("v", 0.30, 0.88, 95, 55))
    # specular glints: bright slashes near the top-left of each lens (white stays the
    # brightest value, so it is the lightest part of the tinted result)
    for ox in (0.0, a - 2 * lcx + 0.0):
        art.poly([(0.32 + ox, 0.36), (0.50 + ox, 0.36), (0.38 + ox, 0.58), (0.28 + ox, 0.58)],
                 255, 235)
        art.poly([(0.58 + ox, 0.36), (0.66 + ox, 0.36), (0.60 + ox, 0.46), (0.54 + ox, 0.46)],
                 255, 200)
    return art.finish()


def draw_hat():
    a = aspect_of("hat")
    art = Art(a)
    cx = a / 2
    # silhouette: pointed cone with a drooping tip, plus a brim
    cone = (
        [(cx - 0.40, 0.76)]
        + bezier((cx - 0.40, 0.76), (cx - 0.22, 0.36), (cx + 0.05, 0.20))[1:]
        + bezier((cx + 0.05, 0.20), (cx + 0.30, 0.10), (cx + 0.38, 0.22))[1:]
        + bezier((cx + 0.38, 0.22), (cx + 0.22, 0.28), (cx + 0.22, 0.46))[1:]
        + [(cx + 0.40, 0.76)]
    )
    brim = circle_pts(cx, 0.80, 1.0, 64)
    brim = [(cx + (x - cx) * 0.62, 0.80 + (y - 0.80) * 0.15) for x, y in brim]
    silhouette = art.mask_poly(cone)
    silhouette.paste(255, mask=art.mask_poly(brim))
    art.paint(art.blur(silhouette, 0.06), 255, 120)  # glow
    art.poly(cone, ("v", 0.10, 0.78, 215, 135))
    art.poly([(cx - 0.40, 0.76), (cx - 0.30, 0.52), (cx - 0.12, 0.40), (cx - 0.12, 0.76)], 100, 55)
    art.poly(brim, ("v", 0.65, 0.95, 195, 140))
    # bright band just above the brim
    art.poly([(cx - 0.375, 0.64), (cx + 0.375, 0.64), (cx + 0.40, 0.76), (cx - 0.40, 0.76)], 255)
    art.poly([(cx - 0.375, 0.64), (cx + 0.375, 0.64), (cx + 0.38, 0.67), (cx - 0.38, 0.67)], 255)
    # sparkles
    for sx, sy, r in ((cx - 0.50, 0.30, 0.09), (cx + 0.52, 0.52, 0.07)):
        art.poly(star_pts(sx, sy, r, r * 0.28, 4, -90), 255)
    return art.finish()


def star_pts(cx, cy, ro, ri, n, start=-90.0):
    out = []
    for i in range(2 * n):
        r = ro if i % 2 == 0 else ri
        ang = math.radians(start + 180.0 * i / n)
        out.append((cx + r * math.cos(ang), cy + r * math.sin(ang)))
    return out


BADGE_FILL = ("v", 0.0, 1.0, 255, 215)
# Every badge is drawn at BADGE_INNER of the cell and wrapped in a dark rim BADGE_RIM wide
# (fractions of the cell): the shader multiplies the texture by the tint, so the rim stays
# dark and the badge keeps its contrast on a cape or avatar of the same colour.
BADGE_INNER = 0.80
BADGE_RIM = 0.085
BADGE_RIM_GREY = 24


def shrink(mask, k):
    """`mask` scaled by `k` about the centre of the canvas."""
    w, h = mask.size
    small = mask.resize((int(round(w * k)), int(round(h * k))), Image.LANCZOS)
    out = Image.new("L", mask.size, 0)
    out.paste(small, ((w - small.size[0]) // 2, (h - small.size[1]) // 2))
    return out


def dilate(mask, d, art):
    """`mask` grown by a disk of radius `d` (logical units): the maximum of its copies
    shifted over rings of the disk (on a padded canvas, so nothing wraps around)."""
    r = d * art.h
    pad = int(math.ceil(r)) + 2
    w, h = mask.size
    big = Image.new("L", (w + 2 * pad, h + 2 * pad), 0)
    big.paste(mask, (pad, pad))
    out = big.copy()
    for ring in (1.0, 0.75, 0.5, 0.25):
        n = max(8, int(round(48 * ring)))
        for i in range(n):
            ang = 2 * math.pi * i / n
            dx, dy = int(round(r * ring * math.cos(ang))), int(round(r * ring * math.sin(ang)))
            out = ImageChops.lighter(out, ImageChops.offset(big, dx, dy))
    return out.crop((pad, pad, pad + w, pad + h))


def badge(shape):
    art = Art(1.0)
    if shape == "circle":
        m = art.mask_ellipse(0.5, 0.5, 0.46, 0.46)
    elif shape == "diamond":
        m = art.mask_poly([(0.5, 0.04), (0.94, 0.5), (0.5, 0.96), (0.06, 0.5)])
    elif shape == "hexagon":
        m = art.mask_poly(circle_pts(0.5, 0.5, 0.47, 6, 0.0, 360.0)[:-1])
    elif shape == "triangle":
        m = art.mask_poly([(0.5, 0.04), (0.97, 0.88), (0.03, 0.88)])
    elif shape == "square":
        r = 0.10
        pts = []
        for cx, cy, a0 in ((0.90 - r, 0.10 + r, -90), (0.90 - r, 0.90 - r, 0),
                           (0.10 + r, 0.90 - r, 90), (0.10 + r, 0.10 + r, 180)):
            pts += circle_pts(cx, cy, r, 8, a0, a0 + 90)
        m = art.mask_poly(pts)
    elif shape == "star":
        m = art.mask_poly(star_pts(0.5, 0.54, 0.50, 0.23, 5))
    elif shape == "ring":
        outer = art.mask_ellipse(0.5, 0.5, 0.47, 0.47)
        inner = art.mask_ellipse(0.5, 0.5, 0.25, 0.25)
        m = Image.composite(Image.new("L", outer.size, 0), outer, inner)
    elif shape == "plus":
        w = 0.19
        m = art.mask_poly([(0.5 - w, 0.04), (0.5 + w, 0.04), (0.5 + w, 0.5 - w), (0.96, 0.5 - w),
                           (0.96, 0.5 + w), (0.5 + w, 0.5 + w), (0.5 + w, 0.96), (0.5 - w, 0.96),
                           (0.5 - w, 0.5 + w), (0.04, 0.5 + w), (0.04, 0.5 - w),
                           (0.5 - w, 0.5 - w)])
    elif shape == "drop":
        cx, cy, r, tip = 0.5, 0.63, 0.35, (0.5, 0.03)
        theta = math.degrees(math.acos(r / (cy - tip[1])))
        m = art.mask_poly([tip] + circle_pts(cx, cy, r, 48, -90 + theta, 270 - theta))
    elif shape == "bolt":
        m = art.mask_poly([(0.62, 0.03), (0.17, 0.57), (0.43, 0.57), (0.33, 0.97), (0.83, 0.40),
                           (0.57, 0.40), (0.73, 0.03)])
    else:
        raise ValueError(shape)
    m = shrink(m, BADGE_INNER)
    art.paint(dilate(m, BADGE_RIM, art), BADGE_RIM_GREY)
    art.paint(m, BADGE_FILL)
    return art.finish()


BADGES = ["circle", "diamond", "hexagon", "triangle", "square", "star", "ring", "plus", "drop",
          "bolt"]

# (name, Rust variant, drawing function) in cell order (row-major); two cells are unused.
CELLS = [
    ("cape", "Cape", draw_cape),
    ("halo", "Halo", draw_halo),
    ("shades", "Shades", draw_shades),
    ("hat", "Hat", draw_hat),
] + [("badge-" + s, "Badge" + s.capitalize(), (lambda s=s: badge(s))) for s in BADGES]
UNUSED_CELLS = 16 - len(CELLS)


# ---------------------------------------------------------------- cell assembly

def build_cell(art):
    """Place 56x56 art into a 64x64 cell: give fully transparent pixels the colour of the
    nearest opaque ones (so bilinear filtering never drags in black), then extrude the
    edge colour into the gutter at alpha 0."""
    px = art.load()
    rgb = [[None] * ART for _ in range(ART)]
    for y in range(ART):
        for x in range(ART):
            r, g, b, al = px[x, y]
            if al > 0:
                rgb[y][x] = (r, g, b)
    if any(v is not None for row in rgb for v in row):
        while any(v is None for row in rgb for v in row):
            nxt = [row[:] for row in rgb]
            for y in range(ART):
                for x in range(ART):
                    if rgb[y][x] is not None:
                        continue
                    acc, n = [0, 0, 0], 0
                    for dy in (-1, 0, 1):
                        for dx in (-1, 0, 1):
                            yy, xx = y + dy, x + dx
                            if 0 <= yy < ART and 0 <= xx < ART and rgb[yy][xx] is not None:
                                for k in range(3):
                                    acc[k] += rgb[yy][xx][k]
                                n += 1
                    if n:
                        nxt[y][x] = tuple((acc[k] + n // 2) // n for k in range(3))
            rgb = nxt
    else:
        rgb = [[(0, 0, 0)] * ART for _ in range(ART)]
    cell = Image.new("RGBA", (CELL, CELL), (0, 0, 0, 0))
    cp = cell.load()
    for y in range(CELL):
        for x in range(CELL):
            ax, ay = x - GUTTER, y - GUTTER
            inside = 0 <= ax < ART and 0 <= ay < ART
            cx, cy = min(max(ax, 0), ART - 1), min(max(ay, 0), ART - 1)
            r, g, b = rgb[cy][cx]
            cp[x, y] = (r, g, b, px[ax, ay][3]) if inside else (r, g, b, 0)
    return cell


def cell_origin(index):
    return (index % 4) * CELL, (index // 4) * CELL


def build_atlas():
    atlas = Image.new("RGBA", (ATLAS, ATLAS), (0, 0, 0, 0))
    for i in range(16):
        art = CELLS[i][2]() if i < len(CELLS) else Image.new("RGBA", (ART, ART), (0, 0, 0, 0))
        atlas.paste(build_cell(art), cell_origin(i))
    return atlas


def check(atlas):
    px = atlas.load()
    for i in range(16):
        ox, oy = cell_origin(i)
        opaque = 0
        for y in range(CELL):
            for x in range(CELL):
                r, g, b, al = px[ox + x, oy + y]
                assert r == g == b, "art must be greyscale"
                inside = GUTTER <= x < CELL - GUTTER and GUTTER <= y < CELL - GUTTER
                if not inside:
                    assert al == 0, "gutter alpha must be 0 (cell %d)" % i
                else:
                    opaque += al > 0
        if i < len(CELLS):
            assert opaque > 100, "cell %d (%s) is empty" % (i, CELLS[i][0])
        else:
            assert opaque == 0, "unused cell %d is not empty" % i


# ---------------------------------------------------------------- Rust table

def rust_table():
    names = [c[1] for c in CELLS]
    out = []
    w = out.append
    w("// SPDX-License-Identifier: GPL-3.0-or-later")
    w("// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.")
    w("//")
    w("// Generated by scripts/deco/atlas.py, do not edit.")
    w("")
    w("//! Cell table of the decoration atlas `data/decorations.png` (AI-54).")
    w("//!")
    w(f"//! {ATLAS}x{ATLAS} RGBA, a 4x4 grid of {CELL}x{CELL} px cells. The art of each cell is the")
    w(f"//! central {ART}x{ART} px; the {GUTTER} px gutter around it holds the extruded edge colour at alpha 0, so")
    w("//! mip levels 0 to 2 do not bleed into a neighbour. [`DecoCell::uv`] is the rectangle of")
    w("//! the art area only, as `(u0, v0, u1, v1)` with `v` down. The art is greyscale with alpha;")
    w("//! the vertex colour tints it.")
    w("//!")
    w("//! Each cell is authored for the pixel aspect (width over height) of its AI-55 anchor on")
    w("//! the 384x512 avatar quad, see [`DecoCell::aspect`], so it is drawn unstretched there.")
    w("")
    w("/// Atlas edge in pixels.")
    w(f"pub const ATLAS_SIZE: u32 = {ATLAS};")
    w("/// Cell edge in pixels.")
    w(f"pub const CELL_SIZE: u32 = {CELL};")
    w("/// Transparent gutter on every side of a cell's art.")
    w(f"pub const GUTTER: u32 = {GUTTER};")
    w("/// Edge of the art area inside a cell.")
    w(f"pub const ART_SIZE: u32 = {ART};")
    w("/// Number of grid cells (4x4); the last cells are unused and fully transparent.")
    w("pub const CELL_COUNT: usize = 16;")
    w("")
    w("/// Number of cells that hold art.")
    w(f"pub const USED_CELLS: usize = {len(CELLS)};")
    w("")
    w("/// A cell that holds art, in grid order (row-major).")
    w("#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]")
    w("pub enum DecoCell {")
    for n in names:
        w(f"    {n},")
    w("}")
    w("")
    w("impl DecoCell {")
    w("    /// Every used cell in grid order.")
    w("    pub const ALL: [DecoCell; USED_CELLS] = [")
    for n in names:
        w(f"        DecoCell::{n},")
    w("    ];")
    w("")
    w("    /// The badge shapes in `UserDecor.badge` order.")
    w("    pub const BADGES: [DecoCell; 10] = [")
    for n in names[4:]:
        w(f"        DecoCell::{n},")
    w("    ];")
    w("")
    w("    /// Grid index, 0 to 13 (row-major, 4 cells per row).")
    w("    pub const fn index(self) -> usize {")
    w("        self as usize")
    w("    }")
    w("")
    w("    /// Pixel rectangle `(x0, y0, x1, y1)` of the art area in the atlas.")
    w("    pub const fn px(self) -> [u32; 4] {")
    w("        PX[self as usize]")
    w("    }")
    w("")
    w("    /// Normalised rectangle `(u0, v0, u1, v1)` of the art area (`v` down).")
    w("    pub const fn uv(self) -> [f32; 4] {")
    w("        UV[self as usize]")
    w("    }")
    w("")
    w("    /// Pixel aspect (width over height) of the AI-55 anchor this art was drawn for.")
    w("    pub const fn aspect(self) -> f32 {")
    w("        ASPECT[self as usize]")
    w("    }")
    w("}")
    w("")
    w("#[rustfmt::skip]")
    w("const PX: [[u32; 4]; USED_CELLS] = [")
    for i in range(len(CELLS)):
        ox, oy = cell_origin(i)
        w(f"    [{ox + GUTTER}, {oy + GUTTER}, {ox + GUTTER + ART}, {oy + GUTTER + ART}], // {CELLS[i][0]}")
    w("];")
    w("")
    w("#[rustfmt::skip]")
    w("const UV: [[f32; 4]; USED_CELLS] = [")
    for i in range(len(CELLS)):
        ox, oy = cell_origin(i)
        r = [(ox + GUTTER) / ATLAS, (oy + GUTTER) / ATLAS,
             (ox + GUTTER + ART) / ATLAS, (oy + GUTTER + ART) / ATLAS]
        w("    [" + ", ".join(repr(v) for v in r) + f"], // {CELLS[i][0]}")
    w("];")
    w("")
    w("#[rustfmt::skip]")
    w("const ASPECT: [f32; USED_CELLS] = [")
    for name, _, _ in CELLS:
        key = "badge" if name.startswith("badge") else name
        w(f"    {aspect_of(key):.4f}, // {name}")
    w("];")
    w("")
    w("#[cfg(test)]")
    w('#[path = "deco_atlas_tests.rs"]')
    w("mod tests;")
    return "\n".join(out) + "\n"


# ---------------------------------------------------------------- preview

def preview(atlas, path):
    avatar = Image.open(AVATAR_PATH).convert("RGBA")
    scale = 2
    pad = 90 * scale
    W, H = AVATAR_W * scale + 2 * pad, AVATAR_H * scale + 2 * pad
    bg = (28, 30, 38, 255)

    def place(canvas, name, cell_index, tint, flip=False):
        u0, v0, u1, v1 = ANCHORS[name]
        ox, oy = cell_origin(cell_index)
        art = atlas.crop((ox + GUTTER, oy + GUTTER, ox + GUTTER + ART, oy + GUTTER + ART))
        w, h = int((u1 - u0) * AVATAR_W * scale), int((v1 - v0) * AVATAR_H * scale)
        art = art.resize((w, h), Image.BILINEAR)
        tr, tg, tb = tint
        tinted = Image.merge("RGBA", (art.getchannel("R").point(lambda p: p * tr // 255),
                                      art.getchannel("G").point(lambda p: p * tg // 255),
                                      art.getchannel("B").point(lambda p: p * tb // 255),
                                      art.getchannel("A")))
        layer = Image.new("RGBA", canvas.size, (0, 0, 0, 0))
        layer.paste(tinted, (pad + int(u0 * AVATAR_W * scale), pad + int(v0 * AVATAR_H * scale)))
        return Image.alpha_composite(canvas, layer)

    def figure(back, front):
        canvas = Image.new("RGBA", (W, H), bg)
        for name, idx, tint in back:
            canvas = place(canvas, name, idx, tint)
        av = Image.new("RGBA", (W, H), (0, 0, 0, 0))
        av.paste(avatar.resize((AVATAR_W * scale, AVATAR_H * scale), Image.BILINEAR), (pad, pad))
        canvas = Image.alpha_composite(canvas, av)
        for name, idx, tint in front:
            canvas = place(canvas, name, idx, tint)
        return canvas

    assisted = figure(
        [("cape", 0, (0xD9, 0x30, 0x30))],
        [("shades", 2, (0x1A, 0x1A, 0x24)), ("badge", 4 + 5, (0xE6, 0x9F, 0x00))])
    agent = figure(
        [("halo", 1, (0xFF, 0xE0, 0x80))],
        [("badge", 4 + 3, (0x56, 0xB4, 0xE9))])
    wizard = figure(
        [],
        [("hat", 3, (0xA7, 0x8B, 0xFA)), ("shades", 2, (0xB0, 0xB0, 0xC0)),
         ("badge", 4 + 9, (0xF0, 0xE4, 0x42))])
    sheet = Image.new("RGBA", (W * 3, H), bg)
    for i, f in enumerate((assisted, agent, wizard)):
        sheet.paste(f, (W * i, 0))
    # the atlas itself over a checkerboard, below, at 2x
    big = atlas.resize((ATLAS * 2, ATLAS * 2), Image.NEAREST)
    check = Image.new("RGBA", big.size, bg)
    d = ImageDraw.Draw(check)
    for y in range(0, big.size[1], 16):
        for x in range(0, big.size[0], 16):
            if (x // 16 + y // 16) % 2:
                d.rectangle([x, y, x + 15, y + 15], fill=(52, 56, 68, 255))
    check = Image.alpha_composite(check, big)
    out = Image.new("RGBA", (sheet.size[0], sheet.size[1] + check.size[1]), bg)
    out.paste(sheet, (0, 0))
    out.paste(check, (0, sheet.size[1]))
    out.convert("RGB").save(path)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--preview", metavar="PNG", help="also write a composite preview")
    args = ap.parse_args()
    atlas = build_atlas()
    check(atlas)
    atlas.save(PNG_PATH, format="PNG", optimize=True)
    with open(RS_PATH, "w", newline="\n") as f:
        f.write(rust_table())
    if args.preview:
        preview(atlas, args.preview)
    print("wrote", os.path.relpath(PNG_PATH, ROOT), "and", os.path.relpath(RS_PATH, ROOT))


if __name__ == "__main__":
    sys.exit(main())
