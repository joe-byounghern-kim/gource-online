// SPDX-License-Identifier: GPL-3.0-or-later

//! `EngineStatus` (PLAN section 4) as hand-written JSON, so the wasm build
//! needs no serialisation crate.

use std::fmt::Write;

/// Everything `GourceApp.status()` reports.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EngineStatus {
    pub date: String,
    /// Simulation clock, unix seconds.
    pub time: i64,
    pub fraction: f32,
    pub seekable: bool,
    pub paused: bool,
    pub finished: bool,
    pub commits_loaded: usize,
    pub commits_played: usize,
    pub files: usize,
    pub users: usize,
    pub dirs: usize,
    pub fps: f64,
    pub frame_ms: f64,
    pub sim_ms: f64,
    pub build_ms: f64,
    pub gpu_ms: f64,
    pub draw_calls: u32,
    pub vertices: u32,
    // --- additions to the PLAN interface ---
    pub uploaded_bytes: u32,
    /// Fixed tick length in seconds; 0 = real time.
    pub fixed_dt: f64,
    /// Simulation ticks run since the engine was created.
    pub ticks: u64,
    pub width: u32,
    pub height: u32,
    /// Size of the WebAssembly linear memory in bytes (0 natively).
    pub memory_bytes: u64,
    /// `sim_ms`, `build_ms` and `gpu_ms` of the latest frame alone (the
    /// fields above are moving averages).
    pub last_sim_ms: f64,
    pub last_build_ms: f64,
    pub last_gpu_ms: f64,
}

impl EngineStatus {
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(512);
        out.push('{');
        let mut first = true;
        let mut key = |out: &mut String, name: &str| {
            if !first {
                out.push(',');
            }
            first = false;
            out.push('"');
            out.push_str(name);
            out.push_str("\":");
        };
        key(&mut out, "date");
        push_json_string(&mut out, &self.date);
        for (name, v) in [
            ("time", self.time as f64),
            ("fraction", f64::from(self.fraction)),
        ] {
            key(&mut out, name);
            push_number(&mut out, v);
        }
        for (name, v) in [
            ("seekable", self.seekable),
            ("paused", self.paused),
            ("finished", self.finished),
        ] {
            key(&mut out, name);
            out.push_str(if v { "true" } else { "false" });
        }
        for (name, v) in [
            ("commits_loaded", self.commits_loaded as f64),
            ("commits_played", self.commits_played as f64),
            ("files", self.files as f64),
            ("users", self.users as f64),
            ("dirs", self.dirs as f64),
            ("fps", self.fps),
            ("frame_ms", self.frame_ms),
            ("sim_ms", self.sim_ms),
            ("build_ms", self.build_ms),
            ("gpu_ms", self.gpu_ms),
            ("draw_calls", f64::from(self.draw_calls)),
            ("vertices", f64::from(self.vertices)),
            ("uploaded_bytes", f64::from(self.uploaded_bytes)),
            ("fixed_dt", self.fixed_dt),
            ("ticks", self.ticks as f64),
            ("width", f64::from(self.width)),
            ("height", f64::from(self.height)),
            ("memory_bytes", self.memory_bytes as f64),
            ("last_sim_ms", self.last_sim_ms),
            ("last_build_ms", self.last_build_ms),
            ("last_gpu_ms", self.last_gpu_ms),
        ] {
            key(&mut out, name);
            push_number(&mut out, v);
        }
        out.push('}');
        out
    }
}

/// JSON number; non-finite values (not representable in JSON) become 0.
fn push_number(out: &mut String, v: f64) {
    if !v.is_finite() {
        out.push('0');
    } else if v == v.trunc() && v.abs() < 9.0e15 {
        let _ = write!(out, "{}", v as i64);
    } else {
        let _ = write!(out, "{v}");
    }
}

pub(crate) fn push_json_string(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_is_written() {
        let s = EngineStatus {
            date: "Monday, \"1\"\n\u{1}".into(),
            time: 1_230_000_000,
            fraction: 0.5,
            fps: 59.94,
            fixed_dt: f64::NAN,
            draw_calls: 12,
            ..EngineStatus::default()
        };
        let json = s.to_json();
        assert!(json.starts_with(r#"{"date":"Monday, \"1\"\n\u0001","time":1230000000,"fraction":0.5,"seekable":false"#), "{json}");
        for field in [
            "paused",
            "finished",
            "commits_loaded",
            "commits_played",
            "files",
            "users",
            "dirs",
            "fps",
            "frame_ms",
            "sim_ms",
            "build_ms",
            "gpu_ms",
            "draw_calls",
            "vertices",
            "memory_bytes",
            "last_sim_ms",
            "last_build_ms",
            "last_gpu_ms",
        ] {
            assert!(
                json.contains(&format!("\"{field}\":")),
                "{field} missing in {json}"
            );
        }
        assert!(json.contains(r#""fps":59.94"#));
        assert!(json.contains(r#""fixed_dt":0"#));
        assert!(json.contains(r#""draw_calls":12"#));
        assert!(json.ends_with('}'));
    }
}
