// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sound activation (media.md §3): a shared ring of encoded frames and a
//! per-listener gate.
//!
//! Time is measured in captured frames (20 ms each), so the gate is fully
//! deterministic. A listener's `seq` counts captured frames since its gate
//! was created; frames withheld by the gate leave gaps.

use std::collections::VecDeque;

use crate::{AudioFrame, CodecId, FRAME_MS, FrameFlags};

/// Ring capacity: 2 s of 20 ms frames (§3.1).
pub const RING_FRAMES: usize = 100;
/// Longest accepted pre-roll in ms (protocol.md §7.4).
pub const MAX_PREROLL_MS: u32 = 2000;
/// Longest accepted hold time in ms (protocol.md §7.4).
pub const MAX_HOLD_MS: u32 = 60_000;

/// One encoded frame in the ring.
#[derive(Debug, Clone, PartialEq)]
pub struct RingEntry {
    /// Capture index, +1 per frame since the ring was created.
    pub index: u64,
    /// Codec of `payload`.
    pub codec: CodecId,
    /// Capture time in 1/48000 s units.
    pub timestamp: u32,
    /// Frame level after gain (§1).
    pub level_dbfs: f32,
    /// Encoded payload.
    pub payload: Vec<u8>,
}

/// Last 2 s of encoded frames of one stream, shared by all gates (§3.1).
#[derive(Debug, Clone, Default)]
pub struct AudioRing {
    entries: VecDeque<RingEntry>,
    next_index: u64,
}

impl AudioRing {
    /// Creates an empty ring.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends a frame, evicting the oldest beyond 2 s, and returns its index.
    pub fn push(
        &mut self,
        codec: CodecId,
        timestamp: u32,
        level_dbfs: f32,
        payload: Vec<u8>,
    ) -> u64 {
        let index = self.next_index;
        self.next_index += 1;
        if self.entries.len() == RING_FRAMES {
            self.entries.pop_front();
        }
        self.entries.push_back(RingEntry {
            index,
            codec,
            timestamp,
            level_dbfs,
            payload,
        });
        index
    }

    /// Index the next pushed frame will get.
    pub fn next_index(&self) -> u64 {
        self.next_index
    }

    /// Index of the oldest frame still held.
    pub fn oldest_index(&self) -> u64 {
        self.entries.front().map_or(self.next_index, |e| e.index)
    }

    /// Frame with capture index `index`, if still held.
    pub fn get(&self, index: u64) -> Option<&RingEntry> {
        let oldest = self.entries.front()?.index;
        let pos = usize::try_from(index.checked_sub(oldest)?).ok()?;
        self.entries.get(pos)
    }

    /// Number of frames held.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the ring is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Listen mode (protocol.md §7.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ListenMode {
    /// Every frame is sent.
    #[default]
    Continuous,
    /// Frames are sent while sound activation is open.
    Activated,
}

/// Gate configuration from a `listen` message.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GateConfig {
    /// Mode.
    pub mode: ListenMode,
    /// Opening threshold in dBFS (`-90..0`).
    pub threshold_dbfs: f32,
    /// Hold time in ms (`0..60000`).
    pub hold_ms: u32,
    /// Pre-roll in ms (`0..2000`).
    pub preroll_ms: u32,
}

impl Default for GateConfig {
    /// Continuous audio, as sent until the first `listen` (protocol.md §7.4).
    fn default() -> Self {
        Self {
            mode: ListenMode::Continuous,
            ..Self::product_default()
        }
    }
}

impl GateConfig {
    /// Product defaults (product.md FR-AU-4): activated, -50 dBFS, 4 s hold,
    /// 500 ms pre-roll.
    pub const fn product_default() -> Self {
        Self {
            mode: ListenMode::Activated,
            threshold_dbfs: -50.0,
            hold_ms: 4000,
            preroll_ms: 500,
        }
    }

    /// Activated mode with the given parameters.
    pub const fn activated(threshold_dbfs: f32, hold_ms: u32, preroll_ms: u32) -> Self {
        Self {
            mode: ListenMode::Activated,
            threshold_dbfs,
            hold_ms,
            preroll_ms,
        }
    }

    /// Clamps every field into its protocol range.
    pub fn clamped(self) -> Self {
        let threshold = if self.threshold_dbfs.is_nan() {
            -50.0
        } else {
            self.threshold_dbfs.clamp(-90.0, 0.0)
        };
        Self {
            mode: self.mode,
            threshold_dbfs: threshold,
            hold_ms: self.hold_ms.min(MAX_HOLD_MS),
            preroll_ms: self.preroll_ms.min(MAX_PREROLL_MS),
        }
    }

    /// Pre-roll in whole frames, rounded up so the frames cover `preroll_ms`.
    pub fn preroll_frames(&self) -> u64 {
        u64::from(self.preroll_ms.div_ceil(FRAME_MS))
    }
}

/// Output of a gate for one captured frame.
#[derive(Debug, Clone, PartialEq)]
pub enum GateEvent {
    /// Queue this frame for the listener.
    Frame(AudioFrame),
    /// Send `activity {open}` (protocol.md §7.5).
    Activity {
        /// Whether sound activation opened.
        open: bool,
    },
}

/// Per-listener sound activation gate (§3).
#[derive(Debug, Clone)]
pub struct ListenGate {
    config: GateConfig,
    base_index: u64,
    open: bool,
    last_loud: u64,
    last_sent: Option<u64>,
}

impl ListenGate {
    /// Creates a gate for a session starting now; frames captured before
    /// this point are never sent.
    pub fn new(config: GateConfig, ring: &AudioRing) -> Self {
        Self {
            config: config.clamped(),
            base_index: ring.next_index(),
            open: false,
            last_loud: 0,
            last_sent: None,
        }
    }

    /// Current configuration.
    pub fn config(&self) -> GateConfig {
        self.config
    }

    /// Whether audio currently flows to this listener (`level.open`).
    pub fn is_flowing(&self) -> bool {
        match self.config.mode {
            ListenMode::Continuous => true,
            ListenMode::Activated => self.open,
        }
    }

    /// Applies a new `listen` configuration. Switching from continuous to
    /// activated keeps the gate open with the hold timer armed at the last
    /// sent frame, so audio does not cut off mid-sound.
    pub fn set_config(&mut self, config: GateConfig) {
        let config = config.clamped();
        if self.config.mode == ListenMode::Continuous
            && config.mode == ListenMode::Activated
            && let Some(last) = self.last_sent
        {
            self.open = true;
            self.last_loud = last;
        }
        if config.mode == ListenMode::Continuous {
            self.open = false;
        }
        self.config = config;
    }

    fn frame(&mut self, e: &RingEntry, flags: FrameFlags) -> GateEvent {
        self.last_sent = Some(e.index);
        GateEvent::Frame(AudioFrame {
            codec: e.codec,
            flags,
            seq: (e.index - self.base_index) as u32,
            timestamp: e.timestamp,
            payload: e.payload.clone(),
        })
    }

    fn continuity(&self, index: u64) -> FrameFlags {
        if self.last_sent.is_some_and(|l| l + 1 == index) {
            FrameFlags::NONE
        } else {
            FrameFlags::BURST_START
        }
    }

    /// Processes the frame just pushed at `index` and returns what to send,
    /// in order. Frames are returned immediately: the gate adds no latency.
    pub fn process(&mut self, ring: &AudioRing, index: u64) -> Vec<GateEvent> {
        let mut out = Vec::new();
        let Some(e) = ring.get(index) else {
            return out;
        };
        if index < self.base_index || self.last_sent.is_some_and(|l| index <= l) {
            return out;
        }
        match self.config.mode {
            ListenMode::Continuous => {
                let flags = self.continuity(index);
                out.push(self.frame(e, flags));
            }
            ListenMode::Activated => {
                let loud = e.level_dbfs >= self.config.threshold_dbfs;
                if self.open {
                    let held = (index - self.last_loud) * u64::from(FRAME_MS)
                        <= u64::from(self.config.hold_ms);
                    if loud || held {
                        if loud {
                            self.last_loud = index;
                        }
                        let flags = self.continuity(index);
                        out.push(self.frame(e, flags));
                    } else {
                        self.open = false;
                        out.push(GateEvent::Activity { open: false });
                    }
                } else if loud {
                    self.open = true;
                    self.last_loud = index;
                    let start = index
                        .saturating_sub(self.config.preroll_frames())
                        .max(self.base_index)
                        .max(ring.oldest_index())
                        .max(self.last_sent.map_or(0, |l| l + 1));
                    for i in start..index {
                        if let Some(p) = ring.get(i) {
                            out.push(self.frame(p, FrameFlags::PRE_ROLL));
                        }
                    }
                    out.push(self.frame(e, FrameFlags::BURST_START));
                    out.push(GateEvent::Activity { open: true });
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Sim {
        ring: AudioRing,
        gate: ListenGate,
    }

    impl Sim {
        fn new(config: GateConfig) -> Self {
            let ring = AudioRing::new();
            let gate = ListenGate::new(config, &ring);
            Self { ring, gate }
        }

        fn feed(&mut self, level: f32) -> Vec<GateEvent> {
            let i = self.ring.next_index();
            let idx = self.ring.push(
                CodecId::ADPCM,
                (i as u32).wrapping_mul(960),
                level,
                (i as u32).to_be_bytes().to_vec(),
            );
            self.gate.process(&self.ring, idx)
        }

        fn feed_n(&mut self, level: f32, n: usize) -> Vec<GateEvent> {
            (0..n).flat_map(|_| self.feed(level)).collect()
        }
    }

    fn frames(ev: &[GateEvent]) -> Vec<(u32, FrameFlags)> {
        ev.iter()
            .filter_map(|e| match e {
                GateEvent::Frame(f) => Some((f.seq, f.flags)),
                GateEvent::Activity { .. } => None,
            })
            .collect()
    }

    #[test]
    fn media_3_ring_keeps_two_seconds() {
        let mut r = AudioRing::new();
        for i in 0..250u32 {
            r.push(CodecId::ADPCM, i, -60.0, vec![]);
        }
        assert_eq!(r.len(), RING_FRAMES);
        assert_eq!(r.oldest_index(), 150);
        assert!(r.get(149).is_none());
        assert_eq!(r.get(249).unwrap().timestamp, 249);
        assert!(r.get(250).is_none());
    }

    #[test]
    fn media_3_2_continuous_sends_everything() {
        let mut s = Sim::new(GateConfig::default());
        let ev = s.feed_n(-80.0, 5);
        let f = frames(&ev);
        assert_eq!(f.len(), 5);
        assert_eq!(f[0], (0, FrameFlags::BURST_START));
        assert!(f[1..].iter().all(|&(_, fl)| fl == FrameFlags::NONE));
        assert!(ev.iter().all(|e| matches!(e, GateEvent::Frame(_))));
    }

    #[test]
    fn media_3_3_quiet_room_sends_nothing() {
        let mut s = Sim::new(GateConfig::product_default());
        assert!(s.feed_n(-60.0, 500).is_empty());
        assert!(!s.gate.is_flowing());
    }

    #[test]
    fn media_3_3_cry_with_preroll_order_and_flags() {
        let mut s = Sim::new(GateConfig::activated(-50.0, 4000, 500));
        s.feed_n(-60.0, 100);
        let ev = s.feed(-30.0);
        // 25 pre-roll frames (seq 75..99), trigger (seq 100), then activity.
        assert_eq!(ev.len(), 27);
        let f = frames(&ev);
        for (k, &(seq, flags)) in f[..25].iter().enumerate() {
            assert_eq!(seq, 75 + k as u32);
            assert_eq!(flags, FrameFlags::PRE_ROLL);
        }
        assert_eq!(f[25], (100, FrameFlags::BURST_START));
        assert_eq!(ev[26], GateEvent::Activity { open: true });
        // Following frames flow immediately with no flags.
        let next = s.feed(-55.0);
        assert_eq!(frames(&next), vec![(101, FrameFlags::NONE)]);
    }

    #[test]
    fn media_3_3_preroll_limited_to_session() {
        let mut ring = AudioRing::new();
        for _ in 0..10 {
            ring.push(CodecId::ADPCM, 0, -60.0, vec![]);
        }
        let mut gate = ListenGate::new(GateConfig::activated(-50.0, 0, 2000), &ring);
        for _ in 0..3 {
            let i = ring.push(CodecId::ADPCM, 0, -60.0, vec![]);
            assert!(gate.process(&ring, i).is_empty());
        }
        let i = ring.push(CodecId::ADPCM, 0, -20.0, vec![]);
        let f = frames(&gate.process(&ring, i));
        assert_eq!(
            f,
            vec![
                (0, FrameFlags::PRE_ROLL),
                (1, FrameFlags::PRE_ROLL),
                (2, FrameFlags::PRE_ROLL),
                (3, FrameFlags::BURST_START)
            ]
        );
    }

    #[test]
    fn media_3_4_hold_rearmed_and_expiry() {
        // 100 ms hold = 5 frames.
        let mut s = Sim::new(GateConfig::activated(-50.0, 100, 0));
        let ev = s.feed(-40.0);
        assert_eq!(frames(&ev), vec![(0, FrameFlags::BURST_START)]);
        assert_eq!(frames(&s.feed_n(-60.0, 3)).len(), 3);
        // A loud frame re-arms the hold.
        assert_eq!(frames(&s.feed(-45.0)).len(), 1);
        assert_eq!(frames(&s.feed_n(-60.0, 5)).len(), 5);
        let ev = s.feed(-60.0);
        assert_eq!(ev, vec![GateEvent::Activity { open: false }]);
        assert!(!s.gate.is_flowing());
        assert!(s.feed_n(-60.0, 10).is_empty());
    }

    #[test]
    fn media_3_4_threshold_is_inclusive() {
        let mut s = Sim::new(GateConfig::activated(-50.0, 0, 0));
        assert_eq!(frames(&s.feed(-50.0)).len(), 1);
    }

    #[test]
    fn media_3_reopen_does_not_resend() {
        let mut s = Sim::new(GateConfig::activated(-50.0, 0, 500));
        s.feed_n(-60.0, 30);
        s.feed(-30.0); // seq 30 with pre-roll 5..29
        let close = s.feed(-60.0);
        assert_eq!(close, vec![GateEvent::Activity { open: false }]);
        s.feed_n(-60.0, 2); // seq 32, 33 withheld
        let ev = s.feed(-30.0); // seq 34
        let f = frames(&ev);
        assert_eq!(f.first().unwrap().0, 31, "pre-roll starts after last sent");
        assert_eq!(f.last().unwrap(), &(34, FrameFlags::BURST_START));
    }

    #[test]
    fn media_3_threshold_change_applies_next_frame() {
        let mut s = Sim::new(GateConfig::activated(-50.0, 0, 0));
        assert!(s.feed(-55.0).is_empty());
        s.gate.set_config(GateConfig::activated(-60.0, 0, 0));
        assert_eq!(frames(&s.feed(-55.0)).len(), 1);
        s.gate.set_config(GateConfig::activated(-40.0, 0, 0));
        assert_eq!(s.feed(-55.0), vec![GateEvent::Activity { open: false }]);
    }

    #[test]
    fn media_3_mode_switches() {
        let mut s = Sim::new(GateConfig::default());
        s.feed_n(-70.0, 3);
        // Continuous → activated: stays open for the hold, then closes.
        s.gate.set_config(GateConfig::activated(-50.0, 40, 0));
        assert_eq!(frames(&s.feed_n(-70.0, 1)).len(), 1);
        let ev = s.feed_n(-70.0, 2);
        assert_eq!(ev.last(), Some(&GateEvent::Activity { open: false }));
        // Activated → continuous: flows again, first frame flagged.
        s.gate.set_config(GateConfig::default());
        let f = frames(&s.feed(-70.0));
        assert_eq!(f[0].1, FrameFlags::BURST_START);
    }

    #[test]
    fn media_3_config_is_clamped() {
        let c = GateConfig::activated(10.0, 100_000, 5000).clamped();
        assert_eq!(
            (c.threshold_dbfs, c.hold_ms, c.preroll_ms),
            (0.0, 60_000, 2000)
        );
        assert_eq!(GateConfig::activated(-50.0, 0, 510).preroll_frames(), 26);
    }
}
