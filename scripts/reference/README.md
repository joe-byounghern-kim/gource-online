<!-- SPDX-License-Identifier: GPL-3.0-or-later -->
# Reference renders from the C++ Gource

Reproducible, headless renders of the unmodified C++ Gource (v0.57, the `src/`
of this repository) to PNG frames. They are the look reference for the
Rust/WebAssembly port (`docs/web-mvp/PLAN.md`, phase 3: side-by-side frames
against the C++ output): same log in, same settings, same simulation timing, so
a port run can be set up to produce the same frame.

Everything runs in Docker (Ubuntu 24.04, Xvfb, Mesa llvmpipe software OpenGL),
so no GPU, display or host package is involved.

## Quick start

```sh
scripts/reference/build.sh                  # once: image gource-reference:0.57, about 2 minutes, 1.4 GB
scripts/reference/render-all.sh             # all scenarios -> target/reference/<scenario>/, about 3 minutes, 100 MB
scripts/reference/render.sh my.log out-dir  # one log
scripts/reference/render.sh my.log out-dir --seconds-per-day 1   # extra gource arguments
```

Host requirements: `docker` (your user must be allowed to run it), `bash`,
`tar`, `sha256sum`. Python, Xvfb, Mesa and git are inside the image.
`render.sh` and `render-all.sh` build the image if it is missing.

`src/core` is a git submodule and is empty in secondary worktrees. `build.sh`
therefore takes the C++ sources from the checkout where it is initialised (this
one, otherwise the main worktree; override with `GOURCE_SRC=/path`) and copies
only `src/`, `data/`, `m4/`, `configure.ac`, `Makefile.am` and `autogen.sh` into
a temporary directory that becomes the Docker build context. `target/`,
`node_modules/` and `.git/` never reach Docker.

Rerun from scratch:

```sh
docker rmi gource-reference:0.57
rm -rf target/reference
scripts/reference/build.sh --no-cache       # optional: ignore Docker's layer cache too
scripts/reference/render-all.sh
```

## What runs

The image builds Gource the documented way: `autoreconf -f -i`, `./configure
--prefix=/opt/gource`, `make`, `make install-strip` (`autoreconf -f -i` is the
command `autogen.sh` runs; its other steps are `git submodule` calls and the
context already contains `src/core`).

`render.sh` copies the log to `<out-dir>/input.log`, starts a container with
that directory mounted at `/out` (running as your uid, so the output is owned by
you), and inside it runs

```sh
xvfb-run -a -s "-screen 0 1920x1080x24" \
  /ref/gource-run.sh 1800 /out/gource.stderr.log \
    --viewport 1280x720 --output-framerate 30 --output-ppm-stream - \
    --stop-at-end --hide mouse --disable-input --no-vsync \
    [extra gource args] /out/input.log \
  | python3 /ref/ppm2png.py --out-dir /out --every 15 --fps 30 --require-ink
```

with `LIBGL_ALWAYS_SOFTWARE=1 GALLIUM_DRIVER=llvmpipe LP_NUM_THREADS=4
MESA_SHADER_CACHE_DISABLE=true TZ=UTC LC_ALL=C.UTF-8` in the environment
(Mesa 25.2 llvmpipe, OpenGL 4.5 compatibility profile; Gource's shaders are
legacy GLSL 1.10 using `gl_Vertex`, `gl_TexCoord` and `ftransform`).

`gource-run.sh` is `timeout -k 10 1800 gource "$@" 2>>/out/gource.stderr.log`.
It exists because `xvfb-run` runs its command with `2>&1`: without it any
message gource or Mesa prints to stderr would end up inside the PPM stream.

| Flag | Why |
|---|---|
| `--viewport 1280x720` | Window and frame size. No HiDPI: with an explicit viewport Gource does not request a high-DPI drawable. Font scale is `1 + height / 1600` = 1. |
| `--output-framerate 30` | Frame rate of the PPM stream (25, 30 or 60). It also fixes the simulation tick, see the timing model. |
| `--output-ppm-stream -` | Binary PPM (P6) frames on stdout, top row first. |
| `--stop-at-end` | Stop when the log is exhausted and every user has left. Gource implies it for exports; it is spelled out for clarity. |
| `--hide mouse` | No cursor and no position slider (`hide mouse` also sets `hide progress`). |
| `--disable-input` | Ignore keyboard and mouse. |
| `--no-vsync` | Do not wait for display sync. |

The Xvfb screen is larger than the viewport so the GL window is never clipped
(pixels outside the screen are undefined for `glReadPixels`). `render.sh`
limits the container to 8 CPUs (`REF_CPUS`) and llvmpipe to 4 threads
(`REF_GL_THREADS`); the output does not depend on either (see Reproducibility).

## Output

`render.sh LOG OUT` and each `render-all.sh` scenario write to the output
directory:

```
frame-00000.png ...    every REF_EVERY-th frame (default 15) and the last frame; the number
                       is the frame's index in the stream, so frame-00030.png is frame 30
contact-sheet.png      12 evenly spaced saved frames, labelled with frame index and simulated time
input.log              the log that was rendered (for gource-history: the generated custom log)
command.txt            the exact gource command that ran (paths are the in-container ones)
environment.txt        Gource version, OpenGL strings, image id and source fingerprint
gource.stderr.log      stderr of gource (empty when all is well)
manifest.json          size, timing model, and for every saved frame: index, ticks, runtime_s,
                       ink (share of pixels that differ from the background, 0 = flat frame), sha256
frames.sha256          sha256 of the raw RGB of EVERY frame, "<hash>  <index>" per line
```

`render.sh` exits non-zero when gource fails, times out (exit 124) or every
saved frame is one flat colour (GL failure). On success it prints a summary
line such as `file-removal frames=432 saved=30 last=431 size=1280x720 fps=30
tick=1/60 ink=0.0021..0.0472 stream=8c436a493bb3`; `stream` is a short hash of
all frame hashes, handy for comparing runs.

## Timing model

Gource ignores the wall clock while exporting video. `Gource::update`
(`src/gource.cpp`) replaces the measured frame time by a fixed tick, and
`Gource::setFrameExporter` picks that tick: the smallest multiple of the video
frame rate that is at least 60 Hz. The exporter then writes every m-th update,
starting with the first.

| `--output-framerate` | tick `dt` | ticks per frame (m) | frame k is the state after |
|---|---|---|---|
| 60 | 1/60 s | 1 | k + 1 ticks |
| **30 (default here)** | **1/60 s** | **2** | **2k + 1 ticks** |
| 25 | 1/75 s | 3 | 3k + 1 ticks |

So at 30 fps a video frame is **two simulation ticks of dt = 1/60 s**, not one
tick of 1/30 s, and frame 0 is the state after the first tick. In general frame
k has simulated runtime `(m*k + 1) / (fps*m)` seconds, which is k/30 + 1/60 s
at 30 fps; `manifest.json` lists `ticks` and `runtime_s` for every saved frame.
The tick is a single-precision `1.0f / 60.0f`; the simulation clock `runtime`
is a float accumulated per tick.

Per tick (`Gource::logic`), with `time_scale` = 1:

- `runtime += dt * time_scale`; `--stop-at-time` compares against this runtime.
- The simulation clock advances by `dt * 86400 * days_per_second` seconds
  (whole seconds, the fraction carried in `subseconds`). `days_per_second` is
  `1 / seconds_per_day`, default 1/10, so 144 simulated seconds per tick.
- Commits with `timestamp <= currtime` are applied in that tick. On the first
  tick the clock is first set to the first commit's timestamp.
- The date label shows `currtime` while commits are queued ahead of it and the
  last applied commit's timestamp when the queue is empty. Frame 0 therefore
  shows the time of the first commit.
- Auto-skip jumps the clock to the next commit once every user has been idle
  for `auto_skip_seconds` (3) seconds of runtime; it is off once the end of the
  log or a stop condition has been reached.
- A run ends when the log is exhausted (or `--stop-at-time`, `--stop-date`,
  `--stop-position` is reached) and every user has left the scene, which
  Gource does 10 s after a user's last action. Most clips therefore end with
  several seconds of an empty scene and only the date label.

This was checked by experiment, not only by reading the code: a two-commit log
one day apart rendered with `--seconds-per-day 1` (1440 simulated seconds per
60 Hz tick) shows the clock at start, +4320 s, +7200 s, +10080 s in frames
0 to 3 at 30 fps (1, 3, 5, 7 ticks), start, +2880 s, +4320 s, +5760 s at
60 fps (1, 2, 3, 4 ticks) and start, +4608 s, +8064 s, +11520 s at 25 fps with
1152 s per tick (1, 4, 7, 10 ticks).

Caveat: frames are written only once the log has been loaded, but the update
counter that selects the exported ticks (every second update at 30 fps) starts
when Gource is created, and the log loader runs on its own thread. It almost
always finishes before the first update. If it loses that race by one update,
the whole run is sampled one tick later than the table says (frame k after
2k + 2 ticks). The pixels of each tick are unchanged, only the sampling phase
moves. At 60 fps every tick is exported, so the phase cannot move there:
`REF_FPS=60` is immune, and 30 fps frame k is 60 fps frame 2k (all 357 frames
of a probe run were compared). Reproducibility says how often the shift was
seen.

### Reproducing a frame with the port

To produce frame k of a scenario: take the settings from `command.txt` (the
default `Settings` plus the scenario's extra arguments, e.g.
`set_option("seconds-per-day", "0.15")`), `set_viewport(1280, 720, 1.0)`, feed
`input.log` into the `EventLog`, call `Simulation::update(&log, 1.0 / 60.0)`
`2k + 1` times and `extract_view` / build the frame after the last call. Dates
must be formatted in UTC (`%A, %d %B, %Y %X`, for example `Tuesday, 14 November,
2023 22:13:20`).

## Scenarios

`render-all.sh` renders these into `target/reference/<scenario>/` with the
default flags above plus the extra arguments in the third column (each
scenario's `command.txt` has the full command). Frame counts are at 30 fps.

| Scenario | Log | Extra arguments | Frames | Exercises |
|---|---|---|---|---|
| `custom-dir-delete` | `tests/logs/custom-dir-delete.log` | none | 420 | `D` on a directory path (`path/to/`) deleting everything under it |
| `file-removal` | `tests/logs/file-removal.log` | none | 432 | five files added, removed with beams |
| `file-to-dir` | `tests/logs/file-to-dir.log` | none | 411 | a path that is a file first and a directory later |
| `utf8` | `tests/logs/utf8.log` | none | 397 | non-ASCII user and file names |
| `visual-hierarchy-activity` | `tests/fixtures/custom-log/visual-hierarchy-activity.log` | none | 398 | three users, nested directories, create/modify/delete |
| `dense-contributors` | `tests/fixtures/custom-log/dense-contributors.log` | none | 398 | six users acting in the same seconds |
| `deletion-recreation` | `tests/fixtures/custom-log/deletion-recreation.log` | none | 398 | a file deleted by one user and recreated by another |
| `gource-history` | generated, see below | `--start-date 2009-09-01 --stop-date 2010-09-01 --seconds-per-day 0.15` | 2014 | a real project: the first year of Gource's own history (up to 9 contributors, about 1200 file events), 67 s of video |

Only custom-format logs are rendered: `tests/logs/svn-dir-delete.log` (SVN XML)
and `tests/logs/utf8-caption.log` (a caption file) are skipped. The tiny logs
span a few seconds of log time, so at the default 10 s per day all their
commits are applied within the first ticks; what the frames show is the users
walking to the files and the files fading away afterwards.

`gource-history` is this repository's history up to `70db547` ("Bump version
and fix ChangeLog", the last upstream Gource commit, v0.57; override with
`REF_HISTORY_REV`). It is the same project the web MVP's end-to-end test
analyses (`github.com/acaudwell/Gource`). The custom log is produced inside the
container by `gource --output-custom-log /out/input.log --git-branch 70db547
/repo` against a read-only mount of the main checkout, i.e. by Gource's own git
reader (`git log --reverse --raw --no-renames ...`), and is saved as
`target/reference/gource-history/input.log`. Pinning the revision keeps the
scenario stable while the fork gains commits.

To add a scenario, add an `add name path "args"` line to `render-all.sh` or call
`render.sh` directly.

## Reproducibility

Rendering is deterministic for a given image:

- In export mode the simulation runs on the fixed tick, not on wall-clock time.
- Gource's `rand()` is never seeded (`srand` is only called for `--start-position
  random`), so glibc's default seed applies; the colour hash seed is the fixed 31.
- The container pins `TZ=UTC` and `LC_ALL=C.UTF-8`, so date labels do not depend
  on the host.
- llvmpipe rasterises deterministically: re-rendering `dense-contributors`
  gave the same sha256 for all 398 frames on repeated runs, with 1, 4 and 16
  llvmpipe threads, and with 2 and 8 CPUs or no CPU limit.
- Deleting the image and `target/reference` and running `render-all.sh` again
  (it rebuilt the image first) reproduced the sha256 of every frame of all eight
  scenarios, 4868 frames including the 2014 of `gource-history`.

Compare two renders with `diff a/frames.sha256 b/frames.sha256`, or compare the
`stream=` value of the summary line.

Not covered: another Mesa or LLVM version (rebuilding the image later can pull
newer packages; `environment.txt` records the image id and the GL strings of
each render), and the loader race described under the timing model. How often
that happens was measured with a two-commit probe log, killed after a few
seconds, comparing the hash of frame 0: 114 of 115 runs had frame 0 = tick 1
(30 of 30 without a CPU limit, 16 of 16 at 1 CPU, 45 of 46 at half a CPU, 23 of
23 at a quarter of a CPU). The one exception, at half a CPU, was shifted by
exactly one tick for all 358 frames (its hashes equal the odd frames of the
60 fps stream). If a render must be certain, render twice and compare
`frames.sha256`, or use `REF_FPS=60`.

The port will not match these frames bit for bit: text is rasterised by
FreeType/FTGL here and by canvas 2D in the browser, and the rasteriser differs.
Compare layout, counts, colours and positions, with a tolerance.

## Environment variables

| Variable | Default | Meaning |
|---|---|---|
| `REF_WIDTH`, `REF_HEIGHT` | 1280, 720 | viewport |
| `REF_FPS` | 30 | `--output-framerate` (25, 30, 60) |
| `REF_EVERY` | 15 | save every Nth frame; the last frame is always saved; 1 saves all |
| `REF_TIMEOUT` | 1800 | seconds before gource is killed |
| `REF_CPUS` | 8 | docker `--cpus`, 0 for no limit |
| `REF_GL_THREADS` | 4 | llvmpipe threads (`LP_NUM_THREADS`) |
| `REF_IMAGE` | `gource-reference:0.57` | image name |
| `REF_OUT` | `target/reference` | output root of `render-all.sh` |
| `REF_HISTORY_REV` | `70db547` | revision of the `gource-history` scenario |
| `GOURCE_SRC` | auto | checkout with an initialised `src/core` (build) or `.git` (history) |
| `MAKE_JOBS` | 8 | parallel compile jobs in the image build |

## Troubleshooting

- `ppm2png: no frames received`, or every frame one flat colour: read
  `gource.stderr.log` in the output directory (`render.sh` prints its last lines
  on failure). Typical causes are a log Gource cannot read (`unsupported log
  format`, `no commits found`) or a GL failure; check the stack with
  `docker run --rm gource-reference:0.57 xvfb-run -a glxinfo -B`, and if Xvfb
  itself does not start, `docker run --rm gource-reference:0.57 xvfb-run -a -e
  /dev/stdout true` shows its messages (it prints harmless xkbcomp warnings).
- `ppm2png: bad PPM magic`: gource wrote text to stdout instead of frames, for
  example because an extra argument such as `--help` or `--output-custom-log -`
  makes it print and exit.
- Exit code 124: the run hit `REF_TIMEOUT`. Raise it or bound the run with
  `--stop-at-time SECONDS` or `--stop-date`.
- `permission denied` on the Docker socket: your user is not in the `docker`
  group.

## Files

| File | Role |
|---|---|
| `Dockerfile` | the image: Ubuntu 24.04, Gource built from this tree, Xvfb, Mesa, python3, git |
| `build.sh` | assembles the build context and runs `docker build` |
| `render.sh` | one log to frames |
| `render-all.sh` | every scenario |
| `common.sh` | helpers shared by the scripts |
| `container/render-inner.sh` | runs inside the image: Xvfb, gource, the converter |
| `container/gource-run.sh` | gource with a time limit and stderr in a file (see above) |
| `container/ppm2png.py` | PPM stream to PNG frames, manifest, hashes, contact sheet (standard library; Pillow only for the contact sheet) |
