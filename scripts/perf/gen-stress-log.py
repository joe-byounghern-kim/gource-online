#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Write a deterministic synthetic custom log for stress benchmarks.

    scripts/perf/gen-stress-log.py [--files N] [--users N] [--commits N]
                                   [--days D] [--seed S] > stress.log

Defaults: 100 000 files in ~2 600 directories (depth 1-5), 500 users,
12 000 commits spread evenly over 5 days, so Gource's default speed
(10 seconds per day) plays the whole log in about 50 s, i.e. ~3 000 frames
at 1/60 s. The first 85 % of commits add the files (plus a few modifies);
the rest modify and delete existing files. Same arguments, same bytes.
"""

import argparse
import sys

EXTS = ["c", "h", "rs", "py", "js", "ts", "md", "txt", "json", "png", "cpp", "go",
        "java", "sh", "yml", "css", "html", "toml", "sql", "xml"]


class Lcg:
    """Numerical Recipes LCG: portable and reproducible."""

    def __init__(self, seed):
        self.state = seed & 0xFFFFFFFF

    def next(self, n):
        self.state = (self.state * 1664525 + 1013904223) & 0xFFFFFFFF
        return (self.state >> 8) % n


def build_dirs(rng, want):
    dirs = [""]
    frontier = [""]
    while len(dirs) < want:
        parent = frontier[rng.next(len(frontier))]
        depth = parent.count("/")
        name = f"{parent}/d{len(dirs)}"
        dirs.append(name)
        if depth < 4:
            frontier.append(name)
    return dirs[1:]


def main():
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--files", type=int, default=100_000)
    ap.add_argument("--users", type=int, default=500)
    ap.add_argument("--commits", type=int, default=12_000)
    ap.add_argument("--days", type=float, default=5.0)
    ap.add_argument("--seed", type=int, default=7)
    args = ap.parse_args()

    rng = Lcg(args.seed)
    dirs = build_dirs(rng, max(2, args.files // 38))
    users = [f"user{i:03d}" for i in range(args.users)]
    start = 1_600_000_000
    step = max(1, int(args.days * 86400 / max(1, args.commits)))
    add_commits = max(1, int(args.commits * 0.85))
    per_commit = max(1, -(-args.files // add_commits))

    out = sys.stdout
    live = []
    created = 0
    for c in range(args.commits):
        t = start + c * step
        # A few prolific users and a long tail, like real projects.
        u = users[min(rng.next(len(users)), rng.next(len(users)))]
        lines = []
        if c < add_commits and created < args.files:
            base = dirs[rng.next(len(dirs))]
            for _ in range(min(per_commit, args.files - created)):
                # Mostly one directory per commit, some spill to a sibling.
                d = base if rng.next(4) else dirs[rng.next(len(dirs))]
                path = f"{d}/f{created}.{EXTS[rng.next(len(EXTS))]}"
                live.append(path)
                created += 1
                lines.append(f"{t}|{u}|A|{path}")
            for _ in range(rng.next(3)):
                lines.append(f"{t}|{u}|M|{live[rng.next(len(live))]}")
        else:
            for _ in range(1 + rng.next(12)):
                if live and rng.next(10) == 0:
                    i = rng.next(len(live))
                    path = live[i]
                    live[i] = live[-1]
                    live.pop()
                    lines.append(f"{t}|{u}|D|{path}")
                elif live:
                    lines.append(f"{t}|{u}|M|{live[rng.next(len(live))]}")
        # A commit lists each path once.
        seen = set()
        for line in lines:
            key = line.rsplit("|", 1)[1]
            if key not in seen:
                seen.add(key)
                out.write(line + "\n")


if __name__ == "__main__":
    main()
