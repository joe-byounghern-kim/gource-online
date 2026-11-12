// SPDX-License-Identifier: GPL-3.0-or-later

//! `gource-rs bench`: per-phase timings of the frame pipeline.

use std::ffi::{OsStr, OsString};
use std::fmt::Write as _;
use std::hint::black_box;
use std::path::Path;
use std::time::{Duration, Instant};

use gource_core::events::EventLog;
use gource_core::sim::Simulation;
use gource_core::view::SceneView;
use gource_render::{Frame, FrameBuilder, MonospaceGlyphs};

use crate::args::{self, OptionSpec, Parsed};
use crate::error::CliError;
use crate::hash::hash_view;
use crate::json::Json;
use crate::load::{Loaded, load_custom_log};
use crate::run::{self, RunOptions};
use crate::stats::Summary;
use crate::streams;

const COMMAND: &str = "bench";

pub const HELP: &str = "\
gource-rs bench: time the simulation and frame-building phases

USAGE:
    gource-rs bench <custom.log> [OPTIONS]

ARGS:
    <custom.log>    custom log file, or '-' for standard input

OPTIONS:
        --frames <N>    frames to run, 1 to 10000000                   [default: 3000]
        --dt <S>        seconds per frame, a decimal or a fraction     [default: 1/60]
                        such as 1/120; above 0, at most 3600. As in Gource,
                        the simulation caps one step at 1/60 s.
        --seed <N>      colour hash seed (Gource's --hash-seed)        [default: 31]
        --width <W>     viewport width in pixels, 1 to 16384           [default: 1280]
        --height <H>    viewport height in pixels, 1 to 16384          [default: 720]
        --json          print one JSON document instead of the table
    -h, --help          print this help

The whole log is loaded first, then every frame runs, each timed on its own
with a monotonic clock:

    update          Simulation::update
    extract_view    Simulation::extract_view
    build           FrameBuilder::build (monospace glyph stand-in, no GPU)
    frame           the three together

For each phase the report gives the total, mean, median (p50), 95th
percentile (p95) and maximum in milliseconds; percentiles are nearest-rank,
so they are measured values. It ends with the entity counts of the last
frame (simulation, scene view, built frame) and the FNV-1a hash of the last
scene view, the value `simulate` prints for the same frame, and the peak
entity counts over the run. Warm-up frames are included. Build with --release for numbers that mean anything.
";

const FRAMES: OptionSpec = OptionSpec::value("--frames");
const JSON: OptionSpec = OptionSpec::switch("--json");
const GOURCE_OPTIONS: OptionSpec = OptionSpec::value("--options");
const OPTIONS: &[OptionSpec] = &[
    FRAMES,
    JSON,
    GOURCE_OPTIONS,
    run::DT,
    run::SEED,
    run::WIDTH,
    run::HEIGHT,
];

const DEFAULT_FRAMES: u32 = 3000;
/// Every frame's timings are kept, so the count is bounded.
const MAX_FRAMES: u32 = 10_000_000;

pub fn run(args: Vec<OsString>) -> Result<(), CliError> {
    let args = match args::parse(COMMAND, &args, OPTIONS)? {
        Parsed::Help => return streams::print_stdout(HELP),
        Parsed::Args(args) => args,
    };
    let [path] = args.positionals(["<custom.log>"])?;
    let frames = args
        .uint("--frames", 1..=MAX_FRAMES)?
        .unwrap_or(DEFAULT_FRAMES);
    let options = RunOptions::from_args(&args)?;
    let json = args.switch("--json");
    let gource_options = parse_gource_options(args.value("--options").unwrap_or(""))?;

    let loaded = load_custom_log(path)?;
    let mut sim = options.simulation();
    for (name, value) in &gource_options {
        sim.set_option(name, value).map_err(|e| {
            CliError::usage(
                COMMAND,
                format!("invalid Gource option '{name}={value}': {e}"),
            )
        })?;
    }
    sim.restart(&loaded.log);
    let report = measure(path, &loaded, sim, &options, frames);
    let text = if json {
        report.to_json().to_pretty_string()
    } else {
        report.to_text()
    };
    streams::print_stdout(&text)
}

/// `name=value&name=value` pairs; empty pieces are skipped.
fn parse_gource_options(query: &str) -> Result<Vec<(String, String)>, CliError> {
    query
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((n, v)) if !n.is_empty() => Ok((n.to_string(), v.to_string())),
            _ => Err(CliError::usage(
                COMMAND,
                format!("invalid --options entry '{p}': expected name=value"),
            )),
        })
        .collect()
}

/// Largest value of each count over the run.
#[derive(Clone, Copy, Debug, Default)]
struct Peaks {
    files: usize,
    users: usize,
    dirs: usize,
    view_files: usize,
    view_labels: usize,
    quads: usize,
    text_quads: usize,
}

impl Peaks {
    fn record(&mut self, sim: &Simulation, log: &EventLog, view: &SceneView, frame: &Frame) {
        let s = sim.status(log);
        let max = |a: &mut usize, b: usize| *a = (*a).max(b);
        max(&mut self.files, s.files);
        max(&mut self.users, s.users);
        max(&mut self.dirs, s.dirs);
        max(&mut self.view_files, view.files.len());
        max(&mut self.view_labels, view.labels.len());
        max(&mut self.quads, frame.quads.len() / 4);
        max(&mut self.text_quads, frame.text.len() / 4);
    }
}

/// Everything `bench` reports.
struct Report {
    log_path: String,
    log_bytes: u64,
    log_commits: u64,
    log_changes: u64,
    load_ms: f64,

    frames: u32,
    dt: f32,
    seed: u32,
    width: u32,
    height: u32,
    wall_ms: f64,

    update: Summary,
    extract_view: Summary,
    build: Summary,
    frame: Summary,

    time: i64,
    commits_processed: usize,
    files: usize,
    users: usize,
    dirs: usize,

    view_dirs: usize,
    view_files: usize,
    view_users: usize,
    view_actions: usize,
    view_labels: usize,

    quads: usize,
    bloom_quads: usize,
    text_quads: usize,
    draw_commands: usize,

    hash: u64,
    peaks: Peaks,
}

fn millis(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn measure(
    path: &OsStr,
    loaded: &Loaded,
    mut sim: Simulation,
    options: &RunOptions,
    frames: u32,
) -> Report {
    let mut peaks = Peaks::default();
    let mut view = SceneView::default();
    let mut frame = Frame::default();
    let mut builder = FrameBuilder::new();
    let mut glyphs = MonospaceGlyphs;

    let capacity = frames.min(1 << 20) as usize;
    let mut update = Vec::with_capacity(capacity);
    let mut extract_view = Vec::with_capacity(capacity);
    let mut build = Vec::with_capacity(capacity);
    let mut whole = Vec::with_capacity(capacity);

    let wall = Instant::now();
    for _ in 0..frames {
        let t0 = Instant::now();
        sim.update(&loaded.log, options.dt);
        let t1 = Instant::now();
        sim.extract_view(&loaded.log, &mut view);
        let t2 = Instant::now();
        builder.build(&view, sim.settings(), options.dt, &mut glyphs, &mut frame);
        let t3 = Instant::now();

        update.push(millis(t1 - t0));
        extract_view.push(millis(t2 - t1));
        build.push(millis(t3 - t2));
        whole.push(millis(t3 - t0));
        peaks.record(&sim, &loaded.log, &view, &frame);
    }
    let wall_ms = millis(wall.elapsed());
    black_box((&view, &frame));

    let status = sim.status(&loaded.log);
    Report {
        log_path: Path::new(path).display().to_string(),
        log_bytes: loaded.stats.bytes,
        log_commits: loaded.stats.commits,
        log_changes: loaded.stats.files,
        load_ms: millis(loaded.elapsed),

        frames,
        dt: options.dt,
        seed: sim.settings().hash_seed,
        width: options.width,
        height: options.height,
        wall_ms,

        update: Summary::of(&update),
        extract_view: Summary::of(&extract_view),
        build: Summary::of(&build),
        frame: Summary::of(&whole),

        time: status.current_time,
        commits_processed: status.commits_processed,
        files: status.files,
        users: status.users,
        dirs: status.dirs,

        view_dirs: view.dirs.len(),
        view_files: view.files.len(),
        view_users: view.users.len(),
        view_actions: view.actions.len(),
        view_labels: view.labels.len(),

        quads: frame.quads.len() / 4,
        bloom_quads: frame.bloom.len() / 4,
        text_quads: frame.text.len() / 4,
        draw_commands: frame.commands.len(),

        hash: hash_view(&view),
        peaks,
    }
}

impl Report {
    fn phases(&self) -> [(&'static str, &Summary); 4] {
        [
            ("update", &self.update),
            ("extract_view", &self.extract_view),
            ("build", &self.build),
            ("frame", &self.frame),
        ]
    }

    fn to_text(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "log         {}: {} bytes, {} commits, {} file changes, loaded in {:.3} ms",
            self.log_path, self.log_bytes, self.log_commits, self.log_changes, self.load_ms
        );
        let _ = writeln!(
            s,
            "run         {} frames at dt {} s ({:.1} s simulated), viewport {}x{}, seed {}, \
             {:.3} ms wall",
            self.frames,
            self.dt,
            f64::from(self.frames) * f64::from(self.dt),
            self.width,
            self.height,
            self.seed,
            self.wall_ms
        );
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "{:<14}{:>13}{:>12}{:>12}{:>12}{:>12}",
            "phase", "total ms", "mean ms", "p50 ms", "p95 ms", "max ms"
        );
        for (name, p) in self.phases() {
            let _ = writeln!(
                s,
                "{:<14}{:>13.3}{:>12.6}{:>12.6}{:>12.6}{:>12.6}",
                name, p.total, p.mean, p.p50, p.p95, p.max
            );
        }
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "last frame  time {}, {} commits played; {} files, {} users, {} dirs",
            self.time, self.commits_processed, self.files, self.users, self.dirs
        );
        let _ = writeln!(
            s,
            "view        {} dirs, {} files, {} users, {} actions, {} labels",
            self.view_dirs, self.view_files, self.view_users, self.view_actions, self.view_labels
        );
        let _ = writeln!(
            s,
            "built       {} quads, {} bloom quads, {} text quads, {} draw commands",
            self.quads, self.bloom_quads, self.text_quads, self.draw_commands
        );
        let p = &self.peaks;
        let _ = writeln!(
            s,
            "peak        {} files, {} users, {} dirs; view {} files, {} labels; \
             {} quads, {} text quads",
            p.files, p.users, p.dirs, p.view_files, p.view_labels, p.quads, p.text_quads
        );
        let _ = writeln!(s, "hash        {:016x}", self.hash);
        s
    }

    fn to_json(&self) -> Json {
        let phase = |p: &Summary| {
            Json::Object(vec![
                ("total_ms", Json::millis(p.total)),
                ("mean_ms", Json::millis(p.mean)),
                ("p50_ms", Json::millis(p.p50)),
                ("p95_ms", Json::millis(p.p95)),
                ("max_ms", Json::millis(p.max)),
            ])
        };
        let count = |n: usize| Json::uint(n as u64);
        Json::Object(vec![
            (
                "log",
                Json::Object(vec![
                    ("path", Json::string(self.log_path.as_str())),
                    ("bytes", Json::uint(self.log_bytes)),
                    ("commits", Json::uint(self.log_commits)),
                    ("changes", Json::uint(self.log_changes)),
                    ("load_ms", Json::millis(self.load_ms)),
                ]),
            ),
            (
                "run",
                Json::Object(vec![
                    ("frames", Json::uint(u64::from(self.frames))),
                    ("dt", Json::float(self.dt)),
                    ("seed", Json::uint(u64::from(self.seed))),
                    ("width", Json::uint(u64::from(self.width))),
                    ("height", Json::uint(u64::from(self.height))),
                    ("wall_ms", Json::millis(self.wall_ms)),
                ]),
            ),
            (
                "phases",
                Json::Object(
                    self.phases()
                        .into_iter()
                        .map(|(name, p)| (name, phase(p)))
                        .collect(),
                ),
            ),
            (
                "final",
                Json::Object(vec![
                    ("time", Json::int(self.time)),
                    ("commits_processed", count(self.commits_processed)),
                    ("hash", Json::string(format!("{:016x}", self.hash))),
                ]),
            ),
            (
                "entities",
                Json::Object(vec![
                    ("files", count(self.files)),
                    ("users", count(self.users)),
                    ("dirs", count(self.dirs)),
                ]),
            ),
            (
                "view",
                Json::Object(vec![
                    ("dirs", count(self.view_dirs)),
                    ("files", count(self.view_files)),
                    ("users", count(self.view_users)),
                    ("actions", count(self.view_actions)),
                    ("labels", count(self.view_labels)),
                ]),
            ),
            (
                "output",
                Json::Object(vec![
                    ("quads", count(self.quads)),
                    ("bloom_quads", count(self.bloom_quads)),
                    ("text_quads", count(self.text_quads)),
                    ("draw_commands", count(self.draw_commands)),
                ]),
            ),
            (
                "peak",
                Json::Object(vec![
                    ("files", count(self.peaks.files)),
                    ("users", count(self.peaks.users)),
                    ("dirs", count(self.peaks.dirs)),
                    ("view_files", count(self.peaks.view_files)),
                    ("view_labels", count(self.peaks.view_labels)),
                    ("quads", count(self.peaks.quads)),
                    ("text_quads", count(self.peaks.text_quads)),
                ]),
            ),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> Report {
        let p = |n: f64| Summary {
            total: n * 10.0,
            mean: n,
            p50: n,
            p95: n * 2.0,
            max: n * 3.0,
        };
        Report {
            log_path: "a \"b\".log".into(),
            log_bytes: 100,
            log_commits: 4,
            log_changes: 9,
            load_ms: 0.5,
            frames: 10,
            dt: 1.0 / 60.0,
            seed: 31,
            width: 1280,
            height: 720,
            wall_ms: 12.0,
            update: p(0.1),
            extract_view: p(0.2),
            build: p(0.3),
            frame: p(0.6),
            time: 1_700_000_000,
            commits_processed: 4,
            files: 7,
            users: 2,
            dirs: 3,
            view_dirs: 3,
            view_files: 7,
            view_users: 2,
            view_actions: 1,
            view_labels: 12,
            quads: 20,
            bloom_quads: 3,
            text_quads: 40,
            draw_commands: 11,
            hash: 0xab,
            peaks: Peaks {
                files: 8,
                ..Peaks::default()
            },
        }
    }

    #[test]
    fn text_report_has_every_section() {
        let text = report().to_text();
        for needle in [
            "log         a \"b\".log: 100 bytes, 4 commits, 9 file changes",
            "run         10 frames at dt 0.016666668 s",
            "viewport 1280x720, seed 31",
            "phase",
            "update",
            "extract_view",
            "build",
            "frame",
            "7 files, 2 users, 3 dirs",
            "20 quads, 3 bloom quads, 40 text quads, 11 draw commands",
            "hash        00000000000000ab",
            "peak        8 files, 0 users",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in\n{text}");
        }
    }

    #[test]
    fn gource_options_parse_as_a_query() {
        let o = parse_gource_options("seconds-per-day=0.01&&hide=date,users").unwrap();
        assert_eq!(
            o,
            vec![
                ("seconds-per-day".to_string(), "0.01".to_string()),
                ("hide".to_string(), "date,users".to_string()),
            ]
        );
        assert!(parse_gource_options("").unwrap().is_empty());
        assert!(parse_gource_options("novalue").is_err());
        assert!(parse_gource_options("=1").is_err());
    }

    #[test]
    fn json_report_has_the_documented_shape() {
        let text = report().to_json().to_pretty_string();
        for needle in [
            "\"path\": \"a \\\"b\\\".log\"",
            "\"frames\": 10",
            "\"dt\": 0.016666668",
            "\"update\": {",
            "\"p95_ms\": 0.200000",
            "\"hash\": \"00000000000000ab\"",
            "\"draw_commands\": 11",
            "\"peak\": {",
        ] {
            assert!(text.contains(needle), "missing {needle:?} in\n{text}");
        }
    }
}
