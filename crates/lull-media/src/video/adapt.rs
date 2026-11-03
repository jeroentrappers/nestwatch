// SPDX-License-Identifier: AGPL-3.0-or-later
//! Video adaptation from "frame replaced" signals (media.md §5,
//! protocol.md §8).
//!
//! Levels, from best to worst: the frame-rate ladder 15, 10, 7, 5, 3, 2, 1
//! fps at quality 70, where each step that lands at 3 fps or below also
//! lowers quality by 10, followed by extra 1 fps levels down to quality 30.
//! Each replacement moves one level down; 5 s without replacement moves one
//! level up. The top level is the highest ladder rate within `max_fps`.

/// Frame-rate ladder (§5).
pub const FPS_LADDER: [u32; 7] = [15, 10, 7, 5, 3, 2, 1];
/// Starting JPEG quality (§5).
pub const START_QUALITY: u8 = 70;
/// Lowest JPEG quality (§5).
pub const MIN_QUALITY: u8 = 30;
/// Quality step (§5).
pub const QUALITY_STEP: u8 = 10;
/// Quality is lowered only at or below this rate (§5).
pub const QUALITY_FPS_LIMIT: u32 = 3;
/// Time without replacement before stepping back up (§5).
pub const RECOVERY_MS: u64 = 5000;

/// One adaptation level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoLevel {
    /// Frames per second.
    pub fps: u32,
    /// JPEG quality 1..100.
    pub quality: u8,
}

/// Per-listener video adaptation controller and frame pacer.
#[derive(Debug, Clone)]
pub struct VideoAdaptation {
    levels: Vec<VideoLevel>,
    level: usize,
    last_change_ms: u64,
    next_due_ms: Option<u64>,
}

/// Levels for a listener's `max_fps`.
pub fn levels_for(max_fps: u32) -> Vec<VideoLevel> {
    let max_fps = max_fps.max(1);
    let top = FPS_LADDER.iter().position(|&f| f <= max_fps).unwrap_or(0);
    let mut quality = START_QUALITY;
    let mut levels = Vec::new();
    for (i, &fps) in FPS_LADDER[top..].iter().enumerate() {
        if i > 0 && fps <= QUALITY_FPS_LIMIT {
            quality = quality.saturating_sub(QUALITY_STEP).max(MIN_QUALITY);
        }
        levels.push(VideoLevel { fps, quality });
    }
    while quality > MIN_QUALITY {
        quality = quality.saturating_sub(QUALITY_STEP).max(MIN_QUALITY);
        levels.push(VideoLevel { fps: 1, quality });
    }
    levels
}

impl VideoAdaptation {
    /// Starts at the best level allowed by `max_fps`, quality 70.
    pub fn new(max_fps: u32, now_ms: u64) -> Self {
        Self {
            levels: levels_for(max_fps),
            level: 0,
            last_change_ms: now_ms,
            next_due_ms: None,
        }
    }

    /// Current level.
    pub fn current(&self) -> VideoLevel {
        self.levels[self.level]
    }

    /// Current frame rate.
    pub fn fps(&self) -> u32 {
        self.current().fps
    }

    /// Current JPEG quality.
    pub fn quality(&self) -> u8 {
        self.current().quality
    }

    /// A queued frame was replaced: step one level down.
    pub fn on_frame_replaced(&mut self, now_ms: u64) {
        self.update(now_ms);
        if self.level + 1 < self.levels.len() {
            self.level += 1;
        }
        self.last_change_ms = now_ms;
    }

    /// Steps up one level per 5 s without replacement.
    pub fn update(&mut self, now_ms: u64) {
        while self.level > 0 && now_ms.saturating_sub(self.last_change_ms) >= RECOVERY_MS {
            self.level -= 1;
            self.last_change_ms += RECOVERY_MS;
        }
        if self.level == 0 {
            self.last_change_ms = self.last_change_ms.max(now_ms.saturating_sub(RECOVERY_MS));
        }
    }

    /// Whether a camera frame offered at `now_ms` should be sent; if so it is
    /// counted. Frames are paced on a fixed schedule of `1000 / fps` ms.
    pub fn take_frame(&mut self, now_ms: u64) -> bool {
        self.update(now_ms);
        let interval = 1000 / u64::from(self.fps());
        match self.next_due_ms {
            Some(due) if now_ms < due => false,
            Some(due) => {
                self.next_due_ms = Some(if now_ms - due >= interval {
                    now_ms + interval
                } else {
                    due + interval
                });
                true
            }
            None => {
                self.next_due_ms = Some(now_ms + interval);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(a: &VideoAdaptation) -> (u32, u8) {
        (a.fps(), a.quality())
    }

    #[test]
    fn media_5_ladder_down_on_replacement() {
        let mut a = VideoAdaptation::new(15, 0);
        let mut seen = vec![pairs(&a)];
        for t in 0..10 {
            a.on_frame_replaced(t * 10);
            seen.push(pairs(&a));
        }
        assert_eq!(
            seen,
            vec![
                (15, 70),
                (10, 70),
                (7, 70),
                (5, 70),
                (3, 60),
                (2, 50),
                (1, 40),
                (1, 30),
                (1, 30),
                (1, 30),
                (1, 30)
            ]
        );
    }

    #[test]
    fn media_5_default_request_starts_at_10_fps() {
        let a = VideoAdaptation::new(10, 0);
        assert_eq!(pairs(&a), (10, 70));
        assert_eq!(pairs(&VideoAdaptation::new(12, 0)), (10, 70));
        assert_eq!(pairs(&VideoAdaptation::new(60, 0)), (15, 70));
        assert_eq!(pairs(&VideoAdaptation::new(0, 0)), (1, 70));
        assert_eq!(levels_for(3).last().unwrap().quality, MIN_QUALITY);
    }

    #[test]
    fn media_5_steps_up_after_5_s_without_replacement() {
        let mut a = VideoAdaptation::new(10, 0);
        for _ in 0..4 {
            a.on_frame_replaced(1000);
        }
        assert_eq!(pairs(&a), (2, 50));
        a.update(5999);
        assert_eq!(pairs(&a), (2, 50));
        a.update(6000);
        assert_eq!(pairs(&a), (3, 60));
        a.update(11_000);
        assert_eq!(pairs(&a), (5, 70));
        // A replacement restarts the timer.
        a.on_frame_replaced(12_000);
        assert_eq!(pairs(&a), (3, 60));
        a.update(16_999);
        assert_eq!(pairs(&a), (3, 60));
        a.update(40_000);
        assert_eq!(pairs(&a), (10, 70));
        a.update(100_000);
        assert_eq!(pairs(&a), (10, 70));
    }

    #[test]
    fn media_5_pacing_follows_fps() {
        let mut a = VideoAdaptation::new(10, 0);
        // Camera at 30 fps for 3 s.
        let sent = (0..90).filter(|i| a.take_frame(i * 1000 / 30)).count();
        assert!((29..=31).contains(&sent), "{sent}");
    }
}
