#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Turn the PPM stream of `gource --output-ppm-stream -` into PNG frames.

Reads concatenated binary PPM (P6) images from stdin, writes every Nth frame and
the last frame as <out-dir>/frame-%05d.png (the number is the index in the
stream, starting at 0), plus:

  manifest.json   frame size, timing model, and per saved frame: index, number
                  of simulation ticks, simulated runtime, share of non-background
                  pixels, sha256 of the raw RGB
  frames.sha256   sha256 of the raw RGB of *every* frame (not only saved ones),
                  so two runs can be compared with `diff`
  contact-sheet.png  a grid of evenly spaced saved frames (needs Pillow)

Only the standard library is needed for the frames; Pillow is optional and used
for the contact sheet.

Timing model (see README.md): Gource's exporter ticks the simulation with a
fixed dt of 1/(fps * m) where m = ceil(60 / fps), and writes every m-th tick.
Frame k is therefore the state after k*m + 1 ticks.
"""

import argparse
import hashlib
import json
import math
import os
import struct
import sys
import zlib


class StreamError(Exception):
    pass


def read_ppm_header(fp):
    """Return (width, height) of the next image, or None at a clean end of stream."""
    magic = fp.read(2)
    if not magic:
        return None
    if magic != b"P6":
        raise StreamError("bad PPM magic %r (is something else writing to stdout?)" % magic)

    values = []
    token = b""
    while True:
        c = fp.read(1)
        if not c:
            raise StreamError("truncated PPM header")
        if c == b"#":  # comment up to the end of the line
            while c not in (b"\n", b""):
                c = fp.read(1)
            continue
        if c.isspace():
            if token:
                values.append(int(token))
                token = b""
                if len(values) == 3:
                    break  # exactly one whitespace byte follows maxval
            continue
        if not c.isdigit():
            raise StreamError("bad PPM header byte %r" % c)
        token += c

    width, height, maxval = values
    if maxval != 255 or width <= 0 or height <= 0:
        raise StreamError("unsupported PPM header %dx%d maxval %d" % (width, height, maxval))
    return width, height


def png_bytes(width, height, rgb):
    """Encode 8-bit RGB as PNG (no filtering; the scenes compress very well)."""
    stride = width * 3
    raw = b"\x00" + b"\x00".join(rgb[i:i + stride] for i in range(0, height * stride, stride))

    def chunk(tag, data):
        return (struct.pack(">I", len(data)) + tag + data
                + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF))

    return (b"\x89PNG\r\n\x1a\n"
            + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(raw, 6))
            + chunk(b"IEND", b""))


def write_atomic(path, data):
    tmp = path + ".tmp"
    with open(tmp, "wb") as f:
        f.write(data)
    os.replace(tmp, path)


def ink_fraction(rgb, width, height):
    """Share of pixels whose colour differs from the top-left pixel (the background)."""
    n = width * height
    ref = rgb[0:3] * n
    if rgb == ref:
        return 0.0
    diff = (int.from_bytes(rgb, "little") ^ int.from_bytes(ref, "little")).to_bytes(n * 3, "little")
    changed = (int.from_bytes(diff[0::3], "little")
               | int.from_bytes(diff[1::3], "little")
               | int.from_bytes(diff[2::3], "little")).to_bytes(n, "little")
    return (n - changed.count(0)) / n


def contact_sheet(saved, out_path, wanted, columns, thumb_width, ticks_of, tick_dt):
    """Grid of up to `wanted` evenly spaced saved frames. Returns False without Pillow."""
    try:
        from PIL import Image, ImageDraw
    except ImportError:
        return False

    if len(saved) <= wanted:
        picks = list(saved)
    else:
        step = (len(saved) - 1) / (wanted - 1)
        picks = [saved[int(round(i * step))] for i in range(wanted)]

    resample = Image.Resampling.BOX if hasattr(Image, "Resampling") else Image.BOX
    label_h = 14
    first = Image.open(picks[0][1])
    thumb_h = max(1, round(first.height * thumb_width / first.width))
    cell_w, cell_h = thumb_width, thumb_h + label_h
    rows = math.ceil(len(picks) / columns)
    sheet = Image.new("RGB", (columns * cell_w, rows * cell_h), (24, 24, 24))
    draw = ImageDraw.Draw(sheet)

    for i, (index, path) in enumerate(picks):
        x, y = (i % columns) * cell_w, (i // columns) * cell_h
        with Image.open(path) as img:
            sheet.paste(img.convert("RGB").resize((thumb_width, thumb_h), resample), (x, y))
        runtime = ticks_of(index) * tick_dt
        draw.text((x + 4, y + thumb_h + 1), "frame %d  t=%.2fs" % (index, runtime), fill=(220, 220, 220))

    write_atomic(out_path, _png_from_pil(sheet))
    return True


def _png_from_pil(img):
    return png_bytes(img.width, img.height, img.tobytes())


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--out-dir", required=True)
    ap.add_argument("--every", type=int, default=15, help="save every Nth frame (default 15); the last frame is always saved")
    ap.add_argument("--fps", type=int, default=30, help="the --output-framerate Gource was run with")
    ap.add_argument("--prefix", default="frame-")
    ap.add_argument("--contact-frames", type=int, default=12, help="frames in the contact sheet (0 disables it)")
    ap.add_argument("--contact-columns", type=int, default=4)
    ap.add_argument("--contact-width", type=int, default=320)
    ap.add_argument("--require-ink", action="store_true", help="exit 3 if every saved frame is one flat colour")
    ap.add_argument("--scenario", default="", help="label for the summary line")
    args = ap.parse_args()

    if args.every < 1:
        ap.error("--every must be >= 1")

    # Gource's FrameExporter timing (Gource::setFrameExporter): the tick rate is
    # the smallest multiple of the video frame rate that is >= 60, and one frame
    # is written every `ticks_per_frame` ticks, starting with the first tick.
    ticks_per_frame = max(1, math.ceil(60 / args.fps))
    tick_rate = args.fps * ticks_per_frame
    tick_dt = 1.0 / tick_rate

    def ticks_of(index):
        return index * ticks_per_frame + 1

    os.makedirs(args.out_dir, exist_ok=True)
    fp = sys.stdin.buffer

    digests = []          # sha256 hex of every frame
    saved = []            # (index, path)
    entries = []          # manifest rows for saved frames
    size = None
    ink_values = []
    held = None           # most recent frame (index, rgb), saved at the end if needed
    held_saved = True
    truncated = False
    stream_error = None

    def save(index, rgb, width, height):
        name = "%s%05d.png" % (args.prefix, index)
        path = os.path.join(args.out_dir, name)
        write_atomic(path, png_bytes(width, height, rgb))
        ink = ink_fraction(rgb, width, height)
        ink_values.append(ink)
        saved.append((index, path))
        entries.append({
            "index": index,
            "file": name,
            "ticks": ticks_of(index),
            "runtime_s": round(ticks_of(index) * tick_dt, 6),
            "ink": round(ink, 6),
            "sha256": digests[index],
        })

    try:
        while True:
            header = read_ppm_header(fp)
            if header is None:
                break
            width, height = header
            if size is None:
                size = (width, height)
            elif size != (width, height):
                raise StreamError("frame size changed from %dx%d to %dx%d" % (size + (width, height)))

            expected = width * height * 3
            rgb = fp.read(expected)
            if len(rgb) != expected:
                truncated = True
                sys.stderr.write("ppm2png: stream ended in the middle of frame %d; dropping it\n" % len(digests))
                break

            index = len(digests)
            digests.append(hashlib.sha256(rgb).hexdigest())

            # Frames off the grid are dropped as soon as the next one arrives,
            # except that the final one must be saved once the stream ends.
            held = (index, rgb)
            held_saved = index % args.every == 0
            if held_saved:
                save(index, rgb, width, height)
    except StreamError as err:
        stream_error = str(err)
        sys.stderr.write("ppm2png: %s\n" % stream_error)
        if not digests:
            return 2

    if not digests:
        sys.stderr.write("ppm2png: no frames received; check gource.stderr.log\n")
        return 2

    if not held_saved:
        save(held[0], held[1], size[0], size[1])

    width, height = size
    stream_sha = hashlib.sha256("".join(digests).encode()).hexdigest()

    with open(os.path.join(args.out_dir, "frames.sha256"), "w") as f:
        for index, digest in enumerate(digests):
            f.write("%s  %d\n" % (digest, index))

    manifest = {
        "format": 1,
        "width": width,
        "height": height,
        "output_framerate": args.fps,
        "ticks_per_frame": ticks_per_frame,
        "tick_dt": "1/%d" % tick_rate,
        "timing": "frame k is the state after k*ticks_per_frame+1 simulation ticks of tick_dt each",
        "frames_total": len(digests),
        "frames_saved": len(entries),
        "every": args.every,
        "last_frame": len(digests) - 1,
        "truncated_stream": truncated,
        "stream_error": stream_error,
        "stream_sha256": stream_sha,
        "frames": entries,
    }
    write_atomic(os.path.join(args.out_dir, "manifest.json"),
                 (json.dumps(manifest, indent=2) + "\n").encode())

    sheet = False
    if args.contact_frames > 0:
        sheet = contact_sheet(saved, os.path.join(args.out_dir, "contact-sheet.png"),
                              args.contact_frames, args.contact_columns, args.contact_width,
                              ticks_of, tick_dt)
        if not sheet:
            sys.stderr.write("ppm2png: Pillow not available, no contact sheet\n")

    print("%sframes=%d saved=%d last=%d size=%dx%d fps=%d tick=1/%d ink=%.4f..%.4f stream=%s%s"
          % (args.scenario + " " if args.scenario else "",
             len(digests), len(entries), len(digests) - 1, width, height, args.fps, tick_rate,
             min(ink_values), max(ink_values), stream_sha[:12],
             " contact-sheet" if sheet else ""))

    if stream_error:
        return 4
    if args.require_ink and max(ink_values) == 0.0:
        sys.stderr.write("ppm2png: every saved frame is a single flat colour; GL probably failed\n")
        return 3
    return 0


if __name__ == "__main__":
    sys.exit(main())
