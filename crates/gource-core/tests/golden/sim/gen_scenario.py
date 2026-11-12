#!/usr/bin/env python3
# Writes scenario.log, the input of the sim golden harness (see ../README.md).
lines = []
t = 1600000000


def add(ts, user, action, path, colour=None):
    s = f"{ts}|{user}|{action}|{path}"
    if colour:
        s += f"|{colour}"
    lines.append(s)


# initial import by alice: 36 files over several dirs in one commit
dirs = ["src/core", "src/ui", "src/ui/widgets", "docs", "tests/unit", "assets/img"]
exts = ["c", "h", "md", "png", "rs", "txt"]
for i in range(36):
    d = dirs[i % len(dirs)]
    add(t, "alice", "A", f"{d}/file{i}.{exts[(i*5) % len(exts)]}")
t += 3600
# bob modifies a few files, one with an explicit colour
for i in range(0, 12, 2):
    add(t, "bob", "M", f"{dirs[i % len(dirs)]}/file{i}.{exts[(i*5) % len(exts)]}", "FF8800" if i == 4 else None)
t += 1800
# carol adds tests while dave deletes docs
for i in range(8):
    add(t, "carol", "A", f"tests/integration/case{i}.rs")
add(t + 60, "dave", "D", "docs/file3.c")
add(t + 60, "dave", "D", "docs/file9.md")
t += 86400 * 3  # long gap: auto-skip
for i in range(10):
    add(t + i * 600, ["erin", "frank"][i % 2], "M" if i % 3 else "A", f"src/core/extra{i}.c")
t += 86400
# directory delete of src/ui/widgets/
add(t, "alice", "D", "src/ui/widgets/")
# time travel: an older commit
add(t - 86400 * 2, "bob", "A", "src/old/legacy.c")
t += 600
# many users touch the same files
for k, u in enumerate(["alice", "bob", "carol", "dave", "erin", "frank", "gina"]):
    add(t + k, u, "M", "src/core/file0.c")
    add(t + k, u, "A", f"misc/{u}/notes{k}.txt")
t += 86400
# a file that becomes a directory
add(t, "gina", "A", "lib/thing")
add(t + 30, "gina", "A", "lib/thing/inner.c")
t += 4000
for i in range(20):
    add(t + i * 300, "hank", "A", f"deep/a/b/c/d/e/f{i}.go")
open("scenario.log", "w").write("\n".join(lines) + "\n")
