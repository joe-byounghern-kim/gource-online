#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Compare port renders (render.mjs) with the C++ reference frames.

    python3 scripts/parity/compare.py PORT_DIR [--ref DIR] [--out DIR]
        [--sheets N] [--label TEXT] [scenario ...]

For every scenario directory in PORT_DIR (target/parity/port-glibc/<scenario>/)
and every frame-NNNNN.png it holds, the reference frame of the same name is
loaded from --ref (default target/reference or $PARITY_REF) and compared:

  mad       mean absolute difference over all pixels and channels, 0..255
  mad_r/g/b the same per channel
  diff24    share of pixels whose largest channel difference exceeds 24
  mad8      mean absolute difference after both frames are box-filtered to
            1/8 size (160x90): structure and colour, insensitive to sub-pixel
            and text rasterisation differences
  ssim      mean SSIM of the luma over 8x8 blocks (non-overlapping, the
            standard constants for 8-bit data)
  ink_ref / ink_port  share of pixels that differ from the background colour
            (the top-left pixel) by more than 8

Writes metrics.json and metrics.csv per scenario, a summary.md table over all
scenarios to --out (default PORT_DIR), and side-by-side sheets
(sheet-NNNNN.jpg: reference | port | difference x4) for N evenly spaced frames
(--sheets, default 3: first non-empty, middle, last).

Needs only Pillow (no numpy).
"""

import argparse
import csv
import json
import os
import sys

from PIL import Image, ImageChops, ImageDraw, ImageMath, ImageStat

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))


def load(path):
    im = Image.open(path)
    if im.mode != "RGB":
        im = im.convert("RGB")
    return im


def ink(im):
    bg = im.getpixel((0, 0))
    diff = ImageChops.difference(im, Image.new("RGB", im.size, bg))
    r, g, b = diff.split()
    m = ImageChops.lighter(ImageChops.lighter(r, g), b)
    hist = m.histogram()
    return sum(hist[9:]) / (im.size[0] * im.size[1])


def block_ssim(a, b, block=8):
    """Mean SSIM of luma over non-overlapping block x block windows."""
    w, h = a.size
    bw, bh = w // block, h // block
    fa = a.convert("L").convert("F")
    fb = b.convert("L").convert("F")
    small = (bw, bh)

    def mean(im):
        return list(im.resize(small, Image.BOX).getdata())

    def prod(x, y):
        if hasattr(ImageMath, "lambda_eval"):  # Pillow >= 10.3
            return ImageMath.lambda_eval(lambda args: args["x"] * args["y"], x=x, y=y)
        return ImageMath.eval("x * y", x=x, y=y)

    ma, mb = mean(fa), mean(fb)
    maa, mbb, mab = mean(prod(fa, fa)), mean(prod(fb, fb)), mean(prod(fa, fb))
    c1, c2 = (0.01 * 255) ** 2, (0.03 * 255) ** 2
    total = 0.0
    for i in range(len(ma)):
        ux, uy = ma[i], mb[i]
        vx, vy, cxy = maa[i] - ux * ux, mbb[i] - uy * uy, mab[i] - ux * uy
        total += ((2 * ux * uy + c1) * (2 * cxy + c2)) / ((ux * ux + uy * uy + c1) * (vx + vy + c2))
    return total / len(ma)


def compare(ref, port):
    diff = ImageChops.difference(ref, port)
    stat = ImageStat.Stat(diff)
    r, g, b = diff.split()
    m = ImageChops.lighter(ImageChops.lighter(r, g), b)
    hist = m.histogram()
    n = ref.size[0] * ref.size[1]
    small = (ref.size[0] // 8, ref.size[1] // 8)
    d8 = ImageChops.difference(ref.resize(small, Image.BOX), port.resize(small, Image.BOX))
    return {
        "mad": sum(stat.mean) / 3,
        "mad_r": stat.mean[0],
        "mad_g": stat.mean[1],
        "mad_b": stat.mean[2],
        "diff24": sum(hist[25:]) / n,
        "mad8": sum(ImageStat.Stat(d8).mean) / 3,
        "ssim": block_ssim(ref, port),
        "ink_ref": ink(ref),
        "ink_port": ink(port),
    }, diff


def sheet(ref, port, diff, title, path, scale=0.5):
    w, h = int(ref.size[0] * scale), int(ref.size[1] * scale)
    boost = diff.point(lambda v: min(255, v * 4))
    out = Image.new("RGB", (w * 3, h + 22), (24, 24, 24))
    for i, (im, name) in enumerate(((ref, "C++ reference"), (port, "port"), (boost, "|difference| x4"))):
        out.paste(im.resize((w, h), Image.LANCZOS), (i * w, 22))
        ImageDraw.Draw(out).text((i * w + 6, 5), name, fill=(230, 230, 230))
    ImageDraw.Draw(out).text((w * 3 - 8 - 6 * len(title), 5), title, fill=(230, 230, 230))
    out.save(path, quality=88)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("port")
    ap.add_argument("scenarios", nargs="*")
    ap.add_argument("--ref", default=os.environ.get("PARITY_REF", os.path.join(ROOT, "target", "reference")))
    ap.add_argument("--out")
    ap.add_argument("--sheets", type=int, default=3)
    ap.add_argument("--label", default="")
    a = ap.parse_args()
    out_root = a.out or a.port
    scenarios = a.scenarios or sorted(
        d for d in os.listdir(a.port) if os.path.isdir(os.path.join(a.port, d)) and os.path.isdir(os.path.join(a.ref, d))
    )
    keys = ["mad", "mad_r", "mad_g", "mad_b", "diff24", "mad8", "ssim", "ink_ref", "ink_port"]
    summary = []
    for sc in scenarios:
        pdir, rdir, odir = os.path.join(a.port, sc), os.path.join(a.ref, sc), os.path.join(out_root, sc)
        os.makedirs(odir, exist_ok=True)
        frames = sorted(f for f in os.listdir(pdir) if f.startswith("frame-") and f.endswith(".png"))
        rows = []
        for f in frames:
            ref, port = load(os.path.join(rdir, f)), load(os.path.join(pdir, f))
            if ref.size != port.size:
                sys.exit(f"{sc}/{f}: size {port.size} != reference {ref.size}")
            m, _ = compare(ref, port)
            m["frame"] = int(f[6:11])
            rows.append(m)
        if not rows:
            continue
        with open(os.path.join(odir, "metrics.json"), "w") as fh:
            json.dump(rows, fh, indent=1)
        with open(os.path.join(odir, "metrics.csv"), "w", newline="") as fh:
            wr = csv.DictWriter(fh, ["frame"] + keys)
            wr.writeheader()
            for r in rows:
                wr.writerow({k: (round(v, 5) if isinstance(v, float) else v) for k, v in r.items()})
        # sheets: first frame with something besides the date drawn, middle, last
        if a.sheets > 0:
            live = [r for r in rows if r["ink_ref"] > 0.004] or rows
            picks = sorted({live[round(i * (len(live) - 1) / max(1, a.sheets - 1))]["frame"] for i in range(a.sheets)})
            for fr in picks:
                f = f"frame-{fr:05d}.png"
                ref, port = load(os.path.join(rdir, f)), load(os.path.join(pdir, f))
                _, diff = compare(ref, port)
                sheet(ref, port, diff, f"{sc} frame {fr} {a.label}".strip(), os.path.join(odir, f"sheet-{fr:05d}.jpg"))
        mean = {k: sum(r[k] for r in rows) / len(rows) for k in keys}
        worst = max(rows, key=lambda r: r["mad8"])
        summary.append((sc, len(rows), mean, worst))
        print(
            f"{sc:28s} n={len(rows):4d} mad={mean['mad']:.3f} diff24={mean['diff24'] * 100:.3f}% "
            f"mad8={mean['mad8']:.3f} ssim={mean['ssim']:.4f} worst mad8={worst['mad8']:.3f}@{worst['frame']}"
        )
    with open(os.path.join(out_root, "summary.md"), "w") as fh:
        fh.write("| scenario | frames | MAD | MAD R/G/B | >24 levels | MAD 1/8 | block SSIM | worst MAD 1/8 (frame) |\n")
        fh.write("|---|---:|---:|---|---:|---:|---:|---:|\n")
        for sc, n, m, w in summary:
            fh.write(
                f"| {sc} | {n} | {m['mad']:.3f} | {m['mad_r']:.2f}/{m['mad_g']:.2f}/{m['mad_b']:.2f} | "
                f"{m['diff24'] * 100:.3f}% | {m['mad8']:.3f} | {m['ssim']:.4f} | {w['mad8']:.3f} ({w['frame']}) |\n"
            )


if __name__ == "__main__":
    main()
