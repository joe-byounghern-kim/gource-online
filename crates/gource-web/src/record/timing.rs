// SPDX-License-Identifier: GPL-3.0-or-later

//! The frame plan (`recording.md` section 5.3), which mirrors
//! `Gource::setFrameExporter` (`src/gource.cpp:505-522`): the tick rate is
//! the smallest multiple of the video frame rate that is at least 60 Hz, and
//! one frame is dumped every `k` ticks.
//!
//! | fps | k | tick rate | `fixed_dt` |
//! |---|---|---|---|
//! | 60 | 1 | 60 Hz | 1/60 |
//! | 30 | 2 | 60 Hz | 1/60 |
//! | 25 | 3 | 75 Hz | 1/75 |
//! | 24 | 3 | 72 Hz | 1/72 |
//!
//! Phase: Gource dumps a frame when `framecount % k == 0` after the update
//! and draw of that tick, with `framecount` starting at 0, so frame `n` is
//! the state after `n * k + 1` ticks. Frame 0 therefore runs one tick and
//! every later frame `k`. Presentation times are `round(n * 1e6 / fps)`
//! microseconds, computed per frame, so no error accumulates.

/// The simulation clamps every tick to 1/60 s (`max_tick_rate`), so a tick
/// rate below this would silently slow the run.
pub const MIN_TICK_HZ: u32 = 60;

/// Timing of an output video.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct FramePlan {
    pub fps: u32,
    /// Ticks between two output frames (`frameskip + 1`).
    pub ticks_per_frame: u32,
    pub tick_hz: u32,
    /// Seconds per tick, as the engine's `f32` fixed dt.
    pub fixed_dt: f32,
    /// A keyframe every this many frames.
    pub keyframe_every: u32,
}

impl FramePlan {
    /// The plan for `fps` (> 0) with a keyframe every `keyframe_every`
    /// frames.
    pub fn new(fps: u32, keyframe_every: u32) -> Self {
        let fps = fps.max(1);
        let k = MIN_TICK_HZ.div_ceil(fps);
        let tick_hz = k * fps;
        FramePlan {
            fps,
            ticks_per_frame: k,
            tick_hz,
            fixed_dt: (1.0f64 / f64::from(tick_hz)) as f32,
            keyframe_every: keyframe_every.max(1),
        }
    }

    /// Ticks to run before drawing frame `n`: 1 for the first frame, `k`
    /// after it.
    pub fn ticks_to_step(&self, n: u64) -> u32 {
        if n == 0 { 1 } else { self.ticks_per_frame }
    }

    /// Ticks run in total once frame `n` is drawn: `n * k + 1`.
    pub fn ticks_for_frame(&self, n: u64) -> u64 {
        n * u64::from(self.ticks_per_frame) + 1
    }

    /// Presentation time of frame `n` in microseconds.
    pub fn pts_us(&self, n: u64) -> u64 {
        let fps = u64::from(self.fps);
        (n * 1_000_000 + fps / 2) / fps
    }

    /// `pts_us(n + 1) - pts_us(n)`; the last frame uses
    /// [`FramePlan::nominal_duration_us`] instead (REC-33).
    pub fn duration_us(&self, n: u64) -> u64 {
        self.pts_us(n + 1) - self.pts_us(n)
    }

    /// `round(1e6 / fps)`.
    pub fn nominal_duration_us(&self) -> u64 {
        self.pts_us(1)
    }

    pub fn is_keyframe(&self, n: u64) -> bool {
        n.is_multiple_of(u64::from(self.keyframe_every))
    }

    /// Frames in `seconds` of video, rounded up.
    pub fn frames_in(&self, seconds: f64) -> u64 {
        if seconds.is_finite() && seconds > 0.0 {
            (seconds * f64::from(self.fps)).ceil() as u64
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REC-32 vectors: the four rates, the tick rates of the C++ exporter.
    #[test]
    fn rates_follow_the_cpp_exporter() {
        let rows = [
            (60, 1, 60, (1.0f64 / 60.0) as f32),
            (30, 2, 60, (1.0f64 / 60.0) as f32),
            (25, 3, 75, (1.0f64 / 75.0) as f32),
            (24, 3, 72, (1.0f64 / 72.0) as f32),
        ];
        for (fps, k, hz, dt) in rows {
            let p = FramePlan::new(fps, 1);
            assert_eq!(
                (p.ticks_per_frame, p.tick_hz, p.fixed_dt),
                (k, hz, dt),
                "{fps}"
            );
            assert!(p.fixed_dt <= (1.0f64 / 60.0) as f32);
        }
        assert_eq!(FramePlan::new(30, 1).fixed_dt, 0.016666668);
    }

    /// REC-32: cumulative ticks `n * k + 1` over 10 minutes, monotonic pts
    /// within 1 us of the nominal period, durations summing to `pts(N)`.
    #[test]
    fn ten_minutes_of_frames_line_up() {
        for fps in [24, 25, 30, 60] {
            let p = FramePlan::new(fps, 60);
            let frames = u64::from(fps) * 600;
            let period = 1e6 / f64::from(fps);
            let (mut ticks, mut total) = (0u64, 0u64);
            for n in 0..frames {
                ticks += u64::from(p.ticks_to_step(n));
                assert_eq!(ticks, p.ticks_for_frame(n));
                let d = p.duration_us(n);
                assert!((d as f64 - period).abs() <= 1.0, "{fps} fps frame {n}: {d}");
                assert!(p.pts_us(n + 1) > p.pts_us(n));
                total += d;
            }
            assert_eq!(total, p.pts_us(frames));
            assert_eq!(p.pts_us(frames), 600_000_000);
            assert_eq!(
                ticks,
                frames * u64::from(p.ticks_per_frame) - u64::from(p.ticks_per_frame) + 1
            );
        }
        let p = FramePlan::new(60, 120);
        assert_eq!(
            (p.pts_us(1), p.pts_us(2), p.pts_us(3)),
            (16_667, 33_333, 50_000)
        );
        assert_eq!(p.nominal_duration_us(), 16_667);
        assert!(p.is_keyframe(0) && p.is_keyframe(120) && !p.is_keyframe(119));
        assert_eq!(p.frames_in(5.0), 300);
        assert_eq!(p.frames_in(0.001), 1);
    }
}
