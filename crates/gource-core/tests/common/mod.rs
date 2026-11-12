// SPDX-License-Identifier: GPL-3.0-or-later
// Derived from Gource, Copyright (C) 2009 Andrew Caudwell <acaudwell@gmail.com>.

//! Test-only log readers and simulation drivers.
//!
//! The readers follow Gource's `formats/custom.cpp` (and, for the one SVN
//! scenario, `formats/svn.cpp`) closely enough for the scenario tests; the
//! real parsers live in `gource-sources`, which this crate must not depend
//! on.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use gource_core::events::{ChangeEvent, ChangeKind, EventLog, FileChange};
use gource_core::settings::Settings;
use gource_core::sim::Simulation;
use gource_core::view::SceneView;

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every scenario log: `tests/logs/*.log` and `tests/fixtures/custom-log/*.log`.
pub fn scenario_logs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in ["tests/logs", "tests/fixtures/custom-log"] {
        let mut logs: Vec<PathBuf> = std::fs::read_dir(repo_root().join(dir))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "log"))
            .collect();
        logs.sort();
        out.extend(logs);
    }
    out
}

struct Line<'a> {
    timestamp: i64,
    user: &'a str,
    action: &'a str,
    path: &'a str,
    colour: Option<[u8; 3]>,
}

/// `custom_regex` of `custom.cpp`:
/// `^(?:\xEF\xBB\xBF)?([^|]+)\|([^|]*)\|([ADM]?)\|([^|]+)(?:\|#?([a-fA-F0-9]{6}))?`
fn parse_custom_line(line: &str) -> Option<Line<'_>> {
    let line = line.strip_prefix('\u{feff}').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    let mut parts = line.splitn(5, '|');
    let ts = parts.next()?;
    let user = parts.next()?;
    let action = parts.next()?;
    let rest = parts.next()?;
    let colour_part = parts.next();
    if ts.is_empty() || !matches!(action, "" | "A" | "D" | "M") || rest.is_empty() {
        return None;
    }
    let path = rest;
    let colour = colour_part.and_then(|c| {
        let c = c.strip_prefix('#').unwrap_or(c);
        if c.len() >= 6 && c.as_bytes()[..6].iter().all(u8::is_ascii_hexdigit) {
            let v = |i: usize| u8::from_str_radix(&c[i..i + 2], 16).unwrap();
            Some([v(0), v(2), v(4)])
        } else {
            None
        }
    });

    // Allow timestamp to be a string
    let timestamp = if ts.len() > 1 && ts[1..].contains('-') {
        parse_iso(ts)?
    } else {
        let v = atoll(ts);
        if v == 0 && ts != "0" {
            return None;
        }
        v
    };

    Some(Line {
        timestamp,
        user: if user.is_empty() { "Unknown" } else { user },
        action: if action.is_empty() { "A" } else { action },
        path,
        colour,
    })
}

fn atoll(s: &str) -> i64 {
    let s = s.trim_start();
    let (neg, digits) = match s.strip_prefix('-') {
        Some(d) => (true, d),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let mut v: i64 = 0;
    for b in digits.bytes() {
        if !b.is_ascii_digit() {
            break;
        }
        v = v * 10 + (b - b'0') as i64;
    }
    if neg { -v } else { v }
}

/// `YYYY-MM-DD[ hh:mm[:ss]]` in UTC (enough for the fixtures).
fn parse_iso(s: &str) -> Option<i64> {
    let s = s.trim_end_matches('Z');
    let (date, time) = s.split_once([' ', 'T']).unwrap_or((s, "00:00:00"));
    let mut d = date.split('-').map(|x| x.parse::<i64>());
    let (y, m, day) = (d.next()?.ok()?, d.next()?.ok()?, d.next()?.ok()?);
    let mut t = time
        .split(':')
        .map(|x| x.split('.').next().unwrap().parse::<i64>());
    let h = t.next().and_then(Result::ok).unwrap_or(0);
    let mi = t.next().and_then(Result::ok).unwrap_or(0);
    let sec = t.next().and_then(Result::ok).unwrap_or(0);
    let yy = if m <= 2 { y - 1 } else { y };
    let era = yy.div_euclid(400);
    let yoe = yy - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    Some(days * 86400 + h * 3600 + mi * 60 + sec)
}

fn change(log: &mut EventLog, path: &str, action: &str, colour: Option<[u8; 3]>) -> FileChange {
    // RCommitFile prepends a root slash
    let rooted;
    let path = if path.starts_with('/') {
        path
    } else {
        rooted = format!("/{path}");
        &rooted
    };
    FileChange {
        path: log.intern(path),
        kind: ChangeKind::from_action(action),
        colour,
        event: ChangeEvent::Commit,
    }
}

/// Append custom-log `bytes` to `log`, grouping consecutive lines with the
/// same timestamp and user into one commit. A malformed line ends the
/// current commit (`parseCommit` stops at the first failed entry).
pub fn append_custom_log(log: &mut EventLog, bytes: &str, base_offset: u64) {
    let mut pending: Option<(i64, String, Vec<FileChange>, u64)> = None;
    let mut offset = base_offset;

    let flush = |log: &mut EventLog, pending: &mut Option<(i64, String, Vec<FileChange>, u64)>| {
        if let Some((ts, user, changes, off)) = pending.take() {
            let u = log.intern(&user);
            log.push_commit(ts, u, changes, off);
        }
    };

    for raw in bytes.split_inclusive('\n') {
        let line_offset = offset;
        offset += raw.len() as u64;
        let line = raw.trim_end_matches('\n');
        match parse_custom_line(line) {
            None => flush(log, &mut pending),
            Some(l) => {
                let same = pending
                    .as_ref()
                    .is_some_and(|(ts, user, _, _)| *ts == l.timestamp && user == l.user);
                if !same {
                    flush(log, &mut pending);
                    pending = Some((l.timestamp, l.user.to_string(), Vec::new(), line_offset));
                }
                let c = change(log, l.path, l.action, l.colour);
                pending.as_mut().unwrap().2.push(c);
            }
        }
    }
    flush(log, &mut pending);
}

/// The SVN XML subset of `formats/svn.cpp` used by `svn-dir-delete.log`.
pub fn append_svn_log(log: &mut EventLog, text: &str) {
    let mut offset = 0u64;
    for entry in text.split("<logentry").skip(1) {
        offset += 1;
        let tag = |name: &str| -> Option<String> {
            let open = format!("<{name}>");
            let close = format!("</{name}>");
            let start = entry.find(&open)? + open.len();
            let end = entry[start..].find(&close)? + start;
            Some(entry[start..end].to_string())
        };
        let user = tag("author").unwrap_or_else(|| "Unknown".into());
        let ts = parse_iso(&tag("date").unwrap()).unwrap();
        let mut changes = Vec::new();
        for p in entry.split("<path").skip(1) {
            if p.starts_with('s') {
                continue; // <paths>
            }
            let attr = |name: &str| -> Option<String> {
                let key = format!("{name}=\"");
                let start = p.find(&key)? + key.len();
                let end = p[start..].find('"')? + start;
                Some(p[start..end].to_string())
            };
            let Some(action) = attr("action") else {
                continue;
            };
            let start = p.find('>').unwrap() + 1;
            let end = p.find("</path>").unwrap();
            let mut file = p[start..end].to_string();
            if attr("kind").as_deref() == Some("dir") {
                if action != "D" {
                    continue;
                }
                if !file.ends_with('/') {
                    file.push('/');
                }
            }
            changes.push(change(log, &file, &action, None));
        }
        let u = log.intern(&user);
        log.push_commit(ts, u, changes, offset);
    }
}

/// A complete log from a file in either format.
pub fn load_log(path: &Path) -> EventLog {
    let text = String::from_utf8_lossy(&std::fs::read(path).unwrap()).into_owned();
    let mut log = EventLog::new();
    if text.trim_start().starts_with("<?xml") {
        append_svn_log(&mut log, &text);
    } else {
        append_custom_log(&mut log, &text, 0);
    }
    log.set_total_bytes(text.len() as u64);
    log.set_complete(true);
    log
}

/// A simulation on a 1280 x 720 viewport.
pub fn new_sim(settings: Settings) -> Simulation {
    let mut sim = Simulation::new(settings);
    sim.set_viewport(1280, 720, 1.0);
    sim
}

/// `frames` frames of update + extract_view at `dt`.
pub fn run(sim: &mut Simulation, log: &EventLog, view: &mut SceneView, frames: usize, dt: f32) {
    for _ in 0..frames {
        sim.update(log, dt);
        sim.extract_view(log, view);
    }
}

/// FNV-1a over the bit patterns of every position, size and colour in the
/// view, plus the status counts.
pub fn view_hash(view: &SceneView) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut put = |x: u32| {
        for b in x.to_le_bytes() {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for d in &view.dirs {
        for v in [d.pos.x, d.pos.y, d.spos.x, d.spos.y, d.radius] {
            put(v.to_bits());
        }
        for v in d.colour.to_array() {
            put(v.to_bits());
        }
        put(d.parent.unwrap_or(u32::MAX));
        put(d.in_frustum as u32 | (d.visible as u32) << 1 | (d.draw_edge as u32) << 2);
    }
    for f in &view.files {
        for v in [
            f.pos.x, f.pos.y, f.size.x, f.colour.x, f.colour.y, f.colour.z, f.alpha,
        ] {
            put(v.to_bits());
        }
    }
    for u in &view.users {
        for v in [
            u.pos.x, u.pos.y, u.size.x, u.colour.x, u.colour.y, u.colour.z, u.alpha,
        ] {
            put(v.to_bits());
        }
    }
    for a in &view.actions {
        for v in [a.src.x, a.src.y, a.dest.x, a.dest.y, a.progress] {
            put(v.to_bits());
        }
    }
    for l in &view.labels {
        for v in l.colour.to_array() {
            put(v.to_bits());
        }
        put(l.text.end - l.text.start);
    }
    for v in [view.camera.pos.x, view.camera.pos.y, view.camera.pos.z] {
        put(v.to_bits());
    }
    for b in view.text.bytes() {
        put(b as u32);
    }
    h
}
