// SPDX-License-Identifier: GPL-3.0-or-later

//! The `wasm-bindgen` surface (PLAN section 4): `GourceApp` for the page and
//! `LogParser` for the parsing worker.

use gource_core::settings::{OptionEffect, Settings};
use gource_render_webgl2::WebGl2Backend;
use wasm_bindgen::prelude::*;
use web_sys::{HtmlCanvasElement, Performance};

use crate::engine::{Clock, Engine};
use crate::parser::ChunkParser;

/// Runs once per realm (page and worker) when the module is initialised.
#[wasm_bindgen(start)]
fn start() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&JsValue::from_str(&format!("gource-web panicked: {info}")));
    }));
}

/// `performance.now()`, or `Date.now()` where there is no Performance API.
pub(crate) struct PerfClock(Option<Performance>);

impl PerfClock {
    pub(crate) fn new() -> Self {
        PerfClock(web_sys::window().and_then(|w| w.performance()))
    }
}

impl Clock for PerfClock {
    fn now_ms(&self) -> f64 {
        match &self.0 {
            Some(p) => p.now(),
            None => js_sys::Date::now(),
        }
    }
}

/// Local time zone of the browser, in minutes east of UTC (Gource formats
/// dates and reads zone-less ISO dates in local time).
fn local_utc_offset_minutes() -> i32 {
    minutes_east(js_sys::Date::new_0().get_timezone_offset())
}

/// The browser's offset in force at unix time `ts` (DST, historical zone
/// rules), in minutes east of UTC.
fn utc_offset_minutes_at(ts: i64) -> i32 {
    let date = js_sys::Date::new(&JsValue::from_f64(ts as f64 * 1000.0));
    minutes_east(date.get_timezone_offset())
}

/// `getTimezoneOffset` (minutes west, fractional for local mean time) to
/// whole minutes east.
fn minutes_east(offset_west: f64) -> i32 {
    if offset_west.is_finite() {
        -(offset_west.round() as i32)
    } else {
        0
    }
}

/// Simulation, frame building and WebGL2 rendering on one canvas.
#[wasm_bindgen]
pub struct GourceApp {
    engine: Engine<WebGl2Backend, PerfClock>,
}

impl GourceApp {
    /// The engine a recorder forks (`record::wasm`).
    pub(crate) fn engine(&self) -> &Engine<WebGl2Backend, PerfClock> {
        &self.engine
    }
}

#[wasm_bindgen]
impl GourceApp {
    /// The canvas must be in the DOM and the `GourceSans` font face loaded.
    /// Throws when WebGL2 is unavailable.
    #[wasm_bindgen(constructor)]
    pub fn new(canvas: HtmlCanvasElement) -> Result<GourceApp, JsValue> {
        let backend = WebGl2Backend::new(&canvas)?;
        let clock = PerfClock::new();
        let settings = Settings {
            utc_offset_minutes: local_utc_offset_minutes(),
            ..Settings::default()
        };
        let mut engine = Engine::new(backend, clock, settings);
        engine.set_time_zone(Box::new(utc_offset_minutes_at));
        Ok(GourceApp { engine })
    }

    // --- events ---

    /// New repository: drop events and entities.
    pub fn begin_log(&mut self) {
        self.engine.begin_log();
    }

    /// A batch from `LogParser.push` / `finish`, in order. Throws on a
    /// corrupt, lost or duplicated batch (the log is left unchanged).
    pub fn append_batch(&mut self, batch: &[u8]) -> Result<(), JsError> {
        self.engine
            .append_batch(batch)
            .map(|_| ())
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// The log is complete: the run becomes seekable.
    pub fn finish_log(&mut self, total_bytes: f64) {
        self.engine.finish_log(total_bytes);
    }

    // --- loop ---

    /// Advance and render; call from `requestAnimationFrame` with its
    /// timestamp.
    pub fn frame(&mut self, now_ms: f64) -> Result<(), JsValue> {
        self.engine.frame(now_ms)
    }

    /// Deterministic mode: `seconds` of simulation per tick, advanced only by
    /// `step`; animation frames just redraw. 0 returns to real time.
    pub fn set_fixed_dt(&mut self, seconds: f64) {
        self.engine.set_fixed_dt(seconds);
    }

    /// Run exactly `ticks` simulation ticks of the fixed dt (Gource's video
    /// exporter: one tick per output frame), then render one frame.
    pub fn step(&mut self, ticks: u32) -> Result<(), JsValue> {
        self.engine.step(ticks)
    }

    /// CSS size of the canvas and `devicePixelRatio`; sets the drawable size
    /// and draws the current frame again (resizing clears the canvas).
    pub fn resize(
        &mut self,
        css_width: f64,
        css_height: f64,
        device_pixel_ratio: f64,
    ) -> Result<(), JsValue> {
        self.engine
            .resize(css_width, css_height, device_pixel_ratio)
    }

    // --- input: CSS pixels relative to the canvas top-left ---

    /// `buttons` is `MouseEvent.buttons`.
    pub fn pointer_move(&mut self, x: f64, y: f64, buttons: u32) {
        self.engine.pointer_move(x, y, buttons);
    }

    /// `button` is `MouseEvent.button`.
    pub fn pointer_down(&mut self, x: f64, y: f64, button: i32) {
        self.engine.pointer_down(x, y, button);
    }

    pub fn pointer_up(&mut self, x: f64, y: f64, button: i32) {
        self.engine.pointer_up(x, y, button);
    }

    pub fn pointer_leave(&mut self) {
        self.engine.pointer_leave();
    }

    /// `WheelEvent.deltaY` in pixels.
    pub fn wheel(&mut self, delta_y: f64) {
        self.engine.wheel(delta_y);
    }

    /// `KeyboardEvent.key`; true when Gource handled it.
    pub fn key(&mut self, key: &str, pressed: bool, shift: bool, ctrl: bool) -> bool {
        self.engine.key(key, pressed, shift, ctrl)
    }

    // --- settings: Gource option names and value syntax ---

    /// "live" or "restart" (then call `restart`); throws on invalid input.
    /// Coded errors (`user-style`) throw `<code>: <text>`.
    pub fn set_option(&mut self, name: &str, value: &str) -> Result<String, JsError> {
        match self.engine.set_option(name, value) {
            Ok(OptionEffect::Live) => Ok("live".into()),
            Ok(OptionEffect::Restart) => Ok("restart".into()),
            Err(e) => Err(JsError::new(&crate::ai::option_error_message(&e))),
        }
    }

    /// Validate `user-style` rules without applying them: JSON
    /// `{"ok":true}` or `{"ok":false,"code":...,"line":N,"message":...}`.
    pub fn check_style(&self, text: &str) -> String {
        crate::ai::check_style_json(text)
    }

    /// JSON AI attribution numbers of the loaded log and of the played
    /// commits (read about twice a second, not every frame).
    pub fn ai_stats(&self) -> String {
        self.engine.ai_stats()
    }

    /// Full escaped frozen PR hover details, or JSON null.
    pub fn pr_hover(&self) -> String {
        self.engine.pr_hover()
    }

    pub fn get_option(&self, name: &str) -> Option<String> {
        self.engine.get_option(name)
    }

    /// Frame the dirs under `dir` (for example `/owner:repo/`) instead of the
    /// whole tree; empty frames the whole tree. Viewer state, not an option.
    pub fn set_camera_focus(&mut self, dir: &str) {
        self.engine.set_camera_focus(dir);
    }

    pub fn camera_focus(&self) -> String {
        self.engine.camera_focus()
    }

    /// Rebuild the simulation from the event log.
    pub fn restart(&mut self) {
        self.engine.restart();
    }

    // --- playback ---

    pub fn set_paused(&mut self, paused: bool) {
        self.engine.set_paused(paused);
    }

    pub fn seek(&mut self, fraction: f64) {
        self.engine.seek(fraction);
    }

    /// Date at `fraction` of the log, or "" when unknown.
    pub fn date_at(&self, fraction: f64) -> String {
        self.engine.date_at(fraction)
    }

    /// JSON `EngineStatus`.
    pub fn status(&self) -> String {
        self.engine.status().to_json()
    }

    /// Size of the module's linear memory in bytes (it only grows; for
    /// memory benchmarks).
    pub fn memory_bytes(&self) -> f64 {
        crate::engine::memory_bytes() as f64
    }
}

/// Incremental custom-log parser for the Web Worker.
#[wasm_bindgen]
pub struct LogParser {
    inner: ChunkParser,
}

impl Default for LogParser {
    fn default() -> Self {
        Self::new()
    }
}

#[wasm_bindgen]
impl LogParser {
    #[wasm_bindgen(constructor)]
    pub fn new() -> LogParser {
        LogParser {
            inner: ChunkParser::new(local_utc_offset_minutes() * 60),
        }
    }

    /// Raw log bytes in; an encoded batch out (may be empty).
    pub fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.inner.push(chunk)
    }

    /// Flush the last line and commit.
    pub fn finish(&mut self) -> Vec<u8> {
        self.inner.finish()
    }

    pub fn commits(&self) -> f64 {
        self.inner.stats().commits as f64
    }

    pub fn bytes(&self) -> f64 {
        self.inner.stats().bytes as f64
    }

    pub fn skipped_lines(&self) -> f64 {
        self.inner.stats().skipped_lines as f64
    }

    /// Attribution data that was ignored (malformed meta pairs, sets past
    /// the cap).
    pub fn meta_ignored(&self) -> f64 {
        self.inner.stats().bad_meta as f64
    }

    /// The input format, decided by the first line: `"custom"` (a custom
    /// log), `"git"` (the output of Gource's git log command) or
    /// `"unknown"` before a whole line has arrived.
    pub fn format(&self) -> String {
        self.inner.format().as_str().to_string()
    }
}

/// Option names whose values hold contributor or file names (redacted
/// from recipes, flagged on private share links).
#[wasm_bindgen]
pub fn name_bearing_options() -> Vec<String> {
    gource_core::settings::NAME_BEARING_OPTIONS
        .iter()
        .map(|s| s.to_string())
        .collect()
}
