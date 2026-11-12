// SPDX-License-Identifier: GPL-3.0-or-later

//! [`RecordDriver`]: the recorder engine (`recording.md` sections 5.3, 5.4
//! and 5.15), generic over backend and clock so it runs natively in tests.
//!
//! A driver owns a second [`Engine`] with its own copy of the event log, the
//! settings snapshot of [`RecordSetup`] and the output size. It has no
//! input methods: hover, selection, the text box and the splash cannot
//! occur in a recording (REC-40). No wall time reaches the simulation; every
//! tick is the plan's fixed dt (REC-36).
//!
//! Lifecycle: created in the configuring state (the dialog may call
//! [`RecordDriver::reconfigure`] and [`RecordDriver::preview_for`]), then
//! [`RecordDriver::begin_recording`] restarts the run from the start and
//! [`RecordDriver::step_frame`] renders one output frame per call until it
//! returns the [`LAST`] flag. After a preview, `begin_recording` gives the
//! same frames as a fresh driver (REC-120).

use std::fmt::Debug;

use gource_core::settings::Settings;
use gource_core::view_hash::hash_view;
use gource_render::Backend;

use crate::engine::{Clock, Engine};
use crate::record::json::{Value, number, object, string};
use crate::record::setup::{LiveView, RecordRequest, RecordSetup};
use crate::record::source::LogSource;
use crate::record::stop::{EndReason, LAST, Progress, StopState};
use crate::record::{RecordError, codes};

/// Where the driver is in its lifecycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Created or reconfigured; nothing rendered for the video yet.
    Configuring,
    /// A preview is being computed (`target` ticks, `done` so far).
    Previewing {
        target: u64,
        done: u64,
    },
    /// The preview of `target` ticks is on the canvas.
    Previewed {
        target: u64,
    },
    Recording,
    /// The last frame was rendered.
    Ended,
}

impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Mode::Configuring => "configuring",
            Mode::Previewing { .. } => "previewing",
            Mode::Previewed { .. } => "previewed",
            Mode::Recording => "recording",
            Mode::Ended => "ended",
        }
    }
}

/// The recorder.
pub struct RecordDriver<B: Backend, C: Clock> {
    engine: Engine<B, C>,
    base: Settings,
    live: LiveView,
    setup: RecordSetup,
    stop: StopState,
    progress: Progress,
    mode: Mode,
    exact_frames: Option<u64>,
    last_flags: u32,
    /// Whether the last finished preview showed any user or file.
    preview_saw_entities: Option<bool>,
    saw_entities: bool,
}

fn gl_error(e: impl Debug) -> RecordError {
    RecordError::new(
        codes::GL_FAILED,
        format!("The graphics context failed; the recording stopped ({e:?})."),
    )
}

impl<B: Backend, C: Clock> RecordDriver<B, C>
where
    B::Error: Debug,
{
    /// Fork a recording of `source`'s log with `base` (the live settings
    /// snapshot) and `request`, drawing with `backend`.
    ///
    /// Errors: `record_log_incomplete`, `record_invalid_option`,
    /// `record_unbounded`, `record_size_unsupported` (also when the backend
    /// allocates a smaller drawable than asked for), `record_empty_run`,
    /// `record_gl_failed`.
    pub fn create(
        source: &impl LogSource,
        base: Settings,
        live: LiveView,
        request: &RecordRequest,
        backend: B,
        clock: C,
    ) -> Result<Self, RecordError> {
        let log = source.clone_log();
        let setup = RecordSetup::resolve(&base, request, live, &log)?;
        check_not_empty(&setup)?;
        let sim = setup.new_simulation();
        let engine = Engine::for_recording(backend, clock, log, sim);
        let mut driver = RecordDriver {
            stop: StopState::new(setup.stop),
            progress: new_progress(&setup),
            engine,
            base,
            live,
            setup,
            mode: Mode::Configuring,
            exact_frames: None,
            last_flags: 0,
            preview_saw_entities: None,
            saw_entities: false,
        };
        driver.apply_output_size()?;
        Ok(driver)
    }

    /// Tick length and output size of the setup (after a create or a
    /// reconfigure).
    fn apply_output_size(&mut self) -> Result<(), RecordError> {
        self.engine
            .set_fixed_dt(f64::from(self.setup.plan.fixed_dt));
        let (w, h, ui) = (self.setup.width, self.setup.height, self.setup.ui_scale);
        self.engine.set_output_size(w, h, ui).map_err(gl_error)?;
        if let Some([bw, bh]) = self.engine.backend().drawing_buffer_size()
            && [bw, bh] != [w, h]
        {
            return Err(RecordError::new(
                codes::SIZE_UNSUPPORTED,
                format!("This size is not supported on this device (largest: {bw}x{bh})."),
            ));
        }
        Ok(())
    }

    /// New options: the setup is resolved again, the simulation restarts,
    /// the output is resized and drawn. Cancels a preview; on error the
    /// driver keeps its previous setup.
    pub fn reconfigure(&mut self, request: &RecordRequest) -> Result<(), RecordError> {
        let setup = RecordSetup::resolve(&self.base, request, self.live, self.engine.log())?;
        check_not_empty(&setup)?;
        self.engine.replace_simulation(setup.new_simulation());
        self.stop = StopState::new(setup.stop);
        self.progress = new_progress(&setup);
        self.setup = setup;
        self.mode = Mode::Configuring;
        self.exact_frames = None;
        self.last_flags = 0;
        self.preview_saw_entities = None;
        self.apply_output_size()
    }

    pub fn setup(&self) -> &RecordSetup {
        &self.setup
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn engine(&self) -> &Engine<B, C> {
        &self.engine
    }

    pub fn engine_mut(&mut self) -> &mut Engine<B, C> {
        &mut self.engine
    }

    /// The plan JSON (with the exact frame count once a dry run gave it).
    pub fn plan_value(&self) -> Value {
        self.setup.plan_value(self.exact_frames)
    }

    /// The dry run's frame count: it becomes the progress denominator and
    /// the plan's exact length.
    pub fn set_exact_frames(&mut self, frames: u64) {
        self.exact_frames = (frames > 0).then_some(frames);
        self.progress.set_exact_frames(frames);
    }

    /// Render the run `sim_seconds` of video after its start, without
    /// capturing, spending at most `budget_ms` per call (it may overrun by
    /// one tick). Returns true once the preview is on the canvas; calls
    /// with the same `sim_seconds` continue the same preview. A preview
    /// cannot run while recording.
    pub fn preview_for(&mut self, sim_seconds: f64, budget_ms: f64) -> Result<bool, RecordError> {
        if self.mode == Mode::Recording {
            return Err(RecordError::new(
                codes::BUSY,
                "a preview cannot run while recording",
            ));
        }
        let seconds = if sim_seconds.is_finite() {
            sim_seconds.clamp(0.0, f64::from(crate::record::options::HARD_MAX_SECONDS))
        } else {
            0.0
        };
        let target = (seconds * f64::from(self.setup.plan.tick_hz)).round() as u64;
        let mut done = match self.mode {
            Mode::Previewed { target: t } if t == target => return Ok(true),
            Mode::Previewing { target: t, done } if t == target => done,
            _ => {
                self.engine.replace_simulation(self.setup.new_simulation());
                self.saw_entities = false;
                0
            }
        };
        let dt = self.setup.plan.fixed_dt;
        let started = self.engine.clock().now_ms();
        while done < target {
            self.engine.tick_without_drawing(dt);
            done += 1;
            let view = self.engine.scene_view();
            self.saw_entities |= !view.users.is_empty() || !view.files.is_empty();
            if self.engine.clock().now_ms() - started >= budget_ms {
                break;
            }
        }
        if done < target {
            self.mode = Mode::Previewing { target, done };
            return Ok(false);
        }
        self.engine.render().map_err(gl_error)?;
        self.mode = Mode::Previewed { target };
        self.preview_saw_entities = Some(self.saw_entities);
        Ok(true)
    }

    /// Start the recording from the beginning of the run: a fresh
    /// simulation from the snapshot, frame and pts counters and the
    /// progress at zero. (The page also resets the glyph atlas.)
    pub fn begin_recording(&mut self) {
        self.engine.replace_simulation(self.setup.new_simulation());
        self.stop = StopState::new(self.setup.stop);
        self.progress.reset();
        self.last_flags = 0;
        self.mode = Mode::Recording;
    }

    /// Run the ticks of the next output frame and draw it once. Returns the
    /// frame's flags ([`LAST`], `KEY`, `IN_TAIL`, `LIMIT`, `HARD_CAP`).
    /// Does not allocate once the run is warmed up.
    pub fn step_frame(&mut self) -> Result<u32, RecordError> {
        if self.mode != Mode::Recording {
            return Err(RecordError::new(
                codes::NOT_STARTED,
                "step_frame needs begin_recording first",
            ));
        }
        let plan = self.setup.plan;
        let n = self.stop.frames();
        self.engine.step(plan.ticks_to_step(n)).map_err(gl_error)?;
        let sim = self.engine.simulation();
        let finished = sim.finished(self.engine.log());
        let app_finished = sim.app_finished();
        let cursor = sim.commit_cursor();
        let flags = self
            .stop
            .after_frame(finished, app_finished, plan.is_keyframe(n));
        self.progress.update(n, cursor, &self.stop);
        if flags & LAST != 0 {
            self.mode = Mode::Ended;
        }
        self.last_flags = flags;
        Ok(flags)
    }

    /// Index of the frame just rendered (`None` before the first).
    pub fn frame_index(&self) -> Option<u64> {
        self.stop.frames().checked_sub(1)
    }

    /// Frames rendered since `begin_recording`.
    pub fn frames(&self) -> u64 {
        self.stop.frames()
    }

    /// Presentation time of the frame just rendered, in microseconds.
    pub fn pts_us(&self) -> u64 {
        self.frame_index().map_or(0, |n| self.setup.plan.pts_us(n))
    }

    /// Duration of the frame just rendered: up to the next frame's pts, the
    /// nominal period for the last one (REC-33).
    pub fn duration_us(&self) -> u64 {
        let plan = &self.setup.plan;
        match self.frame_index() {
            Some(n) if self.last_flags & LAST == 0 => plan.duration_us(n),
            _ => plan.nominal_duration_us(),
        }
    }

    /// Monotone progress, 0 to 1.
    pub fn progress(&self) -> f64 {
        self.progress.value()
    }

    /// The pre-encode frame hash: `hash_view` of the last extracted scene.
    pub fn frame_hash(&self) -> u64 {
        hash_view(self.engine.scene_view())
    }

    /// Why the run ended, once it has.
    pub fn ended(&self) -> Option<EndReason> {
        self.stop.ended()
    }

    /// Counters for the dialog and the progress panel.
    pub fn stats_value(&self) -> Value {
        let s = self.engine.status();
        let range = self.setup.estimate.range;
        let cursor = self.engine.simulation().commit_cursor();
        object([
            ("ticks", number(s.ticks as f64)),
            ("frames", number(self.frames() as f64)),
            ("files", number(s.files as f64)),
            ("users", number(s.users as f64)),
            ("dirs", number(s.dirs as f64)),
            ("commits", number(cursor.saturating_sub(range.start) as f64)),
            ("commit_cursor", number(cursor as f64)),
            ("draw_calls", number(f64::from(s.draw_calls))),
            ("vertices", number(f64::from(s.vertices))),
            (
                "atlas_evictions",
                number(f64::from(self.engine.backend().atlas_evictions())),
            ),
            (
                "preview_empty",
                self.preview_saw_entities
                    .map_or(Value::Null, |saw| Value::Bool(!saw)),
            ),
        ])
    }

    /// State of the run for the page (4 Hz).
    pub fn status_value(&self) -> Value {
        let s = self.engine.status();
        let flags = self.last_flags;
        object([
            ("state", string(self.mode.name())),
            (
                "frame_index",
                self.frame_index().map_or(Value::Null, |n| number(n as f64)),
            ),
            ("frames", number(self.frames() as f64)),
            (
                "exact_frames",
                self.exact_frames.map_or(Value::Null, |n| number(n as f64)),
            ),
            ("progress", number(self.progress())),
            ("pts_us", number(self.pts_us() as f64)),
            ("duration_us", number(self.duration_us() as f64)),
            ("flags", number(f64::from(flags))),
            (
                "in_tail",
                Value::Bool(flags & crate::record::stop::IN_TAIL != 0),
            ),
            (
                "ended",
                self.ended().map_or(Value::Null, |e| {
                    string(e.limit_name().unwrap_or("finished"))
                }),
            ),
            ("date", string(&s.date)),
            ("time", number(s.time as f64)),
        ])
    }

    /// The sidecar's `result` fields the recorder knows (REC-70): frame
    /// count, duration, whether the count was known in advance, the limit
    /// that ended the run and the glyph atlas evictions.
    pub fn result_value(&self) -> Value {
        let frames = self.frames();
        object([
            ("frames", number(frames as f64)),
            (
                "duration_s",
                number(self.setup.plan.pts_us(frames) as f64 / 1e6),
            ),
            ("exact", Value::Bool(self.exact_frames.is_some())),
            (
                "limit",
                self.ended()
                    .and_then(EndReason::limit_name)
                    .map_or(Value::Null, string),
            ),
            (
                "atlas_evictions",
                number(f64::from(self.engine.backend().atlas_evictions())),
            ),
        ])
    }
}

fn check_not_empty(setup: &RecordSetup) -> Result<(), RecordError> {
    if setup.estimate.empty() {
        Err(RecordError::new(
            codes::EMPTY_RUN,
            "The selected range has no commits.",
        ))
    } else {
        Ok(())
    }
}

fn new_progress(setup: &RecordSetup) -> Progress {
    let e = &setup.estimate;
    Progress::new(
        e.expected.clone(),
        e.range.start,
        e.about_s,
        e.tail_s,
        &setup.plan,
        &setup.stop,
    )
}
