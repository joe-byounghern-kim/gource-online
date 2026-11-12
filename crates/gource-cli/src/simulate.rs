// SPDX-License-Identifier: GPL-3.0-or-later

//! `gource-rs simulate`: headless run with a state hash per report.

use std::ffi::OsString;
use std::io::{self, BufWriter, Write};

use gource_core::view::SceneView;

use crate::args::{self, OptionSpec, Parsed};
use crate::error::CliError;
use crate::hash::hash_view;
use crate::load::load_custom_log;
use crate::run::{self, RunOptions};
use crate::streams;

const COMMAND: &str = "simulate";

pub const HELP: &str = "\
gource-rs simulate: run the simulation headless and print a state hash

USAGE:
    gource-rs simulate <custom.log> [OPTIONS]

ARGS:
    <custom.log>    custom log file, or '-' for standard input

OPTIONS:
        --frames <N>    frames to run, 1 or more                       [default: 600]
        --dt <S>        seconds per frame, a decimal or a fraction     [default: 1/60]
                        such as 1/120; above 0, at most 3600. As in Gource,
                        the simulation caps one step at 1/60 s.
        --seed <N>      colour hash seed (Gource's --hash-seed)        [default: 31]
        --width <W>     viewport width in pixels, 1 to 16384           [default: 1280]
        --height <H>    viewport height in pixels, 1 to 16384          [default: 720]
        --every <K>     report every K frames and after the last one   [default: 60]
    -h, --help          print this help

The whole log is loaded first (complete and seekable), then every frame runs
Simulation::update and extract_view at the fixed step --dt. Each report is
one line:

    frame=<n> time=<t> files=<f> users=<u> dirs=<d> hash=<16 hex digits>

frame is the number of frames run so far, time the simulation clock (unix
seconds), files/users/dirs the entity counts, and hash a 64-bit FNV-1a
fingerprint of the SceneView at that frame: dirs, files, users, actions,
labels and HUD text, and the camera, with every float as its bit pattern.
The same log and options always print the same lines.
";

const FRAMES: OptionSpec = OptionSpec::value("--frames");
const EVERY: OptionSpec = OptionSpec::value("--every");
const OPTIONS: &[OptionSpec] = &[FRAMES, EVERY, run::DT, run::SEED, run::WIDTH, run::HEIGHT];

const DEFAULT_FRAMES: u32 = 600;
const DEFAULT_EVERY: u32 = 60;

pub fn run(args: Vec<OsString>) -> Result<(), CliError> {
    let args = match args::parse(COMMAND, &args, OPTIONS)? {
        Parsed::Help => return streams::print_stdout(HELP),
        Parsed::Args(args) => args,
    };
    let [path] = args.positionals(["<custom.log>"])?;
    let frames = args
        .uint("--frames", 1..=u32::MAX)?
        .unwrap_or(DEFAULT_FRAMES);
    let every = args.uint("--every", 1..=u32::MAX)?.unwrap_or(DEFAULT_EVERY);
    let options = RunOptions::from_args(&args)?;

    let loaded = load_custom_log(path)?;
    let mut sim = options.simulation();
    let mut view = SceneView::default();
    let mut out = BufWriter::with_capacity(streams::BUFFER, io::stdout().lock());

    for frame in 1..=frames {
        sim.update(&loaded.log, options.dt);
        sim.extract_view(&loaded.log, &mut view);
        if frame % every == 0 || frame == frames {
            let status = sim.status(&loaded.log);
            writeln!(
                out,
                "frame={frame} time={} files={} users={} dirs={} hash={:016x}",
                status.current_time,
                status.files,
                status.users,
                status.dirs,
                hash_view(&view),
            )
            .map_err(streams::write_error)?;
        }
    }
    out.flush().map_err(streams::write_error)
}
