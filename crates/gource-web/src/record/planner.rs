// SPDX-License-Identifier: GPL-3.0-or-later

//! The exact dry run (`recording.md` section 5.9, REC-67): a simulation
//! driven exactly as the recorder drives it (same snapshot, same viewport
//! and `ui_scale`, per tick `update` then `extract_view`, the same stop
//! rules), without building or drawing frames. `extract_view` is required:
//! it mutates simulation state (visibility caches, scratch lists), so
//! leaving it out could change later ticks.
//!
//! The work is sliced ([`RecordPlanner::run_for`]) so the page stays
//! responsive and can cancel between slices.

use gource_core::events::EventLog;
use gource_core::settings::Settings;
use gource_core::sim::Simulation;
use gource_core::view::SceneView;

use crate::engine::Clock;
use crate::record::json::{Value, number, object, string};
use crate::record::setup::{LiveView, RecordRequest, RecordSetup, SIZE_BOUND_FACTOR};
use crate::record::source::LogSource;
use crate::record::stop::{EndReason, LAST, StopState};
use crate::record::{RecordError, codes};

/// A dry run in progress.
pub struct RecordPlanner<C: Clock> {
    log: EventLog,
    sim: Simulation,
    view: SceneView,
    setup: RecordSetup,
    stop: StopState,
    clock: C,
    /// Ticks still to run for the frame in progress (0: start a frame).
    frame_ticks_left: u32,
    ticks: u64,
    saw_entities: bool,
    done: bool,
}

impl<C: Clock> RecordPlanner<C> {
    /// Same inputs and errors as [`crate::record::driver::RecordDriver::create`].
    pub fn create(
        source: &impl LogSource,
        base: Settings,
        live: LiveView,
        request: &RecordRequest,
        clock: C,
    ) -> Result<Self, RecordError> {
        let log = source.clone_log();
        let setup = RecordSetup::resolve(&base, request, live, &log)?;
        if setup.estimate.empty() {
            return Err(RecordError::new(
                codes::EMPTY_RUN,
                "The selected range has no commits.",
            ));
        }
        Ok(RecordPlanner {
            sim: setup.new_simulation(),
            view: SceneView::default(),
            stop: StopState::new(setup.stop),
            setup,
            log,
            clock,
            frame_ticks_left: 0,
            ticks: 0,
            saw_entities: false,
            done: false,
        })
    }

    pub fn setup(&self) -> &RecordSetup {
        &self.setup
    }

    /// Run for about `budget_ms` (at most one tick more); returns the ticks
    /// run in this slice, 0 once done.
    pub fn run_for(&mut self, budget_ms: f64) -> u32 {
        if self.done {
            return 0;
        }
        let plan = self.setup.plan;
        let started = self.clock.now_ms();
        let mut ran = 0u32;
        loop {
            if self.frame_ticks_left == 0 {
                self.frame_ticks_left = plan.ticks_to_step(self.stop.frames());
            }
            self.sim.update(&self.log, plan.fixed_dt);
            self.sim.extract_view(&self.log, &mut self.view);
            self.saw_entities |= !self.view.users.is_empty() || !self.view.files.is_empty();
            self.frame_ticks_left -= 1;
            self.ticks += 1;
            ran = ran.saturating_add(1);
            if self.frame_ticks_left == 0 {
                let n = self.stop.frames();
                let flags = self.stop.after_frame(
                    self.sim.finished(&self.log),
                    self.sim.app_finished(),
                    plan.is_keyframe(n),
                );
                if flags & LAST != 0 {
                    self.done = true;
                    return ran;
                }
            }
            if self.clock.now_ms() - started >= budget_ms {
                return ran;
            }
        }
    }

    pub fn done(&self) -> bool {
        self.done
    }

    /// Frames counted so far (the exact count once [`Self::done`]).
    pub fn frames(&self) -> u64 {
        self.stop.frames()
    }

    /// Fraction of the upper bound covered so far, for a progress bar.
    pub fn progress(&self) -> f64 {
        if self.done {
            return 1.0;
        }
        let upper = self
            .setup
            .plan
            .frames_in(self.setup.estimate.upper_s)
            .max(1);
        (self.frames() as f64 / upper as f64).min(0.999)
    }

    /// `{frames, duration_s, exact, size_upper_bytes, limit, commits,
    /// empty, ticks, done}`; `empty` is true when no user or file ever
    /// appeared (the filters removed everything, REC-122).
    pub fn result_value(&self) -> Value {
        let plan = &self.setup.plan;
        let frames = self.frames();
        let duration_s = plan.pts_us(frames) as f64 / 1e6;
        let cursor = self.sim.commit_cursor();
        let size = SIZE_BOUND_FACTOR * self.setup.bitrate_bps as f64 * duration_s / 8.0;
        object([
            ("done", Value::Bool(self.done)),
            ("frames", number(frames as f64)),
            ("duration_s", number(duration_s)),
            ("exact", Value::Bool(self.done)),
            ("size_upper_bytes", number(size.ceil())),
            (
                "limit",
                self.stop
                    .ended()
                    .and_then(EndReason::limit_name)
                    .map_or(Value::Null, string),
            ),
            (
                "commits",
                number(cursor.saturating_sub(self.setup.estimate.range.start) as f64),
            ),
            ("empty", Value::Bool(self.done && !self.saw_entities)),
            ("ticks", number(self.ticks as f64)),
        ])
    }
}
