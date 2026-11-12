#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Run the reference harness over cases.txt and print expected.txt.

usage: regen.py HARNESS CASES REPO_ROOT

cases.txt: one case per line, "name<TAB>spec", '#' starts a comment line.
  spec is  file:PATH   a file, relative to REPO_ROOT/tests
           str:TEXT    bytes with \\n \\r \\t \\0 \\\\ and \\xNN escapes
           hex:DIGITS  bytes in hex
"""
import os
import subprocess
import sys


def unescape(text):
    out = bytearray()
    i = 0
    raw = text.encode("utf-8")
    while i < len(raw):
        c = raw[i]
        if c != 0x5C:  # backslash
            out.append(c)
            i += 1
            continue
        n = raw[i + 1]
        i += 2
        if n == ord("n"):
            out.append(10)
        elif n == ord("r"):
            out.append(13)
        elif n == ord("t"):
            out.append(9)
        elif n == ord("0"):
            out.append(0)
        elif n == ord("\\"):
            out.append(0x5C)
        elif n == ord("x"):
            out.append(int(raw[i : i + 2], 16))
            i += 2
        else:
            raise ValueError("bad escape in %r" % text)
    return bytes(out)


def load(spec, root):
    kind, _, rest = spec.partition(":")
    if kind == "file":
        with open(os.path.join(root, "tests", rest), "rb") as f:
            return f.read()
    if kind == "str":
        return unescape(rest)
    if kind == "hex":
        return bytes.fromhex(rest)
    raise ValueError("unknown spec %r" % spec)


def main():
    harness, cases_path, root = sys.argv[1:4]
    names, framed = [], bytearray()
    with open(cases_path, encoding="utf-8") as f:
        for line in f:
            line = line.rstrip("\n")
            if not line or line.startswith("#"):
                continue
            name, _, spec = line.partition("\t")
            data = load(spec, root)
            names.append(name)
            framed += b"%d\n" % len(data) + data
    result = subprocess.run([harness], input=bytes(framed), stdout=subprocess.PIPE, check=True)
    blocks = result.stdout.decode("utf-8").split("END\n")
    assert blocks[-1] == "" and len(blocks) == len(names) + 1, "harness output does not match the cases"
    for name, block in zip(names, blocks):
        sys.stdout.write("@ %s\n%s" % (name, block))


main()
