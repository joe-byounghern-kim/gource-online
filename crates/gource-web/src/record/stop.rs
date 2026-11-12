// SPDX-License-Identifier: GPL-3.0-or-later

//! When a recording ends (`recording.md` section 5.3, REC-10), shared by
//! the recorder and the dry run so the two cannot diverge (REC-67).
//!
//! Evaluated after every output frame, in priority order:
//!
//! 1. `record-max-seconds` frames reached: stop ([`LIMIT`]).
//! 2. Loop off, tail `>= 0`: the simulation's `finished` flag (the last
//!    commit was played) has been true for the tail's frames: stop. An
//!    explicit tail is honoured in full, so "after the last commit + N s"
//!    is exact.
//! 3. Loop off, tail `-1`: Gource's `appFinished` (every user left after
//!    the end): stop at once.
//! 4. The hard cap of four hours ([`HARD_CAP`]).

use crate::record::options::HARD_MAX_SECONDS;
use crate::record::timing::FramePlan;

/// The frame just rendered is the last one.
pub const LAST: u32 = 1;
/// Encode the frame as a keyframe.
pub const KEY: u32 = 2;
/// The last commit has been played; the frame belongs to the tail.
pub const IN_TAIL: u32 = 4;
/// The run stopped at `record-max-seconds`.
pub const LIMIT: u32 = 8;
/// The run stopped at the four-hour hard cap.
pub const HARD_CAP: u32 = 16;

/// The stop parameters of one recording.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct StopRules {
    /// `record-max-seconds` in frames; 0 for none.
    pub max_frames: u64,
    /// The tail in frames; `None` for `-1` (wait for `appFinished`).
    pub tail_frames: Option<u64>,
    pub looping: bool,
    pub hard_cap_frames: u64,
}

impl StopRules {
    pub fn new(plan: &FramePlan, tail_seconds: f32, max_seconds: f32, looping: bool) -> Self {
        StopRules {
            max_frames: plan.frames_in(f64::from(max_seconds)),
            tail_frames: (tail_seconds >= 0.0)
                .then(|| (f64::from(tail_seconds) * f64::from(plan.fps)).round() as u64),
            looping,
            hard_cap_frames: plan.frames_in(f64::from(HARD_MAX_SECONDS)),
        }
    }
}

/// How a run ended.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EndReason {
    /// The tail after the last commit, or every user left.
    Finished,
    /// `record-max-seconds`.
    Limit,
    /// [`HARD_MAX_SECONDS`].
    HardCap,
}

impl EndReason {
    /// The sidecar's `result.limit` value (`None` for a natural end).
    pub fn limit_name(self) -> Option<&'static str> {
        match self {
            EndReason::Finished => None,
            EndReason::Limit => Some("max_seconds"),
            EndReason::HardCap => Some("hard_cap"),
        }
    }
}

/// Frame counter and stop state of one run.
#[derive(Clone, Debug, PartialEq)]
pub struct StopState {
    rules: StopRules,
    /// Frames rendered so far.
    frames: u64,
    /// Index of the first frame at which `finished` was seen.
    finished_at: Option<u64>,
    ended: Option<EndReason>,
}

impl StopState {
    pub fn new(rules: StopRules) -> Self {
        StopState {
            rules,
            frames: 0,
            finished_at: None,
            ended: None,
        }
    }

    pub fn rules(&self) -> &StopRules {
        &self.rules
    }

    /// Frames rendered so far; the next frame's index.
    pub fn frames(&self) -> u64 {
        self.frames
    }

    pub fn ended(&self) -> Option<EndReason> {
        self.ended
    }

    /// Index of the first frame of the tail.
    pub fn finished_at(&self) -> Option<u64> {
        self.finished_at
    }

    /// Record that a frame was rendered with the simulation in this state;
    /// returns its flags. `keyframe` is the plan's keyframe decision.
    pub fn after_frame(&mut self, finished: bool, app_finished: bool, keyframe: bool) -> u32 {
        let n = self.frames;
        self.frames += 1;
        let mut flags = if keyframe { KEY } else { 0 };
        if finished && !self.rules.looping && self.finished_at.is_none() {
            self.finished_at = Some(n);
        }
        if self.finished_at.is_some() {
            flags |= IN_TAIL;
        }
        let r = &self.rules;
        let end = if r.max_frames > 0 && self.frames >= r.max_frames {
            Some(EndReason::Limit)
        } else if r.looping {
            None
        } else {
            match (r.tail_frames, self.finished_at) {
                (Some(tail), Some(at)) if n - at >= tail => Some(EndReason::Finished),
                (None, Some(_)) if app_finished => Some(EndReason::Finished),
                _ => None,
            }
        };
        let end = end.or((self.frames >= r.hard_cap_frames).then_some(EndReason::HardCap));
        if let Some(reason) = end {
            self.ended = Some(reason);
            flags |= LAST;
            match reason {
                EndReason::Limit => flags |= LIMIT,
                EndReason::HardCap => flags |= HARD_CAP,
                EndReason::Finished => {}
            }
        }
        flags
    }
}

/// Monotone progress, 0 to 1 (REC-17): the expected video time of the
/// commits played so far over the estimate, then linear over the tail;
/// frames over the exact count once a dry run has measured it.
#[derive(Clone, Debug, PartialEq)]
pub struct Progress {
    /// Expected video seconds at which each commit of the range plays.
    expected: Vec<f32>,
    range_start: usize,
    about_s: f64,
    tail_s: f64,
    fps: f64,
    /// Frames the run will have, when known (dry run or a loop's limit).
    exact_frames: Option<u64>,
    max_frames: u64,
    /// Cursor seen last, and the frame it changed at (within a gap the bar
    /// moves with video time).
    cursor: usize,
    cursor_frame: u64,
    value: f64,
}

impl Progress {
    pub fn new(
        expected: Vec<f32>,
        range_start: usize,
        about_s: f64,
        tail_s: f64,
        plan: &FramePlan,
        rules: &StopRules,
    ) -> Self {
        Progress {
            expected,
            range_start,
            about_s,
            tail_s,
            fps: f64::from(plan.fps),
            exact_frames: None,
            max_frames: rules.max_frames,
            cursor: range_start,
            cursor_frame: 0,
            value: 0.0,
        }
    }

    /// The dry run's frame count becomes the denominator.
    pub fn set_exact_frames(&mut self, frames: u64) {
        self.exact_frames = (frames > 0).then_some(frames);
    }

    pub fn exact_frames(&self) -> Option<u64> {
        self.exact_frames
    }

    /// Start again at 0 (a new recording on the same setup).
    pub fn reset(&mut self) {
        self.cursor = self.range_start;
        self.cursor_frame = 0;
        self.value = 0.0;
    }

    pub fn value(&self) -> f64 {
        self.value
    }

    /// Update after frame `n` with the simulation's commit cursor; `stop`
    /// is the stop state after that frame.
    pub fn update(&mut self, n: u64, commit_cursor: usize, stop: &StopState) -> f64 {
        if commit_cursor != self.cursor {
            self.cursor = commit_cursor;
            self.cursor_frame = n;
        }
        let frames_done = (n + 1) as f64;
        let p = if stop.ended().is_some() {
            1.0
        } else if let Some(total) = self.exact_frames {
            frames_done / total as f64
        } else if self.max_frames > 0 && (self.max_frames as f64) < self.about_s * self.fps {
            frames_done / self.max_frames as f64
        } else if self.about_s <= 0.0 {
            0.0
        } else if let Some(at) = stop.finished_at() {
            let body = (self.about_s - self.tail_s).max(0.0) / self.about_s;
            let tail_frames = (self.tail_s * self.fps).max(1.0);
            body + (1.0 - body) * ((n - at) as f64 / tail_frames)
        } else {
            let played = self.cursor.saturating_sub(self.range_start);
            let at = |i: usize| self.expected.get(i).copied().map_or(0.0, f64::from);
            let (t0, t1) = if played == 0 {
                (0.0, at(0))
            } else {
                (at(played - 1), at(played).max(at(played - 1)))
            };
            let within = (n - self.cursor_frame) as f64 / self.fps;
            (t0 + within.min(t1 - t0)) / self.about_s
        };
        // never backwards, and 1 only at the end
        let cap = if stop.ended().is_some() { 1.0 } else { 0.999 };
        self.value = self.value.max(p.clamp(0.0, cap));
        self.value
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules(tail: f32, max: f32, looping: bool) -> StopRules {
        StopRules::new(&FramePlan::new(30, 60), tail, max, looping)
    }

    /// Runs frames with `finished` from frame `f` and `app_finished` from
    /// frame `a`; returns the frame count and how it ended.
    fn run(r: StopRules, f: u64, a: u64) -> (u64, EndReason) {
        let mut s = StopState::new(r);
        loop {
            let n = s.frames();
            let flags = s.after_frame(n >= f, n >= a, n.is_multiple_of(60));
            assert_eq!(flags & KEY != 0, n.is_multiple_of(60));
            assert_eq!(flags & IN_TAIL != 0, n >= f && !r.looping);
            if flags & LAST != 0 {
                return (s.frames(), s.ended().unwrap());
            }
            assert!(n < 10_000_000);
        }
    }

    #[test]
    fn stop_rules_in_priority_order() {
        // 5 s tail at 30 fps: 150 frames after the first finished frame
        assert_eq!(
            run(rules(5.0, 0.0, false), 100, 200),
            (251, EndReason::Finished)
        );
        // the tail is honoured even when every user left earlier
        assert_eq!(
            run(rules(5.0, 0.0, false), 100, 120),
            (251, EndReason::Finished)
        );
        // tail 0 ends on the frame that played the last commit
        assert_eq!(
            run(rules(0.0, 0.0, false), 100, 120),
            (101, EndReason::Finished)
        );
        // -1 waits for appFinished, and only then
        assert_eq!(
            run(rules(-1.0, 0.0, false), 100, 900),
            (901, EndReason::Finished)
        );
        // the limit wins
        assert_eq!(
            run(rules(5.0, 2.0, false), 100, 200),
            (60, EndReason::Limit)
        );
        // a loop never finishes on its own
        assert_eq!(
            run(rules(5.0, 10.0, true), 100, 120),
            (300, EndReason::Limit)
        );
        // the hard cap
        assert_eq!(
            run(rules(-1.0, 0.0, false), 100, u64::MAX),
            (14_400 * 30, EndReason::HardCap)
        );
        let mut s = StopState::new(rules(5.0, 2.0, false));
        let flags: Vec<u32> = (0..60)
            .map(|_| s.after_frame(false, false, false))
            .collect();
        assert_eq!(flags[59], LAST | LIMIT);
        assert!(flags[..59].iter().all(|&f| f == 0));
        assert_eq!(EndReason::Limit.limit_name(), Some("max_seconds"));
        assert_eq!(EndReason::Finished.limit_name(), None);
    }

    #[test]
    fn progress_is_monotone_and_reaches_one_at_the_end() {
        let plan = FramePlan::new(30, 60);
        let r = rules(1.0, 0.0, false);
        // three commits expected at 0, 2 and 4 s; 1 s tail
        let mut p = Progress::new(vec![0.0, 2.0, 4.0], 10, 5.0, 1.0, &plan, &r);
        let mut s = StopState::new(r);
        let mut last = 0.0;
        let mut values = Vec::new();
        for n in 0..1000u64 {
            // one commit per two seconds of video
            let cursor = 10 + (n / 60).min(3) as usize;
            let flags = s.after_frame(cursor == 13, false, false);
            let v = p.update(n, cursor, &s);
            assert!(v >= last, "frame {n}: {v} < {last}");
            last = v;
            values.push(v);
            if flags & LAST != 0 {
                break;
            }
        }
        assert_eq!(*values.last().unwrap(), 1.0);
        assert!(values[..values.len() - 1].iter().all(|&v| v < 1.0));
        // the tail starts at the body's share and moves linearly
        let tail_start = s.finished_at().unwrap() as usize;
        assert!(
            (values[tail_start] - 0.8).abs() < 0.01,
            "{}",
            values[tail_start]
        );

        let mut exact = Progress::new(vec![0.0], 0, 5.0, 1.0, &plan, &r);
        exact.set_exact_frames(200);
        let s = StopState::new(r);
        assert_eq!(exact.update(99, 0, &s), 0.5);
        exact.reset();
        assert_eq!(exact.value(), 0.0);
    }
}
