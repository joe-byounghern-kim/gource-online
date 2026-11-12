// SPDX-License-Identifier: GPL-3.0-or-later

//! Length estimate and progress table (`recording.md` section 5.9), in one
//! O(commits) pass over the log, without running the simulation.
//!
//! The simulation clock advances `86400 * days_per_second` seconds per
//! second of `dt * time_scale`, so a gap of `g` days between two commits
//! takes `g * seconds_per_day / time_scale` seconds of video. Auto-skip
//! jumps the clock to the next commit once every user has been idle for
//! `auto_skip_seconds` (of scaled time), which only ever shortens a run.
//!
//! - `upper_s` replays the clock on the tick grid, one step per commit: a
//!   commit ahead of the clock plays on the first tick the clock reaches
//!   it; a commit already due plays at most one tick later (the queue is
//!   refilled once per tick), and one older than the last rewinds the clock
//!   unless `no-time-travel`. Each played commit drops the clock's
//!   sub-second remainder, counted as a whole second. Rules that only
//!   shorten a run (auto-skip, filters, `stop-at-time`) are ignored, so it
//!   is an upper bound; the tail and one frame of margin are added.
//! - `about_s` caps every gap at the auto-skip delay plus a settle time
//!   ([`SETTLE_S`]: beams finish and users go idle before the idle timer
//!   starts), a heuristic.
//!
//! The same pass builds the expected video time at which each commit plays,
//! which drives the progress bar (REC-17).

use gource_core::events::{EventLog, PrPayload};
use gource_core::settings::Settings;

use crate::record::options::HARD_MAX_SECONDS;
use crate::record::timing::FramePlan;

/// Seconds a commit keeps users busy before the auto-skip idle timer can
/// start (beams finish, users go idle); heuristic.
pub const SETTLE_S: f64 = 2.0;

/// Seconds after which an idle user is deleted (`isInactive`).
const USER_INACTIVE_S: f64 = 10.0;
/// Time for the last actions to run once activated, plus the 1 s fade and a
/// margin.
const LAST_ACTIONS_S: f64 = 3.0;

/// The commits a run plays, by log index (filters not applied: they can
/// only remove commits).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CommitRange {
    pub start: usize,
    /// Exclusive.
    pub end: usize,
}

impl CommitRange {
    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The commits of `log` a simulation with `settings` reads: from
/// `start-position` (or the first commit at or after `start-date`) up to the
/// first commit after `stop-date` or past `stop-position`.
pub fn commit_range(log: &EventLog, settings: &Settings) -> CommitRange {
    let count = log.commit_count();
    let mut start = 0;
    if settings.start_timestamp != 0 {
        start = (0..count)
            .find(|&i| log.commit(i).timestamp >= settings.start_timestamp)
            .unwrap_or(count);
    }
    if settings.start_position > 0.0 && log.is_complete() && count > 0 {
        start = log.index_at_fraction(settings.start_position);
    }
    let mut end = count;
    for i in start..count {
        if settings.stop_timestamp != 0 && log.commit(i).timestamp > settings.stop_timestamp {
            end = i;
            break;
        }
        // Reading stops once the position passes `stop-position`; the
        // commit that crossed it may still have been queued.
        if settings.stop_position > 0.0
            && i > start
            && log.fraction_at_index(i) >= settings.stop_position
        {
            end = (i + 1).min(count);
            break;
        }
    }
    CommitRange {
        start,
        end: end.max(start),
    }
}

/// Seconds of video for `record-tail-seconds` (`-1`: until every user has
/// left, which takes the inactivity timeout plus the last actions, on
/// scaled time).
pub fn tail_seconds(tail: f32, settings: &Settings) -> f64 {
    if tail >= 0.0 {
        f64::from(tail)
    } else {
        let lag = f64::from(settings.max_file_lag.max(0.0));
        (USER_INACTIVE_S + lag + LAST_ACTIONS_S) / f64::from(settings.time_scale).clamp(1e-3, 1.0)
    }
}

/// The estimate of a run.
#[derive(Clone, Debug, PartialEq)]
pub struct Estimate {
    pub range: CommitRange,
    /// Context seeds establish initial state, not activity or export duration.
    pub has_activity: bool,
    /// Sum of the absolute gaps between consecutive commits, in days.
    pub span_days: f64,
    pub tail_s: f64,
    /// At least the exact length (before `record-max-seconds`).
    pub upper_s: f64,
    /// Heuristic length.
    pub about_s: f64,
    /// `upper_s` and `about_s` were cut to `record-max-seconds` (or the
    /// hard cap).
    pub clipped: bool,
    /// Expected video seconds at which commit `range.start + i` plays.
    pub expected: Vec<f32>,
}

impl Estimate {
    pub fn empty(&self) -> bool {
        !self.has_activity
    }

    /// Expected video seconds at which the last commit plays.
    pub fn body_s(&self) -> f64 {
        self.expected.last().copied().map_or(0.0, f64::from)
    }
}

/// Upper bound on the tick (counted from 1) at which the last commit of
/// `range` is played; see the module docs.
fn last_commit_tick(
    log: &EventLog,
    range: CommitRange,
    settings: &Settings,
    plan: &FramePlan,
) -> u64 {
    // simulated seconds per tick, a little less to absorb `f32` rounding
    let per_tick = 86_400.0
        * f64::from(settings.days_per_second)
        * f64::from(settings.time_scale)
        * f64::from(plan.fixed_dt)
        * (1.0 - 1e-6);
    if per_tick.is_nan() || per_tick <= 0.0 || range.is_empty() {
        return 1;
    }
    let first = log.commit(range.start).timestamp as f64;
    // the first commit sets the clock and plays on the first tick
    let (mut tick, mut clock, mut last) = (1u64, first - 1.0, first);
    for i in range.start + 1..range.end {
        let ts = log.commit(i).timestamp as f64;
        if ts > clock {
            let k = ((ts - clock) / per_tick).ceil().max(1.0);
            tick = tick.saturating_add(k as u64);
            clock += k * per_tick;
        } else {
            tick = tick.saturating_add(1);
            clock += per_tick;
        }
        if !settings.no_time_travel && ts < last {
            // an older commit rewinds the clock
            clock = ts;
        }
        last = if settings.no_time_travel {
            last.max(ts)
        } else {
            ts
        };
        // the sub-second remainder is dropped
        clock -= 1.0;
        if tick > u64::from(HARD_MAX_SECONDS) * u64::from(plan.tick_hz) {
            break;
        }
    }
    tick
}

/// Estimate the run of `log` with `settings` (the recording's simulation
/// settings) at `fps`, with `tail` (`record-tail-seconds`) and `max_s`
/// (`record-max-seconds`, 0 = none; required with `loop`).
pub fn estimate(log: &EventLog, settings: &Settings, fps: u32, tail: f32, max_s: f32) -> Estimate {
    let range = commit_range(log, settings);
    let has_activity = (range.start..range.end).any(|i| {
        let record = &log.records()[i];
        log.commit(i).changes.iter().enumerate().any(|(offset, _)| {
            !matches!(log.pr_change(record.first_change as usize + offset),
                Some(PrPayload::Lifecycle(pr)) if pr.initial)
        })
    });
    let time_scale = f64::from(settings.time_scale).max(1e-6);
    let seconds_per_day = if settings.days_per_second > 0.0 {
        1.0 / f64::from(settings.days_per_second)
    } else {
        0.0
    };
    // video seconds per simulated second
    let video_per_sim = seconds_per_day / 86_400.0 / time_scale;
    let skip_cap = (settings.auto_skip_seconds >= 0.0)
        .then(|| (f64::from(settings.auto_skip_seconds) + SETTLE_S) / time_scale);

    let mut expected = Vec::with_capacity(range.len());
    let (mut span_s, mut about) = (0.0f64, 0.0f64);
    let mut prev: Option<i64> = None;
    for i in range.start..range.end {
        let ts = log.commit(i).timestamp;
        if let Some(p) = prev {
            let gap = (ts as f64 - p as f64).abs();
            span_s += gap;
            let video = gap * video_per_sim;
            about += skip_cap.map_or(video, |cap| video.min(cap));
        }
        expected.push(about as f32);
        prev = Some(ts);
    }

    // Initial PR rows establish context, not a recording timeline. Retain
    // ordinary empty-log estimate behavior and the selected commit indices.
    let context_only = !has_activity && !range.is_empty() && log.has_pr();
    if context_only {
        span_s = 0.0;
        expected.fill(0.0);
    }
    let tail_s = if context_only {
        0.0
    } else {
        tail_seconds(tail, settings)
    };
    let plan = FramePlan::new(fps, 1);
    let frame = 1.0 / f64::from(plan.fps);
    let last_tick = last_commit_tick(log, range, settings, &plan);
    // the frame holding that tick (frame n ends at tick n * k + 1), the
    // frames before it, the tail and a frame of margin
    let body_frames = (last_tick.saturating_sub(1)).div_ceil(u64::from(plan.ticks_per_frame)) + 1;
    let mut upper_s = (body_frames + 1) as f64 * frame + tail_s;
    let mut about_s = if skip_cap.is_some() {
        (about + frame + tail_s).min(upper_s)
    } else {
        upper_s
    };
    if !has_activity {
        (upper_s, about_s) = (0.0, 0.0);
    }
    let cap = if max_s > 0.0 {
        f64::from(max_s)
    } else {
        f64::from(HARD_MAX_SECONDS)
    };
    let clipped = upper_s > cap || (!context_only && settings.loop_playback && max_s > 0.0);
    if has_activity && settings.loop_playback && max_s > 0.0 {
        // a loop runs until the limit
        (upper_s, about_s) = (cap, cap);
    }
    upper_s = upper_s.min(cap);
    about_s = about_s.min(cap);
    Estimate {
        range,
        has_activity,
        span_days: span_s / 86_400.0,
        tail_s,
        upper_s,
        about_s,
        clipped,
        expected,
    }
}

#[cfg(test)]
mod tests {
    use gource_core::events::{ChangeEvent, ChangeKind, FileChange};

    use super::*;

    fn log(ts: &[i64]) -> EventLog {
        let mut log = EventLog::new();
        for (i, &t) in ts.iter().enumerate() {
            let user = log.intern("u");
            let path = log.intern(&format!("/f{i}.c"));
            log.push_commit(
                t,
                user,
                [FileChange {
                    path,
                    kind: ChangeKind::Add,
                    colour: None,
                    event: ChangeEvent::Commit,
                }],
                i as u64 * 10,
            );
        }
        log.set_total_bytes(ts.len() as u64 * 10);
        log.set_complete(true);
        log
    }

    fn settings(options: &[(&str, &str)]) -> Settings {
        let mut s = Settings::default();
        for (k, v) in options {
            s.apply_option(k, v).unwrap();
        }
        s
    }

    const DAY: i64 = 86_400;

    #[test]
    fn span_uses_absolute_gaps_and_the_speed() {
        let l = log(&[0, 10 * DAY, 5 * DAY, 20 * DAY]);
        // 10 + 5 + 15 days, 1 s per day, no auto-skip
        let s = settings(&[("seconds-per-day", "1"), ("disable-auto-skip", "true")]);
        let e = estimate(&l, &s, 30, 5.0, 0.0);
        assert_eq!(e.span_days, 30.0);
        // the clock runs 10 days, rewinds to day 5 (one tick) and runs 15
        // more: 25 s, plus rounding, a frame of margin and the tail
        assert!(
            e.upper_s >= 30.0 && e.upper_s < 30.0 + 5.0 / 30.0,
            "{}",
            e.upper_s
        );
        assert_eq!(
            e.about_s, e.upper_s,
            "without auto-skip the estimate is the bound"
        );
        assert_eq!(e.expected, [0.0, 10.0, 15.0, 30.0]);
        // time-scale 2 halves the body, not the tail
        let s2 = settings(&[
            ("seconds-per-day", "1"),
            ("disable-auto-skip", "true"),
            ("time-scale", "2"),
        ]);
        let e2 = estimate(&l, &s2, 30, 5.0, 0.0);
        assert!(
            e2.upper_s >= 17.5 && e2.upper_s < 17.5 + 5.0 / 30.0,
            "{}",
            e2.upper_s
        );
    }

    #[test]
    fn auto_skip_caps_each_gap_for_the_estimate() {
        let l = log(&[0, 100 * DAY, 100 * DAY + 3600]);
        let s = settings(&[("seconds-per-day", "1")]);
        let e = estimate(&l, &s, 60, 0.0, 0.0);
        // 100 days capped at 3 + 2 s; one hour is 1/24 s
        assert!(
            (e.about_s - (5.0 + 1.0 / 24.0 + 1.0 / 60.0)).abs() < 1e-6,
            "{}",
            e.about_s
        );
        assert!(e.upper_s > 100.0);
        let table = e.expected.clone();
        assert!(table.windows(2).all(|w| w[0] <= w[1]));
        assert_eq!(e.body_s(), f64::from(table[2]));
    }

    #[test]
    fn ranges_follow_dates_and_positions() {
        let l = log(&[DAY, 2 * DAY, 3 * DAY, 4 * DAY, 5 * DAY]);
        let mut s = Settings {
            start_timestamp: 2 * DAY,
            stop_timestamp: 4 * DAY,
            ..Settings::default()
        };
        assert_eq!(commit_range(&l, &s), CommitRange { start: 1, end: 4 });
        s.start_timestamp = 6 * DAY;
        assert!(commit_range(&l, &s).is_empty());
        assert!(estimate(&l, &s, 30, 5.0, 0.0).empty());
        assert_eq!(estimate(&l, &s, 30, 5.0, 0.0).upper_s, 0.0);
        let s = settings(&[("start-position", "0.4")]);
        assert_eq!(commit_range(&l, &s).start, l.index_at_fraction(0.4));
        let s = settings(&[("stop-position", "0.5")]);
        let r = commit_range(&l, &s);
        assert_eq!(r.start, 0);
        assert!(r.end < 5 && r.end > 1, "{r:?}");
        assert!(estimate(&log(&[]), &Settings::default(), 30, 5.0, 0.0).empty());
    }

    #[test]
    fn tails_limits_and_loops() {
        let l = log(&[0, DAY]);
        let s = settings(&[("seconds-per-day", "100"), ("disable-auto-skip", "true")]);
        assert_eq!(tail_seconds(-1.0, &s), 18.0);
        let half = settings(&[("time-scale", "0.5")]);
        assert_eq!(tail_seconds(-1.0, &half), 36.0);
        assert_eq!(tail_seconds(7.0, &half), 7.0);
        let e = estimate(&l, &s, 30, 5.0, 60.0);
        assert_eq!((e.upper_s, e.about_s, e.clipped), (60.0, 60.0, true));
        let mut looped = s.clone();
        looped.loop_playback = true;
        let e = estimate(&l, &looped, 30, 5.0, 20.0);
        assert_eq!((e.upper_s, e.about_s, e.clipped), (20.0, 20.0, true));
        let slow = settings(&[
            ("seconds-per-day", "1000000"),
            ("disable-auto-skip", "true"),
        ]);
        let e = estimate(&log(&[0, 365 * DAY]), &slow, 30, 5.0, 0.0);
        assert_eq!(e.upper_s, f64::from(HARD_MAX_SECONDS));
    }
}
