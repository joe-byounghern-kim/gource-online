// SPDX-License-Identifier: GPL-3.0-or-later

//! The recording API for the page (`recording.md` section 5.15):
//! `GourceRecorder`, `RecordPlanner`, `Mp4Muxer` and a few free functions.
//! Every error is a `JsError` whose message is `"<code>: <text>"`.

use gource_render_webgl2::WebGl2Backend;
use js_sys::Uint8Array;
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::record::driver::RecordDriver;
use crate::record::json::{Value, string};
use crate::record::mux::Muxer;
use crate::record::planner::RecordPlanner;
use crate::record::recipe::{RecipeKind, recipe};
use crate::record::setup::{LiveView, RecordRequest, RecordSetup};
use crate::record::{RecordError, codes};
use crate::wasm::{GourceApp, PerfClock};

fn js(e: RecordError) -> JsError {
    JsError::new(&e.to_string())
}

fn live_view(app: &GourceApp) -> LiveView {
    let engine = app.engine();
    LiveView {
        device: engine.device_size(),
        dpr: engine.dpr(),
    }
}

/// A recording fork of a `GourceApp` on its own canvas.
#[wasm_bindgen]
pub struct GourceRecorder {
    driver: RecordDriver<WebGl2Backend, PerfClock>,
    canvas: HtmlCanvasElement,
}

#[wasm_bindgen]
impl GourceRecorder {
    /// Fork `app` (a copy of its complete event log and a snapshot of its
    /// settings) onto `canvas`, or onto a new detached canvas when it is
    /// undefined. `options_json` is an object of option names to values:
    /// `record-*` recording options and Gource option overrides.
    ///
    /// Throws `record_log_incomplete`, `record_invalid_option`,
    /// `record_unbounded`, `record_size_unsupported`, `record_empty_run` or
    /// `record_gl_failed`.
    pub fn create(
        app: &GourceApp,
        canvas: Option<HtmlCanvasElement>,
        options_json: &str,
    ) -> Result<GourceRecorder, JsError> {
        let request = RecordRequest::from_json(options_json).map_err(js)?;
        let canvas = match canvas {
            Some(c) => c,
            None => new_canvas().map_err(js)?,
        };
        let mut backend = WebGl2Backend::new(&canvas).map_err(|e| {
            js(RecordError::new(
                codes::GL_FAILED,
                format!("WebGL2 is not available for the recording ({e:?})."),
            ))
        })?;
        // every label is rasterised in the frame that needs it (REC-42)
        backend.set_glyph_budget(u32::MAX);
        let engine = app.engine();
        let driver = RecordDriver::create(
            engine,
            engine.simulation().settings_snapshot(),
            live_view(app),
            &request,
            backend,
            PerfClock::new(),
        )
        .map_err(js)?;
        Ok(GourceRecorder { driver, canvas })
    }

    /// The recorder's canvas: `new VideoFrame(recorder.canvas, ...)` right
    /// after `step_frame`, in the same task.
    #[wasm_bindgen(getter)]
    pub fn canvas(&self) -> HtmlCanvasElement {
        self.canvas.clone()
    }

    /// New options: restarts the run, resizes the output and draws it.
    /// Cancels a preview. Same errors as `create`.
    pub fn reconfigure(&mut self, options_json: &str) -> Result<(), JsError> {
        let request = RecordRequest::from_json(options_json).map_err(js)?;
        self.driver.reconfigure(&request).map_err(js)
    }

    /// `RecordPlan` JSON: frame plan, size, codec, bitrate, estimate,
    /// layout, ladder, warnings.
    pub fn plan_json(&self) -> String {
        self.driver.plan_value().to_json()
    }

    /// The dry run's exact frame count (`RecordPlanner.result_json().frames`).
    pub fn set_exact_frames(&mut self, frames: f64) {
        if frames.is_finite() && frames >= 1.0 {
            self.driver.set_exact_frames(frames as u64);
        }
    }

    /// Render the run `sim_seconds` of video after its start, at most `ms`
    /// of work per call; true once the preview is on the canvas.
    pub fn preview_for(&mut self, sim_seconds: f64, ms: f64) -> Result<bool, JsError> {
        self.driver.preview_for(sim_seconds, ms).map_err(js)
    }

    /// Draw the current state again (no tick), so the page can read the
    /// canvas in the same task (a PNG still with `canvas.toBlob`).
    pub fn redraw(&mut self) -> Result<(), JsError> {
        self.driver
            .engine_mut()
            .render()
            .map_err(|e| js(RecordError::new(codes::GL_FAILED, format!("{e:?}"))))
    }

    /// Restart the run from its beginning for the recording: frame and pts
    /// counters, progress and the glyph atlas are reset.
    pub fn begin_recording(&mut self) {
        self.driver.begin_recording();
        self.driver.engine_mut().backend_mut().reset_glyphs();
    }

    /// Run the ticks of the next frame and draw it. Returns its flags:
    /// 1 LAST, 2 KEY, 4 IN_TAIL, 8 LIMIT, 16 HARD_CAP. Throws
    /// `record_not_started` before `begin_recording` or after the last
    /// frame, `record_gl_failed` when the context was lost.
    pub fn step_frame(&mut self) -> Result<u32, JsError> {
        let flags = self.driver.step_frame().map_err(js)?;
        if self.driver.engine().backend().is_context_lost() {
            return Err(js(RecordError::new(
                codes::GL_FAILED,
                "The graphics context was lost; the recording stopped.",
            )));
        }
        Ok(flags)
    }

    /// Index of the frame just rendered; -1 before the first.
    pub fn frame_index(&self) -> f64 {
        self.driver.frame_index().map_or(-1.0, |n| n as f64)
    }

    /// Frames rendered since `begin_recording`.
    pub fn frame_count(&self) -> f64 {
        self.driver.frames() as f64
    }

    /// `VideoFrame` timestamp of the frame just rendered (microseconds).
    pub fn pts_us(&self) -> f64 {
        self.driver.pts_us() as f64
    }

    /// `VideoFrame` duration of the frame just rendered (microseconds).
    pub fn duration_us(&self) -> f64 {
        self.driver.duration_us() as f64
    }

    /// 0 to 1, monotone.
    pub fn progress(&self) -> f64 {
        self.driver.progress()
    }

    /// `hash_view` of the frame just rendered, 16 hex digits.
    pub fn frame_hash(&self) -> String {
        format!("{:016x}", self.driver.frame_hash())
    }

    /// The date shown in the frame.
    pub fn sim_date(&self) -> String {
        self.driver.engine().status().date
    }

    /// `{state, frame_index, frames, exact_frames, progress, pts_us,
    /// duration_us, flags, in_tail, ended, date, time}`.
    pub fn status_json(&self) -> String {
        self.driver.status_value().to_json()
    }

    /// `{ticks, frames, files, users, dirs, commits, commit_cursor,
    /// draw_calls, vertices, atlas_evictions, preview_empty}`.
    pub fn stats_json(&self) -> String {
        self.driver.stats_value().to_json()
    }

    /// The recorder's part of the sidecar `result`.
    pub fn result_json(&self) -> String {
        self.driver.result_value().to_json()
    }

    /// The recipe, `kind` "embedded" or "sidecar"; `context_json` is the
    /// page's `{build, source, confirmed, time_zone, created, result}`.
    pub fn recipe_json(&self, kind: &str, context_json: &str) -> Result<String, JsError> {
        let kind = RecipeKind::parse(kind).ok_or_else(|| {
            js(RecordError::new(
                codes::INVALID_OPTION,
                "recipe kind must be embedded or sidecar",
            ))
        })?;
        let setup = self.driver.setup();
        let log = self.driver.engine().log();
        let first =
            (!setup.estimate.empty()).then(|| log.commit(setup.estimate.range.start).timestamp);
        let result = (kind == RecipeKind::Sidecar).then(|| self.driver.result_value());
        recipe(setup, context_json, kind, result, first)
            .map(|v| v.to_json())
            .map_err(js)
    }
}

fn new_canvas() -> Result<HtmlCanvasElement, RecordError> {
    let fail =
        |what: &str| RecordError::new(codes::GL_FAILED, format!("cannot create a canvas: {what}"));
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| fail("no document"))?;
    document
        .create_element("canvas")
        .map_err(|_| fail("createElement failed"))?
        .dyn_into::<HtmlCanvasElement>()
        .map_err(|_| fail("not a canvas"))
}

/// The exact dry run, sliced.
#[wasm_bindgen(js_name = RecordPlanner)]
pub struct WasmRecordPlanner {
    inner: RecordPlanner<PerfClock>,
}

#[wasm_bindgen(js_class = RecordPlanner)]
impl WasmRecordPlanner {
    /// Same inputs and errors as `GourceRecorder.create` (no canvas).
    pub fn create(app: &GourceApp, options_json: &str) -> Result<WasmRecordPlanner, JsError> {
        let request = RecordRequest::from_json(options_json).map_err(js)?;
        let engine = app.engine();
        let inner = RecordPlanner::create(
            engine,
            engine.simulation().settings_snapshot(),
            live_view(app),
            &request,
            PerfClock::new(),
        )
        .map_err(js)?;
        Ok(WasmRecordPlanner { inner })
    }

    /// Run for about `ms`; the ticks run, 0 once done.
    pub fn run_for(&mut self, ms: f64) -> u32 {
        self.inner.run_for(ms)
    }

    pub fn done(&self) -> bool {
        self.inner.done()
    }

    /// 0 to 1 against the upper bound.
    pub fn progress(&self) -> f64 {
        self.inner.progress()
    }

    /// `{done, frames, duration_s, exact, size_upper_bytes, limit, commits,
    /// empty, ticks}`.
    pub fn result_json(&self) -> String {
        self.inner.result_value().to_json()
    }
}

/// The plan of a recording without forking the engine (the dialog's
/// estimate; it also reports an empty range instead of throwing).
#[wasm_bindgen]
pub fn record_plan(app: &GourceApp, options_json: &str) -> Result<String, JsError> {
    let request = RecordRequest::from_json(options_json).map_err(js)?;
    let engine = app.engine();
    let setup = RecordSetup::resolve(
        &engine.simulation().settings_snapshot(),
        &request,
        live_view(app),
        engine.log(),
    )
    .map_err(js)?;
    Ok(setup.plan_value(None).to_json())
}

/// Validate and normalise option pairs (`{name: value}`, recording and
/// Gource options): returns `{name: normalised value}`; throws
/// `record_invalid_option`.
#[wasm_bindgen]
pub fn parse_record_options(pairs_json: &str) -> Result<String, JsError> {
    let request = RecordRequest::from_json(pairs_json).map_err(js)?;
    let mut settings = gource_core::settings::Settings::default();
    let mut out = Vec::new();
    for (name, value) in &request.overrides {
        settings
            .apply_option(name, value)
            .map_err(|e| js(RecordError::new(codes::INVALID_OPTION, e.to_string())))?;
        out.push((
            name.clone(),
            string(&settings.option_value(name).unwrap_or_default()),
        ));
    }
    let parsed = crate::record::json::parse(pairs_json).unwrap_or(Value::Null);
    for (name, _) in parsed.members().unwrap_or(&[]) {
        if let Some(v) = request.options.value(name) {
            out.push((name.clone(), string(&v)));
        }
    }
    Ok(Value::Object(out).to_json())
}

/// The recording option table: `[{name, default, choices, range, special}]`
/// (the TypeScript `RECORD_OPTION_DEFS` must agree).
#[wasm_bindgen]
pub fn record_option_defs() -> String {
    crate::record::options::defs_value().to_json()
}

/// H.264 MP4 writer (`gource-mux`).
#[wasm_bindgen]
pub struct Mp4Muxer {
    inner: Muxer,
}

#[wasm_bindgen]
impl Mp4Muxer {
    /// `config_json`: `{width, height, fps, layout, max_samples,
    /// max_sync_samples, colour, created, comment, build, timescale}`;
    /// `avcc`: `decoderConfig.description`; `recipe`: the embedded recipe
    /// (UTF-8), or undefined. The header is pending at once.
    #[wasm_bindgen(constructor)]
    pub fn new(
        config_json: &str,
        avcc: &[u8],
        recipe: Option<Vec<u8>>,
    ) -> Result<Mp4Muxer, JsError> {
        Ok(Mp4Muxer {
            inner: Muxer::new(config_json, avcc, recipe).map_err(js)?,
        })
    }

    /// One encoded chunk; returns the bytes pending. Throws
    /// `record_mux_reorder` or `record_mux_failed`.
    pub fn push(
        &mut self,
        data: &[u8],
        pts_us: f64,
        duration_us: f64,
        is_key: bool,
    ) -> Result<u32, JsError> {
        self.inner
            .push(data, pts_us, duration_us, is_key)
            .map(|n| n as u32)
            .map_err(js)
    }

    /// Bytes waiting to be taken.
    pub fn pending(&self) -> u32 {
        self.inner.pending() as u32
    }

    /// Copy up to `dest.length` pending bytes into `dest`; returns how many.
    pub fn take_output(&mut self, dest: &Uint8Array) -> u32 {
        let n = self.inner.pending().min(dest.length() as usize);
        let bytes = self.inner.take(n);
        dest.subarray(0, bytes.len() as u32).copy_from(bytes);
        bytes.len() as u32
    }

    /// Ends the file (after `encoder.flush()`); returns the bytes pending.
    pub fn finish(&mut self) -> Result<u32, JsError> {
        self.inner.finish().map(|n| n as u32).map_err(js)
    }

    /// In-place edits for a seekable sink, valid after `finish`:
    /// `(offset u64 LE, length u32 LE, bytes)*`.
    pub fn take_patches(&self) -> Vec<u8> {
        self.inner.patches()
    }

    /// Where the bytes of `finish` go: `{"kind":"append"}` or
    /// `{"kind":"insert_at","offset":N,"len":N}` (faststart-deferred).
    pub fn placement_json(&self) -> String {
        self.inner.placement_value().to_json()
    }

    /// Faststart outgrew its reserve; `moov` went to the end.
    pub fn moov_at_end(&self) -> bool {
        self.inner.writer().moov_at_end()
    }

    /// The recipe exceeded 16 KiB and was not embedded.
    pub fn recipe_dropped(&self) -> bool {
        self.inner.writer().recipe_dropped()
    }

    pub fn bytes_written(&self) -> f64 {
        self.inner.writer().bytes_written() as f64
    }

    pub fn frames(&self) -> f64 {
        self.inner.writer().frames() as f64
    }

    pub fn duration_us(&self) -> f64 {
        self.inner.writer().duration_us() as f64
    }

    /// The codec string the `avcC` describes (`avc1.PPCCLL`).
    pub fn codec(&self) -> String {
        self.inner.writer().avc().codec_string()
    }
}
