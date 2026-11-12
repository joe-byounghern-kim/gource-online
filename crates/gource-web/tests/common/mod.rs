// SPDX-License-Identifier: GPL-3.0-or-later

//! Test doubles and fixtures for the recorder tests: a backend that counts
//! frames and records the font sizes asked for, a clock that advances 1 ms
//! per reading, and the fixture logs.

#![allow(dead_code)]

use std::cell::Cell;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gource_core::events::EventLog;
use gource_core::settings::Settings;
use gource_render::{
    Backend, BackendStats, FontKey, FontMetrics, Frame, GlyphInfo, GlyphSource, MonospaceGlyphs,
};
use gource_sources::{CustomLogParser, EventLogSink};
use gource_web::engine::Clock;
use gource_web::record::RecordError;
use gource_web::record::driver::RecordDriver;
use gource_web::record::setup::{LiveView, RecordRequest};
use gource_web::record::stop::LAST;

/// Monospace glyphs that remember every font size requested.
#[derive(Default)]
pub struct RecordingGlyphs {
    inner: MonospaceGlyphs,
    pub sizes: BTreeSet<u16>,
}

impl GlyphSource for RecordingGlyphs {
    fn metrics(&mut self, font: FontKey) -> FontMetrics {
        self.sizes.insert(font.size_px);
        self.inner.metrics(font)
    }

    fn glyph(&mut self, font: FontKey, ch: char) -> GlyphInfo {
        self.sizes.insert(font.size_px);
        self.inner.glyph(font, ch)
    }

    fn texel_size(&self) -> [f32; 2] {
        self.inner.texel_size()
    }
}

#[derive(Default)]
pub struct FakeBackend {
    pub glyphs: RecordingGlyphs,
    pub size: [u32; 2],
    pub frames: u32,
    pub draw_calls: u32,
    /// Largest drawable the "platform" allocates.
    pub max_size: Option<[u32; 2]>,
    pub evictions: u32,
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
        self.draw_calls = frame.commands.len() as u32;
        Ok(BackendStats {
            draw_calls: self.draw_calls,
            vertices: 0,
            uploaded_bytes: 0,
        })
    }

    fn drawing_buffer_size(&self) -> Option<[u32; 2]> {
        let [w, h] = self.size;
        Some(match self.max_size {
            Some([mw, mh]) => [w.min(mw), h.min(mh)],
            None => [w, h],
        })
    }

    fn atlas_evictions(&self) -> u32 {
        self.evictions
    }
}

/// Advances 1 ms per reading.
#[derive(Clone, Default)]
pub struct TestClock(pub Rc<Cell<f64>>);

impl Clock for TestClock {
    fn now_ms(&self) -> f64 {
        let t = self.0.get() + 1.0;
        self.0.set(t);
        t
    }
}

pub type Driver = RecordDriver<FakeBackend, TestClock>;

pub const LIVE: LiveView = LiveView {
    device: [1280, 720],
    dpr: 1.0,
};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// A custom log parsed into a complete event log.
pub fn parse_log(text: &[u8]) -> EventLog {
    let mut log = EventLog::new();
    let mut parser = CustomLogParser::new();
    {
        let mut sink = EventLogSink::new(&mut log);
        parser.push(text, &mut sink);
        parser.finish(&mut sink);
    }
    log.set_total_bytes(parser.stats().bytes);
    log.set_complete(true);
    log
}

/// Every fixture log of `tests/fixtures/custom-log/` and `tests/logs/`.
pub fn fixtures() -> Vec<(String, EventLog)> {
    let mut out = Vec::new();
    for dir in ["tests/fixtures/custom-log", "tests/logs"] {
        let mut paths: Vec<PathBuf> = std::fs::read_dir(repo_root().join(dir))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "log"))
            .collect();
        paths.sort();
        for p in paths {
            let name = p.file_name().unwrap().to_string_lossy().into_owned();
            out.push((name, parse_log(&std::fs::read(&p).unwrap())));
        }
    }
    out
}

pub fn fixture(name: &str) -> EventLog {
    fixtures()
        .into_iter()
        .find(|(n, _)| n == name)
        .unwrap_or_else(|| panic!("no fixture {name}"))
        .1
}

/// `commits` commits by 7 users over 40 directories, one every 6 hours.
pub fn generated_log(commits: usize) -> EventLog {
    let mut text = String::new();
    for i in 0..commits {
        let ts = 1_500_000_000 + i as i64 * 6 * 3600;
        let user = format!("user{}", i % 7);
        for f in 0..3 {
            let action = if i < 40 { "A" } else { "M" };
            text.push_str(&format!(
                "{ts}|{user}|{action}|/dir{}/sub{}/file{}.{}\n",
                (i + f) % 40,
                f,
                i % 50,
                ["c", "rs", "md"][f]
            ));
        }
    }
    parse_log(text.as_bytes())
}

pub fn request(pairs: &[(&str, &str)]) -> RecordRequest {
    let mut r = RecordRequest::default();
    for (k, v) in pairs {
        r.set(k, v).unwrap();
    }
    r
}

pub fn driver_with(
    log: &EventLog,
    base: Settings,
    pairs: &[(&str, &str)],
    backend: FakeBackend,
) -> Result<Driver, RecordError> {
    Driver::create(
        log,
        base,
        LIVE,
        &request(pairs),
        backend,
        TestClock::default(),
    )
}

pub fn driver(log: &EventLog, pairs: &[(&str, &str)]) -> Result<Driver, RecordError> {
    driver_with(log, Settings::default(), pairs, FakeBackend::default())
}

/// A frame of a run.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameInfo {
    pub flags: u32,
    pub hash: u64,
    pub pts_us: u64,
    pub duration_us: u64,
    pub progress: f64,
}

/// Record until the last frame (at most `cap` frames).
pub fn record(d: &mut Driver, cap: usize) -> Vec<FrameInfo> {
    d.begin_recording();
    let mut out = Vec::new();
    for _ in 0..cap {
        let flags = d.step_frame().unwrap();
        out.push(FrameInfo {
            flags,
            hash: d.frame_hash(),
            pts_us: d.pts_us(),
            duration_us: d.duration_us(),
            progress: d.progress(),
        });
        if flags & LAST != 0 {
            return out;
        }
    }
    panic!("no last frame within {cap} frames");
}
