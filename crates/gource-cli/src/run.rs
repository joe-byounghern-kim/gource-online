// SPDX-License-Identifier: GPL-3.0-or-later

//! The options `simulate` and `bench` share: how to build the simulation.

use gource_core::settings::Settings;
use gource_core::sim::Simulation;

use crate::args::{Args, OptionSpec, parse_seconds};
use crate::error::CliError;

pub const DT: OptionSpec = OptionSpec::value("--dt");
pub const SEED: OptionSpec = OptionSpec::value("--seed");
pub const WIDTH: OptionSpec = OptionSpec::value("--width");
pub const HEIGHT: OptionSpec = OptionSpec::value("--height");

/// Default seconds per frame, as the text the user could type.
pub const DEFAULT_DT: &str = "1/60";
pub const DEFAULT_WIDTH: u32 = 1280;
pub const DEFAULT_HEIGHT: u32 = 720;
/// Largest viewport side accepted, in pixels.
const MAX_SIDE: u32 = 16384;

/// Fixed-step run configuration.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RunOptions {
    /// Seconds of real time per frame.
    pub dt: f32,
    /// `--seed`; `None` keeps the core's default hash seed.
    pub seed: Option<u32>,
    pub width: u32,
    pub height: u32,
}

impl RunOptions {
    /// Read `--dt`, `--seed`, `--width` and `--height`.
    pub fn from_args(args: &Args) -> Result<Self, CliError> {
        Ok(RunOptions {
            dt: args
                .seconds("--dt")?
                .unwrap_or_else(|| parse_seconds(DEFAULT_DT).expect("default dt parses")),
            seed: args.uint("--seed", 0..=u32::MAX)?,
            width: args.uint("--width", 1..=MAX_SIDE)?.unwrap_or(DEFAULT_WIDTH),
            height: args
                .uint("--height", 1..=MAX_SIDE)?
                .unwrap_or(DEFAULT_HEIGHT),
        })
    }

    /// A fresh simulation with the hash seed and viewport applied. Uses only
    /// public fields and methods, so it works with any core behind the
    /// `gource-core` contract.
    pub fn simulation(&self) -> Simulation {
        let mut settings = Settings::default();
        if let Some(seed) = self.seed {
            settings.hash_seed = seed;
        }
        let mut sim = Simulation::new(settings);
        sim.set_viewport(self.width, self.height, 1.0);
        sim
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use gource_core::events::EventLog;
    use gource_core::glam::Vec2;
    use gource_core::view::SceneView;

    use super::*;
    use crate::args::{Parsed, parse};

    fn options(args: &[&str]) -> Result<RunOptions, String> {
        let specs = [DT, SEED, WIDTH, HEIGHT];
        let args: Vec<OsString> = args.iter().map(OsString::from).collect();
        match parse("test", &args, &specs).unwrap() {
            Parsed::Args(a) => RunOptions::from_args(&a).map_err(|e| e.to_string()),
            Parsed::Help => unreachable!(),
        }
    }

    #[test]
    fn defaults() {
        let o = options(&[]).unwrap();
        assert_eq!(o.dt, 1.0_f32 / 60.0_f32);
        assert_eq!(o.seed, None);
        assert_eq!((o.width, o.height), (1280, 720));
    }

    #[test]
    fn explicit_values() {
        let o = options(&[
            "--dt", "1/30", "--seed", "7", "--width", "800", "--height", "600",
        ])
        .unwrap();
        assert_eq!(o.dt, 1.0_f32 / 30.0_f32);
        assert_eq!(o.seed, Some(7));
        assert_eq!((o.width, o.height), (800, 600));
    }

    #[test]
    fn bad_values_name_the_option() {
        assert!(options(&["--width", "0"]).unwrap_err().contains("--width"));
        assert!(
            options(&["--height", "99999"])
                .unwrap_err()
                .contains("--height")
        );
        assert!(options(&["--seed", "-1"]).unwrap_err().contains("--seed"));
        assert!(options(&["--dt", "0"]).unwrap_err().contains("--dt"));
    }

    #[test]
    fn the_viewport_reaches_the_scene_view() {
        let o = RunOptions {
            dt: 0.1,
            seed: None,
            width: 640,
            height: 480,
        };
        let mut view = SceneView::default();
        o.simulation().extract_view(&EventLog::new(), &mut view);
        assert_eq!(view.viewport, Vec2::new(640.0, 480.0));
    }

    #[test]
    fn seed_reaches_the_settings() {
        let o = RunOptions {
            dt: 0.1,
            seed: Some(1234),
            width: 640,
            height: 480,
        };
        assert_eq!(o.simulation().settings().hash_seed, 1234);
        let default = RunOptions { seed: None, ..o };
        assert_eq!(
            default.simulation().settings().hash_seed,
            Settings::default().hash_seed
        );
    }
}
