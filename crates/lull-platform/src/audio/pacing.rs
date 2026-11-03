// SPDX-License-Identifier: AGPL-3.0-or-later
//! Wall-clock pacing for playback through a pipe.
//!
//! A pipe to a sound-server client buffers seconds of audio, so writing as
//! fast as the pipe accepts would drain the jitter buffer immediately and
//! add seconds of latency. The writer instead keeps at most `lead` of audio
//! ahead of the wall clock.

use std::time::{Duration, Instant};

/// What the writer should do now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pace {
    /// Frames to write immediately.
    pub frames: u32,
    /// The writer fell more than one frame behind the wall clock (its
    /// thread stalled or the pipe pushed back); the schedule was re-anchored
    /// instead of bursting to catch up.
    pub reanchored: bool,
    /// When to poll again.
    pub next: Instant,
}

/// Paces fixed-size frames against the wall clock.
#[derive(Debug, Clone)]
pub struct WallClockPacer {
    frame: Duration,
    lead: Duration,
    origin: Instant,
    sent: u64,
}

impl WallClockPacer {
    /// Creates a pacer starting at `now` for frames of `frame` duration,
    /// keeping up to `lead` written ahead of real time.
    pub fn new(now: Instant, frame: Duration, lead: Duration) -> Self {
        assert!(!frame.is_zero(), "frame duration must be positive");
        Self {
            frame,
            lead,
            origin: now,
            sent: 0,
        }
    }

    /// Frames written so far.
    pub fn sent(&self) -> u64 {
        self.sent
    }

    /// Audio time written so far, relative to the schedule origin.
    fn written(&self) -> Duration {
        self.frame * u32::try_from(self.sent).unwrap_or(u32::MAX)
    }

    /// Decides how many frames are due at `now`, and records them as sent.
    pub fn poll(&mut self, now: Instant) -> Pace {
        let mut elapsed = now.saturating_duration_since(self.origin);
        let mut reanchored = false;
        // Behind by more than a frame: what we write now plays late anyway.
        // Shift the origin so the next frame is due now, and keep the lead.
        if elapsed > self.written() + self.frame {
            self.origin = now.checked_sub(self.written()).unwrap_or(now);
            elapsed = self.written();
            reanchored = true;
        }
        // Write every whole frame that ends within the horizon.
        let horizon = elapsed + self.lead;
        let mut frames = 0u32;
        while self.written() + self.frame <= horizon {
            self.sent += 1;
            frames += 1;
        }
        // The next frame is due once it fits within the horizon.
        let due_at = (self.written() + self.frame).saturating_sub(self.lead);
        let next = self.origin + due_at;
        Pace {
            frames,
            reanchored,
            next: next.max(now),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const F: Duration = Duration::from_millis(20);

    #[test]
    fn initial_burst_fills_the_lead() {
        let t0 = Instant::now();
        let mut p = WallClockPacer::new(t0, F, Duration::from_millis(60));
        let pace = p.poll(t0);
        assert_eq!(pace.frames, 3);
        assert!(!pace.reanchored);
        assert_eq!(pace.next, t0 + F);
        // Nothing more is due until a frame's worth of time passed.
        assert_eq!(p.poll(t0 + Duration::from_millis(5)).frames, 0);
    }

    #[test]
    fn steady_state_writes_one_frame_per_frame_time() {
        let t0 = Instant::now();
        let mut p = WallClockPacer::new(t0, F, Duration::from_millis(40));
        p.poll(t0);
        let mut total = 0;
        for i in 1..=500u32 {
            let pace = p.poll(t0 + F * i);
            assert!(!pace.reanchored);
            total += pace.frames;
        }
        // 10 s of wall clock → 500 frames, never more than the lead ahead.
        assert_eq!(total, 500);
        assert_eq!(p.sent(), 502);
    }

    #[test]
    fn never_runs_ahead_of_wall_clock_plus_lead() {
        let t0 = Instant::now();
        let lead = Duration::from_millis(50);
        let mut p = WallClockPacer::new(t0, F, lead);
        for ms in (0..3000).step_by(7) {
            let now = t0 + Duration::from_millis(ms);
            p.poll(now);
            let written = F * p.sent() as u32;
            assert!(written <= Duration::from_millis(ms) + lead);
            assert!(written + F >= Duration::from_millis(ms));
        }
    }

    #[test]
    fn next_wakeup_is_when_the_next_frame_is_due() {
        let t0 = Instant::now();
        let mut p = WallClockPacer::new(t0, F, Duration::from_millis(40));
        let pace = p.poll(t0 + Duration::from_millis(3));
        // 2 frames written (40 ms), lead 40 → the third is due at 20 ms.
        assert_eq!(pace.frames, 2);
        assert_eq!(pace.next, t0 + Duration::from_millis(20));
    }

    #[test]
    fn stall_reanchors_instead_of_bursting() {
        let t0 = Instant::now();
        let mut p = WallClockPacer::new(t0, F, Duration::from_millis(40));
        p.poll(t0);
        let sent_before = p.sent();
        // Thread stalls for 1 s.
        let pace = p.poll(t0 + Duration::from_secs(1));
        assert!(pace.reanchored);
        // Only the lead is refilled, not 50 frames of backlog.
        assert_eq!(pace.frames, 2);
        assert_eq!(p.sent(), sent_before + 2);
        // Afterwards pacing continues normally from the new origin.
        let pace = p.poll(t0 + Duration::from_secs(1) + F);
        assert_eq!(pace.frames, 1);
        assert!(!pace.reanchored);
    }
}
