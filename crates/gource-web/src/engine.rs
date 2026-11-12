// SPDX-License-Identifier: GPL-3.0-or-later

//! The per-page engine behind `GourceApp`, generic over the GPU backend and
//! the clock so it runs (and is tested) natively as well as in the browser.
//!
//! One display frame is `Gource::update` + `Gource::draw`:
//! `Simulation::update` → `Simulation::extract_view` → `FrameBuilder::build`
//! → `Backend::draw_frame`. Every buffer involved (`EventLog` aside) is
//! owned here and reused, so steady-state frames do not allocate in this
//! crate.

use gource_core::events::EventLog;
use gource_core::input::InputEvent;
use gource_core::settings::{OptionEffect, OptionError, Settings};
use gource_core::sim::Simulation;
use gource_core::view::SceneView;
use gource_render::{Backend, BackendStats, Frame, FrameBuilder};
use gource_sources::{BatchError, BatchSummary, apply_batch};

use crate::input::{WheelAccumulator, gource_reacts_to, map_button, map_buttons, map_key};
use crate::status::EngineStatus;

/// Milliseconds from a monotonic clock (`performance.now()` in the browser).
pub trait Clock {
    fn now_ms(&self) -> f64;
}

/// dt of the first frame, before two timestamps exist (Gource's tick).
pub const FIRST_FRAME_DT: f32 = 1.0 / 60.0;

/// Longest real frame gap passed on, in seconds. After a background tab
/// (no animation frames) the next timestamp can be minutes later; the
/// simulation clamps to 1/60 s itself, this keeps the HUD and the frame
/// statistics sane too.
pub const MAX_FRAME_GAP: f64 = 0.25;

/// Weight of a new sample in the frame-time moving averages.
const EMA_ALPHA: f64 = 0.1;

/// The host's local time zone: offset from UTC in minutes (local = UTC +
/// offset) in force at a unix time (`-new Date(ms).getTimezoneOffset()`).
pub type TimeZone = Box<dyn Fn(i64) -> i32>;

const DAY: i64 = 86_400;
/// Zone offsets are probed for 1900..2100 at most; timestamps outside use
/// the offset at the nearer end (a hostile timestamp must not stretch the
/// probe so far that its samples skip DST periods).
const PROBE_MIN: i64 = -2_208_988_800;
const PROBE_MAX: i64 = 4_102_444_800;
/// Extra years probed around the log's range, so start and stop dates near
/// it and a log that keeps growing need no new probe.
const PROBE_MARGIN: i64 = 2 * 366 * DAY;

/// Offset changes of `zone` in `lo..=hi`: the offset at `lo`, and every
/// `(instant, offset)` from which a new offset applies, in order. Samples
/// are a day apart (DST periods last months) and each change is bisected to
/// the second.
pub fn probe_utc_offsets(zone: &dyn Fn(i64) -> i32, lo: i64, hi: i64) -> (i32, Vec<(i64, i32)>) {
    let base = zone(lo);
    let mut changes = Vec::new();
    let (mut t, mut offset) = (lo, base);
    while t < hi {
        let next = t.saturating_add(DAY).min(hi);
        let o = zone(next);
        if o != offset {
            // first second in (t, next] with an offset other than `offset`
            let (mut a, mut b) = (t, next);
            while b - a > 1 {
                let mid = a + (b - a) / 2;
                if zone(mid) == offset {
                    a = mid;
                } else {
                    b = mid;
                }
            }
            let ob = zone(b);
            changes.push((b, ob));
            offset = ob;
            // a second change within the same day is found from `b` on
            t = b;
            continue;
        }
        t = next;
    }
    (base, changes)
}

/// Moving averages of the frame timings, in milliseconds.
#[derive(Clone, Debug, Default)]
struct Timings {
    fps: f64,
    frame_ms: f64,
    sim_ms: f64,
    build_ms: f64,
    gpu_ms: f64,
    /// The latest frame's timings, unsmoothed (benchmarks).
    last: [f64; 3],
    samples: u64,
}

impl Timings {
    fn record(&mut self, sim_ms: f64, build_ms: f64, gpu_ms: f64) {
        let ema = |avg: &mut f64, v: f64, first: bool| {
            *avg = if first {
                v
            } else {
                *avg + (v - *avg) * EMA_ALPHA
            };
        };
        let first = self.samples == 0;
        self.last = [sim_ms, build_ms, gpu_ms];
        ema(&mut self.sim_ms, sim_ms, first);
        ema(&mut self.build_ms, build_ms, first);
        ema(&mut self.gpu_ms, gpu_ms, first);
        ema(&mut self.frame_ms, sim_ms + build_ms + gpu_ms, first);
        self.samples += 1;
    }

    fn record_interval(&mut self, interval_ms: f64) {
        if interval_ms.is_nan() || interval_ms <= 0.0 || interval_ms > MAX_FRAME_GAP * 1000.0 {
            return;
        }
        let fps = 1000.0 / interval_ms;
        self.fps = if self.fps == 0.0 {
            fps
        } else {
            self.fps + (fps - self.fps) * EMA_ALPHA
        };
    }
}

/// `GourceApp` without the JavaScript bindings.
pub struct Engine<B: Backend, C: Clock> {
    log: EventLog,
    sim: Simulation,
    view: SceneView,
    builder: FrameBuilder,
    frame: Frame,
    backend: B,
    clock: C,

    /// `now_ms` of the previous `frame` call.
    last_frame_ms: Option<f64>,
    /// Seconds per simulation tick in deterministic mode; 0 = real time.
    fixed_dt: f32,
    ticks: u64,

    dpr: f64,
    device: [u32; 2],
    wheel: WheelAccumulator,

    timings: Timings,
    stats: BackendStats,

    /// Host time zone, probed over the log's time range.
    zone: Option<TimeZone>,
    /// Range covered by the simulation's offset table.
    zone_range: Option<(i64, i64)>,
    /// Earliest and latest timestamp of the log, over the first
    /// `zone_scanned` commits.
    log_range: Option<(i64, i64)>,
    zone_scanned: usize,
}

impl<B: Backend, C: Clock> Engine<B, C> {
    pub fn new(backend: B, clock: C, settings: Settings) -> Self {
        Engine {
            log: EventLog::new(),
            sim: Simulation::new(settings),
            view: SceneView::default(),
            builder: FrameBuilder::new(),
            frame: Frame::default(),
            backend,
            clock,
            last_frame_ms: None,
            fixed_dt: 0.0,
            ticks: 0,
            dpr: 1.0,
            device: [0, 0],
            wheel: WheelAccumulator::default(),
            timings: Timings::default(),
            stats: BackendStats::default(),
            zone: None,
            zone_range: None,
            log_range: None,
            zone_scanned: 0,
        }
    }

    /// Use `zone` for dates (Gource's `localtime`): the offsets in force
    /// across the log's time range are probed as commits arrive, so every
    /// date shows the DST of its own instant.
    pub fn set_time_zone(&mut self, zone: TimeZone) {
        self.zone = Some(zone);
        self.zone_range = None;
        self.update_zone();
    }

    /// Extend the offset table to the log's (new) time range.
    fn update_zone(&mut self) {
        let Some(zone) = self.zone.as_ref() else {
            return;
        };
        let count = self.log.commit_count();
        if self.zone_scanned > count {
            // a new log
            self.zone_scanned = 0;
            self.log_range = None;
        }
        for i in self.zone_scanned..count {
            let ts = self.log.commit(i).timestamp.clamp(PROBE_MIN, PROBE_MAX);
            self.log_range = Some(match self.log_range {
                Some((lo, hi)) => (lo.min(ts), hi.max(ts)),
                None => (ts, ts),
            });
        }
        self.zone_scanned = count;
        let Some((lo, hi)) = self.log_range else {
            return;
        };
        if self
            .zone_range
            .is_some_and(|(zlo, zhi)| zlo <= lo && hi <= zhi)
        {
            return;
        }
        let (mut lo, mut hi) = (lo, hi);
        if let Some((zlo, zhi)) = self.zone_range {
            (lo, hi) = (lo.min(zlo), hi.max(zhi));
        }
        let lo = lo.saturating_sub(PROBE_MARGIN).max(PROBE_MIN);
        let hi = hi.saturating_add(PROBE_MARGIN).min(PROBE_MAX);
        let (base, changes) = probe_utc_offsets(zone.as_ref(), lo, hi);
        self.zone_range = Some((lo, hi));
        self.sim.set_utc_offsets(base, changes);
    }

    pub fn log(&self) -> &EventLog {
        &self.log
    }

    pub fn simulation(&self) -> &Simulation {
        &self.sim
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn frame_data(&self) -> &Frame {
        &self.frame
    }

    // --- events ---

    /// A new log starts (new repository or a new parser): drop every event
    /// and entity.
    pub fn begin_log(&mut self) {
        self.log.clear();
        self.sim.restart(&self.log);
    }

    /// Append a batch from the worker's `LogParser`. On error (lost,
    /// duplicated or corrupt batch) the log is unchanged.
    pub fn append_batch(&mut self, batch: &[u8]) -> Result<BatchSummary, BatchError> {
        let summary = apply_batch(&mut self.log, batch)?;
        self.update_zone();
        Ok(summary)
    }

    /// The whole log has arrived: it becomes seekable.
    pub fn finish_log(&mut self, total_bytes: f64) {
        let total = if total_bytes.is_finite() && total_bytes > 0.0 {
            total_bytes as u64
        } else {
            0
        };
        self.log.set_total_bytes(total.max(self.log.total_bytes()));
        self.log.set_complete(true);
    }

    // --- loop ---

    /// Real seconds since the previous frame, clamped; also feeds the fps
    /// average.
    fn real_dt(&mut self, now_ms: f64) -> f32 {
        let dt = match self.last_frame_ms {
            Some(last) if now_ms.is_finite() => {
                let interval = now_ms - last;
                self.timings.record_interval(interval);
                (interval / 1000.0).clamp(0.0, MAX_FRAME_GAP) as f32
            }
            _ => FIRST_FRAME_DT,
        };
        if now_ms.is_finite() {
            self.last_frame_ms = Some(now_ms);
        }
        dt
    }

    /// One animation frame. In real-time mode the simulation advances by
    /// the time since the previous frame; in fixed-dt mode it only redraws
    /// (the host advances it with [`Engine::step`]).
    pub fn frame(&mut self, now_ms: f64) -> Result<(), B::Error> {
        let dt = self.real_dt(now_ms);
        let t0 = self.clock.now_ms();
        let build_dt = if self.fixed_dt > 0.0 {
            0.0
        } else {
            self.sim.update(&self.log, dt);
            self.ticks += 1;
            dt
        };
        self.sim.extract_view(&self.log, &mut self.view);
        let t1 = self.clock.now_ms();
        self.build(build_dt);
        let t2 = self.clock.now_ms();
        self.draw(t0, t1, t2)
    }

    /// Advance exactly `ticks` simulation ticks of the fixed dt (1/60 s when
    /// fixed-dt mode is off), then render one frame. Each tick is a full
    /// `Gource::update` + `Gource::draw` as in Gource's exporter (frame
    /// building included, so HUD animations match); only the last one is
    /// submitted to the GPU.
    pub fn step(&mut self, ticks: u32) -> Result<(), B::Error> {
        let dt = if self.fixed_dt > 0.0 {
            self.fixed_dt
        } else {
            FIRST_FRAME_DT
        };
        let t0 = self.clock.now_ms();
        for i in 0..ticks {
            self.sim.update(&self.log, dt);
            self.ticks += 1;
            if i + 1 < ticks {
                self.sim.extract_view(&self.log, &mut self.view);
                self.build(dt);
            }
        }
        self.sim.extract_view(&self.log, &mut self.view);
        let t1 = self.clock.now_ms();
        self.build(if ticks > 0 { dt } else { 0.0 });
        let t2 = self.clock.now_ms();
        self.draw(t0, t1, t2)
    }

    fn build(&mut self, dt: f32) {
        self.builder.build(
            &self.view,
            self.sim.settings(),
            dt,
            self.backend.glyphs(),
            &mut self.frame,
        );
    }

    fn draw(&mut self, t0: f64, t1: f64, t2: f64) -> Result<(), B::Error> {
        self.stats = self.backend.draw_frame(&self.frame)?;
        let t3 = self.clock.now_ms();
        self.timings.record(t1 - t0, t2 - t1, t3 - t2);
        Ok(())
    }

    /// `seconds` per tick for [`Engine::step`]; 0 returns to real time.
    /// Negative and non-finite values count as 0.
    pub fn set_fixed_dt(&mut self, seconds: f64) {
        self.fixed_dt = if seconds.is_finite() && seconds > 0.0 {
            seconds as f32
        } else {
            0.0
        };
    }

    pub fn fixed_dt(&self) -> f32 {
        self.fixed_dt
    }

    /// CSS size of the canvas and the device pixel ratio. The drawable is
    /// `round(css × dpr)` device pixels.
    ///
    /// Resizing a canvas clears its drawing buffer, and a `ResizeObserver`
    /// callback runs after the animation frame and before the page is
    /// composited, so the frame is drawn again at once (without advancing
    /// the simulation); otherwise every resize would show a blank canvas.
    pub fn resize(&mut self, css_width: f64, css_height: f64, dpr: f64) -> Result<(), B::Error> {
        let dpr = if dpr.is_finite() && dpr > 0.0 {
            dpr
        } else {
            1.0
        };
        let device = |css: f64| {
            if css.is_finite() && css > 0.0 {
                ((css * dpr).round() as u32).max(1)
            } else {
                1
            }
        };
        let (w, h) = (device(css_width), device(css_height));
        self.dpr = dpr;
        self.device = [w, h];
        self.backend.resize(w, h);
        self.sim.set_viewport(w, h, dpr as f32);
        self.redraw()
    }

    /// Render the current state again: no simulation tick, no animation,
    /// and not counted in the frame timings.
    fn redraw(&mut self) -> Result<(), B::Error> {
        self.sim.extract_view(&self.log, &mut self.view);
        self.build(0.0);
        self.stats = self.backend.draw_frame(&self.frame)?;
        Ok(())
    }

    pub fn device_size(&self) -> [u32; 2] {
        self.device
    }

    // --- input (CSS pixels relative to the canvas) ---

    fn device_xy(&self, x: f64, y: f64) -> (f32, f32) {
        ((x * self.dpr) as f32, (y * self.dpr) as f32)
    }

    pub fn pointer_move(&mut self, x: f64, y: f64, buttons: u32) {
        let (x, y) = self.device_xy(x, y);
        self.sim.handle_input(InputEvent::PointerMove {
            x,
            y,
            buttons: map_buttons(buttons),
        });
    }

    pub fn pointer_down(&mut self, x: f64, y: f64, button: i32) {
        let Some(button) = map_button(button) else {
            return;
        };
        let (x, y) = self.device_xy(x, y);
        self.sim
            .handle_input(InputEvent::PointerDown { x, y, button });
    }

    pub fn pointer_up(&mut self, x: f64, y: f64, button: i32) {
        let Some(button) = map_button(button) else {
            return;
        };
        let (x, y) = self.device_xy(x, y);
        self.sim
            .handle_input(InputEvent::PointerUp { x, y, button });
    }

    pub fn pointer_leave(&mut self) {
        self.wheel.reset();
        self.sim.handle_input(InputEvent::PointerLeave);
    }

    /// DOM `WheelEvent.deltaY` in pixels.
    pub fn wheel(&mut self, delta_y: f64) {
        let (notches, delta) = self.wheel.push(delta_y);
        for _ in 0..notches {
            self.sim.handle_input(InputEvent::Wheel { delta });
        }
    }

    /// DOM `KeyboardEvent.key`. Returns true when Gource uses the key (the
    /// page should then prevent the browser's default action). Character
    /// keys with Ctrl held are browser shortcuts and are not forwarded.
    pub fn key(&mut self, key: &str, pressed: bool, shift: bool, ctrl: bool) -> bool {
        let Some(k) = map_key(key) else {
            return false;
        };
        if ctrl && matches!(k, gource_core::input::Key::Char(_)) {
            return false;
        }
        self.sim.handle_input(InputEvent::Key {
            key: k,
            pressed,
            shift,
            ctrl,
        });
        gource_reacts_to(k)
    }

    // --- settings and playback ---

    pub fn set_option(&mut self, name: &str, value: &str) -> Result<OptionEffect, OptionError> {
        self.sim.set_option(name, value)
    }

    /// Viewer state, not an option: see `Simulation::set_camera_focus`.
    pub fn set_camera_focus(&mut self, dir: &str) {
        self.sim.set_camera_focus(dir);
    }

    pub fn camera_focus(&self) -> String {
        self.sim.camera_focus().to_string()
    }

    pub fn get_option(&self, name: &str) -> Option<String> {
        self.sim.option_value(name)
    }

    pub fn restart(&mut self) {
        self.sim.restart(&self.log);
    }

    pub fn seek(&mut self, fraction: f64) {
        if fraction.is_finite() {
            self.sim.seek(&self.log, fraction.clamp(0.0, 1.0) as f32);
        }
    }

    pub fn set_paused(&mut self, paused: bool) {
        self.sim.set_paused(paused);
    }

    pub fn date_at(&self, fraction: f64) -> String {
        if !fraction.is_finite() {
            return String::new();
        }
        self.sim
            .date_at(&self.log, fraction as f32)
            .unwrap_or_default()
    }

    /// `ai_stats()` JSON (see [`crate::ai::ai_stats_json`]).
    pub fn ai_stats(&self) -> String {
        crate::ai::ai_stats_json(&self.sim.attr_stats(&self.log))
    }

    /// Full frozen PR labels for the DOM fallback. Queried by the host at its
    /// status interval, never during frame construction or provider acquisition.
    pub fn pr_hover(&self) -> String {
        use crate::record::json::{self, Value};
        use gource_core::pull_request::PrPhase;
        let Some(node) = self.sim.hovered_pr() else {
            return "null".into();
        };
        let action = match node.phase {
            PrPhase::Merged => "Merged proposal",
            PrPhase::Closed => "Closed proposal",
            PrPhase::Draft => node
                .review
                .and_then(|e| e.label())
                .unwrap_or("Draft proposal"),
            PrPhase::Open => node
                .review
                .and_then(|e| e.label())
                .unwrap_or("Open proposal"),
        };
        let path = self.log.str(node.path);
        json::object([
            (
                "title",
                json::string(path.rsplit('/').next().unwrap_or(path)),
            ),
            ("owner", json::string(self.log.str(node.owner))),
            ("actor", json::string(self.log.str(node.actor))),
            ("action", json::string(action)),
            (
                "target",
                node.target
                    .map_or(Value::Null, |id| json::string(self.log.str(id))),
            ),
            // Decimal text avoids rounding extreme replay ages in JavaScript.
            (
                "age_days",
                json::string(
                    &node
                        .age_days(self.sim.status(&self.log).current_time)
                        .to_string(),
                ),
            ),
        ])
        .to_json()
    }

    pub fn status(&self) -> EngineStatus {
        let s = self.sim.status(&self.log);
        let t = &self.timings;
        EngineStatus {
            date: s.display_date,
            time: s.current_time,
            fraction: s.fraction,
            seekable: s.seekable,
            paused: s.paused,
            finished: s.finished,
            commits_loaded: self.log.commit_count(),
            commits_played: s.commits_processed,
            files: s.files,
            users: s.users,
            dirs: s.dirs,
            fps: t.fps,
            frame_ms: t.frame_ms,
            sim_ms: t.sim_ms,
            build_ms: t.build_ms,
            gpu_ms: t.gpu_ms,
            last_sim_ms: t.last[0],
            last_build_ms: t.last[1],
            last_gpu_ms: t.last[2],
            draw_calls: self.stats.draw_calls,
            vertices: self.stats.vertices,
            uploaded_bytes: self.stats.uploaded_bytes,
            fixed_dt: f64::from(self.fixed_dt),
            ticks: self.ticks,
            width: self.device[0],
            height: self.device[1],
            memory_bytes: memory_bytes(),
        }
    }
}

// --- recording (F3, `docs/features/recording.md` section 5.4) ---

impl<B: Backend, C: Clock> Engine<B, C> {
    /// An engine for a recording: `log` (a copy of the live log) and a
    /// simulation built by the caller, with no time-zone probe (the
    /// simulation's settings carry the live offset table) and no input.
    pub fn for_recording(backend: B, clock: C, log: EventLog, sim: Simulation) -> Self {
        let mut engine = Engine::new(backend, clock, Settings::default());
        engine.log = log;
        engine.replace_simulation(sim);
        engine
    }

    /// Start over with `sim` (a fresh simulation of the same run): the tick
    /// counter and frame timings are zeroed; the output size is kept.
    pub fn replace_simulation(&mut self, sim: Simulation) {
        self.sim = sim;
        self.ticks = 0;
        self.timings = Timings::default();
        self.last_frame_ms = None;
    }

    /// Output size of a recording: the drawable is exactly `width x height`
    /// (no CSS rounding as in [`Engine::resize`]) with `ui_scale` for every
    /// screen-space size. Draws the current state again.
    pub fn set_output_size(
        &mut self,
        width: u32,
        height: u32,
        ui_scale: f32,
    ) -> Result<(), B::Error> {
        self.dpr = f64::from(ui_scale);
        self.device = [width, height];
        self.backend.resize(width, height);
        self.sim.set_viewport(width, height, ui_scale);
        self.redraw()
    }

    /// One simulation tick of `dt` with the scene read back
    /// (`update` + `extract_view`) but nothing built or drawn: the state
    /// advances exactly as in [`Engine::step`].
    pub fn tick_without_drawing(&mut self, dt: f32) {
        self.sim.update(&self.log, dt);
        self.ticks += 1;
        self.sim.extract_view(&self.log, &mut self.view);
    }

    /// Draw the current state (no tick, no animation).
    pub fn render(&mut self) -> Result<(), B::Error> {
        self.redraw()
    }

    /// The scene of the last extracted frame.
    pub fn scene_view(&self) -> &SceneView {
        &self.view
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// Draw-call and upload counters of the last drawn frame.
    pub fn backend_stats(&self) -> BackendStats {
        self.stats
    }

    /// Device pixels per CSS pixel of the drawable.
    pub fn dpr(&self) -> f64 {
        self.dpr
    }

    pub fn clock(&self) -> &C {
        &self.clock
    }
}

/// Size of the WebAssembly linear memory (it only grows), for leak checks.
pub(crate) fn memory_bytes() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        core::arch::wasm32::memory_size(0) as u64 * 65_536
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gource_render::{GlyphSource, MonospaceGlyphs};
    use gource_sources::{BatchEncoder, CustomLogParser};

    use super::*;

    #[derive(Default)]
    struct FakeBackend {
        glyphs: MonospaceGlyphs,
        size: [u32; 2],
        frames: u32,
        last_clear: [f32; 4],
    }

    impl Backend for FakeBackend {
        type Error = String;

        fn resize(&mut self, width: u32, height: u32) {
            self.size = [width, height];
        }

        fn glyphs(&mut self) -> &mut dyn GlyphSource {
            &mut self.glyphs
        }

        fn draw_frame(&mut self, frame: &Frame) -> Result<BackendStats, String> {
            self.frames += 1;
            self.last_clear = frame.clear_colour;
            Ok(BackendStats {
                draw_calls: frame.commands.len() as u32,
                vertices: 0,
                uploaded_bytes: 0,
            })
        }
    }

    /// Advances 1 ms per reading.
    #[derive(Clone, Default)]
    struct TestClock(Rc<Cell<f64>>);

    impl Clock for TestClock {
        fn now_ms(&self) -> f64 {
            let t = self.0.get() + 1.0;
            self.0.set(t);
            t
        }
    }

    fn engine() -> Engine<FakeBackend, TestClock> {
        Engine::new(
            FakeBackend::default(),
            TestClock::default(),
            Settings::default(),
        )
    }

    fn batches(log: &str, chunk: usize) -> Vec<Vec<u8>> {
        let mut parser = CustomLogParser::new();
        let mut enc = BatchEncoder::new();
        let mut out = Vec::new();
        for c in log.as_bytes().chunks(chunk) {
            parser.push(c, &mut enc);
            out.push(enc.take_batch());
        }
        parser.finish(&mut enc);
        out.push(enc.take_batch());
        out
    }

    const LOG: &str = "1230000000|alice|A|/src/a.rs\n\
                       1230000000|alice|A|/src/b.rs\n\
                       1230086400|bob|M|/src/a.rs\n\
                       1230172800|carol|D|/src/b.rs\n";

    #[test]
    fn resize_rounds_device_pixels() {
        let mut e = engine();
        e.resize(1000.5, 600.0, 1.5).unwrap();
        assert_eq!(e.device_size(), [1501, 900]);
        assert_eq!(e.backend().size, [1501, 900]);
        e.resize(0.0, f64::NAN, 0.0).unwrap();
        assert_eq!(e.device_size(), [1, 1]);
    }

    /// A resize clears the canvas, so it draws the frame again at once,
    /// without a simulation tick.
    #[test]
    fn resize_redraws_without_advancing() {
        let mut e = engine();
        e.begin_log();
        for b in batches(LOG, LOG.len()) {
            e.append_batch(&b).unwrap();
        }
        e.finish_log(LOG.len() as f64);
        e.resize(640.0, 480.0, 1.0).unwrap();
        for i in 0..30 {
            e.frame(f64::from(i) * 16.0).unwrap();
        }
        let before = (e.status().ticks, e.status().time, e.backend().frames);
        e.resize(800.0, 600.0, 2.0).unwrap();
        let s = e.status();
        assert_eq!((s.ticks, s.time), (before.0, before.1));
        assert_eq!(e.backend().frames, before.2 + 1);
        assert_eq!(e.frame_data().viewport, [1600, 1200]);
    }

    #[test]
    fn batches_load_and_finish() {
        let mut e = engine();
        e.begin_log();
        for b in batches(LOG, 7) {
            e.append_batch(&b).unwrap();
        }
        assert_eq!(e.log().commit_count(), 3);
        assert!(!e.log().is_complete());
        e.finish_log(LOG.len() as f64);
        assert!(e.log().is_complete());
        assert_eq!(e.log().total_bytes(), LOG.len() as u64);
        let status = e.status();
        assert_eq!(status.commits_loaded, 3);
        assert!(status.seekable);

        // A repeated batch is rejected and leaves the log alone.
        let again = batches(LOG, LOG.len());
        assert!(e.append_batch(&again[0]).is_err() || again[0].is_empty());
        assert_eq!(e.log().commit_count(), 3);

        // A new log starts from scratch.
        e.begin_log();
        assert_eq!(e.log().commit_count(), 0);
        for b in batches(LOG, LOG.len()) {
            e.append_batch(&b).unwrap();
        }
        assert_eq!(e.log().commit_count(), 3);
    }

    #[test]
    fn frames_time_and_draw() {
        let mut e = engine();
        e.resize(640.0, 480.0, 1.0).unwrap();
        e.frame(1000.0).unwrap();
        e.frame(1016.0).unwrap();
        e.frame(1032.0).unwrap();
        // A background-tab gap does not wreck the fps average.
        e.frame(91_032.0).unwrap();
        let s = e.status();
        assert!((s.fps - 62.5).abs() < 1e-9, "{}", s.fps);
        // the resize drew once too
        assert_eq!(e.backend().frames, 5);
        assert_eq!(s.ticks, 4);
        assert!(s.frame_ms > 0.0 && s.sim_ms > 0.0 && s.build_ms > 0.0 && s.gpu_ms > 0.0);
        // The test clock ticks 1 ms per reading.
        assert_eq!(
            (s.last_sim_ms, s.last_build_ms, s.last_gpu_ms),
            (1.0, 1.0, 1.0)
        );
        assert_eq!(e.backend().last_clear[3], 1.0);
        assert_eq!((s.width, s.height), (640, 480));
    }

    #[test]
    fn real_dt_is_clamped() {
        let mut e = engine();
        assert_eq!(e.real_dt(5.0), FIRST_FRAME_DT);
        assert_eq!(e.real_dt(21.0), 0.016);
        assert_eq!(e.real_dt(60_021.0), MAX_FRAME_GAP as f32);
        assert_eq!(e.real_dt(60_000.0), 0.0);
        assert_eq!(e.real_dt(f64::NAN), FIRST_FRAME_DT);
    }

    #[test]
    fn fixed_dt_steps_exact_ticks() {
        let mut e = engine();
        e.resize(320.0, 240.0, 2.0).unwrap();
        e.set_fixed_dt(1.0 / 60.0);
        assert_eq!(e.status().fixed_dt, f64::from(1.0f32 / 60.0));
        // Animation frames only redraw.
        e.frame(0.0).unwrap();
        e.frame(16.0).unwrap();
        assert_eq!(e.status().ticks, 0);
        e.step(1).unwrap();
        e.step(2).unwrap();
        assert_eq!(e.status().ticks, 3);
        assert_eq!(e.backend().frames, 5);
        e.step(0).unwrap();
        assert_eq!(e.status().ticks, 3);
        e.set_fixed_dt(-1.0);
        assert_eq!(e.fixed_dt(), 0.0);
        e.frame(32.0).unwrap();
        assert_eq!(e.status().ticks, 4);
    }

    #[test]
    fn input_is_mapped() {
        let mut e = engine();
        e.resize(100.0, 100.0, 2.0).unwrap();
        e.pointer_move(10.0, 20.0, 1);
        e.pointer_down(10.0, 20.0, 0);
        e.pointer_up(10.0, 20.0, 4); // ignored button
        e.pointer_leave();
        e.wheel(250.0);
        assert!(e.key(" ", true, false, false));
        assert!(e.key("ArrowLeft", false, false, false));
        assert!(e.key("S", true, true, false));
        assert!(!e.key("s", true, false, true));
        assert!(!e.key("Escape", true, false, false));
        assert!(!e.key("F5", true, false, false));
    }

    #[test]
    fn pr_hover_preserves_frozen_unicode_text_and_separates_actor_from_verdict() {
        use crate::record::json;
        use gource_core::glam::Vec2;
        let mut e = engine();
        e.resize(390.0, 384.0, 1.0).unwrap();
        e.set_option("hide", "users").unwrap();
        e.set_option("seconds-per-day", "86400").unwrap();
        e.set_option("auto-skip-seconds", "-1").unwrap();
        let log = "1|Owner|A|/src/.pull-requests/#7 提案 \"quoted\" <img src=x>||ev=pr-opened;pr=7;pr-author=Owner;pr-created=-86400;pr-state=open;pr-initial=1\n\
                   1|Reviewer|M|/src/.pull-requests/#7 提案 \"quoted\" <img src=x>||ev=review-approved;pr=7;pr-author=Owner;pr-created=-86400;pr-state=open;pr-initial=0\n\
                   1|Requester|M|/src/.pull-requests/#7 提案 \"quoted\" <img src=x>||ev=pr-review-requested;pr=7;pr-author=Owner;pr-created=-86400;pr-state=open;pr-initial=0;pr-target=Target\n";
        e.begin_log();
        for batch in batches(log, log.len()) {
            e.append_batch(&batch).unwrap();
        }
        e.finish_log(log.len() as f64);
        assert_eq!(
            e.log.pr_changes().len(),
            3,
            "all mandatory rich fields survived the real parser and v3 decoder"
        );
        e.set_fixed_dt(1.0 / 60.0);
        e.step(120).unwrap();
        assert_eq!(e.pr_hover(), "null");
        let file = e.view.files.first().expect("typed proposal is visible");
        let pointer = e
            .view
            .camera
            .project(Vec2::new(390.0, 384.0), file.pos.extend(0.0));
        e.pointer_move(pointer.x as f64, pointer.y as f64, 0);
        e.step(0).unwrap();
        let raw = e.pr_hover();
        let hover = json::parse(&raw).unwrap();
        assert_eq!(
            hover.get("title").unwrap().as_str(),
            Some("#7 提案 \"quoted\" <img src=x>")
        );
        assert_eq!(hover.get("owner").unwrap().as_str(), Some("Owner"));
        assert_eq!(hover.get("actor").unwrap().as_str(), Some("Requester"));
        assert_eq!(
            hover.get("action").unwrap().as_str(),
            Some("Review approved")
        );
        assert_eq!(hover.get("target").unwrap().as_str(), Some("Target"));
        assert_eq!(hover.get("age_days").unwrap().as_str(), Some("1"));
        assert!(
            raw.contains("\\\"quoted\\\""),
            "JSON strings remain escaped"
        );
        e.set_option("hide-pull-requests", "true").unwrap();
        e.step(0).unwrap();
        assert_eq!(e.pr_hover(), "null");
        e.begin_log();
        assert_eq!(
            e.pr_hover(),
            "null",
            "new sources clear stale hover context"
        );
    }

    /// Unix time of a UTC date (`days_from_civil`), month 1-12.
    fn utc(y: i64, m: i64, d: i64, h: i64) -> i64 {
        let y = if m <= 2 { y - 1 } else { y };
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let mp = (m + 9) % 12;
        let doy = (153 * mp + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        (era * 146_097 + doe - 719_468) * 86_400 + h * 3600
    }

    /// New York since 2007: EST (UTC-5), EDT (UTC-4) from the second
    /// Sunday of March 07:00 UTC to the first Sunday of November 06:00 UTC.
    fn new_york(ts: i64) -> i32 {
        let sunday_on_or_after = |t: i64| {
            let wd = (t.div_euclid(86_400) + 4).rem_euclid(7); // 0 = Sunday
            t + (7 - wd) % 7 * 86_400
        };
        let year = (1970..2200)
            .rev()
            .find(|&y| utc(y, 1, 1, 0) <= ts)
            .unwrap_or(1970);
        let start = sunday_on_or_after(utc(year, 3, 8, 0)) + 7 * 3600;
        let end = sunday_on_or_after(utc(year, 11, 1, 0)) + 6 * 3600;
        if ts >= start && ts < end { -240 } else { -300 }
    }

    #[test]
    fn zone_changes_are_found_to_the_second() {
        let lo = 1_199_145_600; // 2008-01-01
        let hi = 1_262_304_000; // 2010-01-01
        let (base, changes) = probe_utc_offsets(&new_york, lo, hi);
        assert_eq!(base, -300);
        assert_eq!(
            changes,
            [
                (1_205_046_000, -240), // 2008-03-09 07:00 UTC
                (1_225_605_600, -300), // 2008-11-02 06:00 UTC
                (1_236_495_600, -240),
                (1_257_055_200, -300),
            ]
        );
        // a fixed zone has no changes
        assert_eq!(probe_utc_offsets(&|_| 540, lo, hi), (540, Vec::new()));
        assert_eq!(probe_utc_offsets(&new_york, hi, hi), (-300, Vec::new()));
    }

    /// Commits in winter and summer each show their own offset, whatever
    /// the offset is today, and the table grows with the log.
    #[test]
    fn dates_follow_the_dst_of_each_commit() {
        let mut e = engine();
        e.set_time_zone(Box::new(new_york));
        e.begin_log();
        let log = "1230000720|a|A|/w.c\n1245000720|a|A|/s.c\n";
        for b in batches(log, log.len()) {
            e.append_batch(&b).unwrap();
        }
        e.finish_log(log.len() as f64);
        let s = e.simulation().settings();
        assert_eq!(s.utc_offset_at(1_230_000_720), -300);
        assert_eq!(s.utc_offset_at(1_245_000_720), -240);
        assert_eq!(e.date_at(0.0), "Monday, 22 December, 2008");
        e.frame(0.0).unwrap();
        assert!(e.status().date.contains("21:52:00"), "{}", e.status().date);

        // a commit years later extends the probed range
        let more = "1600000000|a|M|/w.c\n";
        e.begin_log();
        for b in batches(more, more.len()) {
            e.append_batch(&b).unwrap();
        }
        let s = e.simulation().settings();
        assert_eq!(s.utc_offset_at(1_600_000_000), -240);
        assert_eq!(s.utc_offset_at(1_610_000_000), -300);

        // hostile timestamps are clamped, not probed at a year scale
        let hostile = "-99999999999999|a|A|/a.c\n99999999999999|a|A|/b.c\n";
        e.begin_log();
        for b in batches(hostile, hostile.len()) {
            e.append_batch(&b).unwrap();
        }
        let s = e.simulation().settings();
        assert_eq!(s.utc_offset_at(1_245_000_720), -240);
        assert_eq!(s.utc_offset_at(utc(2099, 7, 1, 0)), -240);
        assert_eq!(s.utc_offset_at(utc(2099, 12, 1, 0)), -300);
    }

    #[test]
    fn non_finite_inputs_are_safe() {
        let mut e = engine();
        e.seek(f64::NAN);
        assert_eq!(e.date_at(f64::INFINITY), "");
        e.finish_log(f64::NAN);
        assert!(e.log().is_complete());
        let json = e.status().to_json();
        assert!(json.starts_with("{\"date\":"));
    }
}
