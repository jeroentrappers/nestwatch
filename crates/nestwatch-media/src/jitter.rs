// SPDX-License-Identifier: AGPL-3.0-or-later
//! Per-station jitter buffer (media.md §4.1).
//!
//! Playout is scheduled in media time: the frame with capture time `m` (ms)
//! is due at `m + offset` on the listener clock. The desired offset is the
//! smoothed transit time plus the target delay (`3 × jitter + 20 ms`,
//! clamped to 60..300 ms). Each [`JitterBuffer::pull`] call produces one
//! 20 ms frame; the caller pulls once per 20 ms of device playback.

use std::collections::BTreeMap;

use crate::codec::{AudioDecoder, new_decoder};
use crate::level::level_dbfs;
use crate::{AudioFrame, CodecId, FRAME_MS, MediaError, PcmFrame, TIMESTAMP_RATE};

/// Smallest target delay (§4.1).
pub const MIN_TARGET_MS: f64 = 60.0;
/// Largest target delay (§4.1).
pub const MAX_TARGET_MS: f64 = 300.0;
/// Fastest allowed shrink of the target: 20 ms per 10 s (§4.1).
pub const SHRINK_MS_PER_MS: f64 = 20.0 / 10_000.0;

const FRAME: f64 = FRAME_MS as f64;
const TICKS_PER_MS: f64 = TIMESTAMP_RATE as f64 / 1000.0;
/// Weight of each new sample in the jitter and transit averages (1/16).
const EW_GAIN: f64 = 1.0 / 16.0;
/// A frame whose spacing is this much shorter than real time is still part
/// of a burst catching up.
const BURST_TOLERANCE_MS: f64 = 5.0;
/// Frames are played within ±half a frame of their due time.
const SCHEDULE_TOLERANCE_MS: f64 = FRAME / 2.0;
/// Minimum lead given to the first frame of a talkspurt.
const MIN_LEAD_MS: f64 = FRAME;
/// Consecutive missing frames with nothing buffered before going idle.
const IDLE_AFTER_MISSING: u32 = 3;
/// Frames at or below this level count as skimmable silence.
const SKIM_QUIET_DBFS: f32 = -50.0;
/// Frames this far below the recent average level count as pauses.
const SKIM_RELATIVE_DB: f32 = 12.0;
/// After this long with excess delay and no silence, frames are dropped anyway.
const FORCED_SKIM_AFTER_MS: i64 = 1000;
/// Spacing of forced drops.
const FORCED_SKIM_INTERVAL_MS: i64 = 60;
/// Buffered frames beyond this are discarded oldest first (5 s).
const MAX_BUFFERED: usize = 250;

/// One pulled 20 ms frame.
#[derive(Debug, Clone, PartialEq)]
pub enum Pulled {
    /// A received frame, decoded.
    Audio {
        /// Its `seq`.
        seq: u32,
        /// Decoded PCM.
        pcm: PcmFrame,
    },
    /// Codec concealment for a missing frame (§2.2).
    Concealed(PcmFrame),
    /// Nothing to play.
    Silence,
}

impl Pulled {
    /// PCM of this frame (zeros for silence).
    pub fn pcm(&self) -> PcmFrame {
        match self {
            Self::Audio { pcm, .. } | Self::Concealed(pcm) => *pcm,
            Self::Silence => crate::SILENT_FRAME,
        }
    }
}

/// Counters and current estimates.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct JitterStats {
    /// Current target delay in ms.
    pub target_ms: f64,
    /// Inter-arrival jitter estimate in ms.
    pub jitter_ms: f64,
    /// Current playout delay beyond the mean transit time, in ms.
    pub delay_ms: f64,
    /// Frames waiting.
    pub buffered: usize,
    /// Frames dropped because their slot had passed.
    pub late: u64,
    /// Duplicates dropped.
    pub duplicates: u64,
    /// Concealed frames.
    pub concealed: u64,
    /// Frames dropped to reduce excess delay.
    pub skimmed: u64,
    /// Fill frames inserted to increase delay.
    pub stretched: u64,
}

#[derive(Debug, Clone)]
struct Entry {
    frame: AudioFrame,
    arrival: f64,
    media: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Idle,
    Waiting,
    Playing,
}

/// Unwraps a wrapping u32 counter into a monotonic i64.
#[derive(Debug, Clone, Copy, Default)]
struct Unwrap(Option<(u32, i64)>);

impl Unwrap {
    fn ext(&mut self, v: u32) -> i64 {
        match self.0 {
            None => {
                self.0 = Some((v, i64::from(v)));
                i64::from(v)
            }
            Some((last, last_ext)) => {
                let e = last_ext + i64::from(v.wrapping_sub(last) as i32);
                if e > last_ext {
                    self.0 = Some((v, e));
                }
                e
            }
        }
    }
}

/// Jitter buffer and decoder for one station's stream (§4.1).
#[derive(Debug)]
pub struct JitterBuffer {
    decoder: Box<dyn AudioDecoder>,
    frames: BTreeMap<i64, Entry>,
    seq: Unwrap,
    ts: Unwrap,
    // Jitter estimation.
    prev_arrival: Option<(f64, f64)>,
    prev_counted: Option<(f64, f64)>,
    in_burst: bool,
    jitter: f64,
    transit_mean: Option<f64>,
    transit_min: Option<f64>,
    target: f64,
    target_updated: Option<f64>,
    // Playout.
    state: State,
    next_seq: i64,
    floor: i64,
    next_media: f64,
    offset: f64,
    missing_run: u32,
    excess_since: Option<i64>,
    last_skim: i64,
    level_avg: f32,
    stats: JitterStats,
}

impl JitterBuffer {
    /// Creates a buffer decoding `codec`.
    pub fn new(codec: CodecId) -> Result<Self, MediaError> {
        Ok(Self::with_decoder(new_decoder(codec)?))
    }

    /// Creates a buffer using `decoder`.
    pub fn with_decoder(decoder: Box<dyn AudioDecoder>) -> Self {
        Self {
            decoder,
            frames: BTreeMap::new(),
            seq: Unwrap::default(),
            ts: Unwrap::default(),
            prev_arrival: None,
            prev_counted: None,
            in_burst: false,
            jitter: 0.0,
            transit_mean: None,
            transit_min: None,
            target: MIN_TARGET_MS,
            target_updated: None,
            state: State::Idle,
            next_seq: 0,
            floor: i64::MIN,
            next_media: 0.0,
            offset: 0.0,
            missing_run: 0,
            excess_since: None,
            last_skim: i64::MIN / 2,
            level_avg: -90.0,
            stats: JitterStats::default(),
        }
    }

    /// Current counters and estimates.
    pub fn stats(&self) -> JitterStats {
        JitterStats {
            target_ms: self.target,
            jitter_ms: self.jitter,
            delay_ms: if self.state == State::Idle {
                0.0
            } else {
                self.offset - self.transit_ref()
            },
            buffered: self.frames.len(),
            ..self.stats
        }
    }

    /// Current target delay in ms.
    pub fn target_ms(&self) -> f64 {
        self.target
    }

    fn transit_ref(&self) -> f64 {
        self.transit_mean.or(self.transit_min).unwrap_or(0.0)
    }

    fn desired_offset(&self) -> f64 {
        self.transit_ref() + self.target
    }

    fn update_target(&mut self, now: f64) {
        let computed = (3.0 * self.jitter + 20.0).clamp(MIN_TARGET_MS, MAX_TARGET_MS);
        if computed >= self.target {
            self.target = computed;
        } else if let Some(last) = self.target_updated {
            let dt = (now - last).max(0.0);
            self.target = (self.target - dt * SHRINK_MS_PER_MS).max(computed);
        }
        self.target_updated = Some(self.target_updated.map_or(now, |l| l.max(now)));
    }

    fn observe(&mut self, flags: crate::FrameFlags, arrival: f64, media: f64) {
        let transit = arrival - media;
        self.transit_min = Some(self.transit_min.map_or(transit, |m| m.min(transit)));
        let prev_any = self.prev_arrival.replace((arrival, media));
        if flags.is_pre_roll() || flags.is_burst_start() {
            self.in_burst = true;
            return;
        }
        if self.in_burst {
            let catching_up = prev_any
                .is_some_and(|(pa, pm)| (arrival - pa) - (media - pm) < -BURST_TOLERANCE_MS);
            if !catching_up {
                self.in_burst = false;
                self.prev_counted = Some((arrival, media));
            }
            return;
        }
        if let Some((pa, pm)) = self.prev_counted {
            let d = (arrival - pa) - (media - pm);
            self.jitter += (d.abs() - self.jitter) * EW_GAIN;
        }
        self.prev_counted = Some((arrival, media));
        self.transit_mean = Some(match self.transit_mean {
            None => transit,
            Some(m) => m + (transit - m) * EW_GAIN,
        });
    }

    fn start_talkspurt(&mut self, seq: i64) {
        let Some(e) = self.frames.get(&seq) else {
            return;
        };
        let transit = e.arrival - e.media;
        self.next_seq = seq;
        self.next_media = e.media;
        self.offset = self.desired_offset().max(transit + MIN_LEAD_MS);
        self.state = State::Waiting;
        self.missing_run = 0;
        self.excess_since = None;
    }

    /// Accepts a received frame at listener time `arrival_ms`.
    pub fn push(&mut self, frame: AudioFrame, arrival_ms: u64) {
        let arrival = arrival_ms as f64;
        let seq = self.seq.ext(frame.seq);
        let media = self.ts.ext(frame.timestamp) as f64 / TICKS_PER_MS;
        if self.frames.contains_key(&seq) {
            self.stats.duplicates += 1;
            return;
        }
        self.observe(frame.flags, arrival, media);
        self.update_target(arrival);
        if seq <= self.floor || (self.state == State::Playing && seq < self.next_seq) {
            self.stats.late += 1;
            return;
        }
        self.frames.insert(
            seq,
            Entry {
                frame,
                arrival,
                media,
            },
        );
        while self.frames.len() > MAX_BUFFERED {
            if let Some((k, _)) = self.frames.pop_first() {
                self.floor = self.floor.max(k);
                if self.state != State::Idle && k >= self.next_seq {
                    self.next_seq = k + 1;
                }
            }
        }
        match self.state {
            State::Idle => {
                let first = *self.frames.keys().next().expect("just inserted");
                self.start_talkspurt(first);
            }
            State::Waiting if seq < self.next_seq => self.start_talkspurt(seq),
            _ => {}
        }
    }

    fn advance(&mut self, media: f64) {
        self.floor = self.next_seq;
        self.next_seq += 1;
        self.next_media = media + FRAME;
    }

    fn conceal(&mut self) -> Pulled {
        let next = self
            .frames
            .get(&(self.next_seq + 1))
            .map(|e| e.frame.payload.clone());
        let media = self.next_media;
        self.advance(media);
        match self.decoder.conceal(next.as_deref()) {
            Some(pcm) => {
                self.stats.concealed += 1;
                Pulled::Concealed(pcm)
            }
            None => Pulled::Silence,
        }
    }

    fn decode(&mut self, e: &Entry) -> Option<PcmFrame> {
        self.decoder.decode(&e.frame.payload).ok()
    }

    fn skimmable(&self, e: &Entry, pcm: &PcmFrame, now: i64) -> bool {
        if e.frame.flags.is_pre_roll() || !self.frames.contains_key(&self.next_seq) {
            return false;
        }
        let level = level_dbfs(pcm);
        if level <= SKIM_QUIET_DBFS || level <= self.level_avg - SKIM_RELATIVE_DB {
            return true;
        }
        self.excess_since
            .is_some_and(|s| now - s >= FORCED_SKIM_AFTER_MS)
            && now - self.last_skim >= FORCED_SKIM_INTERVAL_MS
    }

    /// Produces the next 20 ms frame at listener time `now_ms`.
    pub fn pull(&mut self, now_ms: u64) -> Pulled {
        let now = now_ms as f64;
        self.update_target(now);
        loop {
            match self.state {
                State::Idle => return Pulled::Silence,
                State::Waiting => {
                    let due = self.next_media + self.offset;
                    if now < due - SCHEDULE_TOLERANCE_MS {
                        return Pulled::Silence;
                    }
                    self.state = State::Playing;
                }
                State::Playing => return self.play(now),
            }
        }
    }

    fn play(&mut self, now: f64) -> Pulled {
        // The delay grows immediately with the target (§4.1).
        self.offset = self.offset.max(self.desired_offset());
        let slot_media = self
            .frames
            .get(&self.next_seq)
            .map_or(self.next_media, |e| e.media);
        let due = slot_media + self.offset;
        if now < due - SCHEDULE_TOLERANCE_MS {
            // Target grew: insert fill to increase the delay.
            self.stats.stretched += 1;
            return match self.decoder.conceal(None) {
                Some(pcm) => Pulled::Concealed(pcm),
                None => Pulled::Silence,
            };
        }
        if now > due + SCHEDULE_TOLERANCE_MS {
            self.offset = now - slot_media;
        }
        let Some(entry) = self.frames.remove(&self.next_seq) else {
            return self.play_missing(now);
        };
        self.missing_run = 0;
        self.next_media = entry.media;
        let mut entry = entry;
        let mut pcm = self.decode(&entry);
        self.advance(entry.media);

        let now_i = now as i64;
        let excess = self.offset - self.desired_offset();
        if excess >= FRAME && !entry.frame.flags.is_pre_roll() {
            self.excess_since.get_or_insert(now_i);
        } else if excess < FRAME {
            self.excess_since = None;
        }
        if excess >= FRAME
            && let Some(p) = pcm
            && self.skimmable(&entry, &p, now_i)
        {
            let next = self.frames.remove(&self.next_seq).expect("checked");
            self.offset -= FRAME;
            self.last_skim = now_i;
            self.stats.skimmed += 1;
            self.advance(next.media);
            pcm = self.decode(&next);
            entry = next;
        }
        match pcm {
            Some(pcm) => {
                self.level_avg += (level_dbfs(&pcm) - self.level_avg) * 0.05;
                Pulled::Audio {
                    seq: entry.frame.seq,
                    pcm,
                }
            }
            None => match self.decoder.conceal(None) {
                Some(pcm) => {
                    self.stats.concealed += 1;
                    Pulled::Concealed(pcm)
                }
                None => Pulled::Silence,
            },
        }
    }

    fn play_missing(&mut self, now: f64) -> Pulled {
        let first = self
            .frames
            .first_key_value()
            .map(|(&k, e)| (k, e.frame.flags));
        match first {
            Some((k, flags)) if flags.is_pre_roll() || flags.is_burst_start() => {
                // A gap before a burst is normal: no concealment (§4.1).
                self.floor = k - 1;
                self.start_talkspurt(k);
                self.pull(now as u64)
            }
            Some(_) => self.conceal(),
            None => {
                self.missing_run += 1;
                if self.missing_run > IDLE_AFTER_MISSING {
                    self.floor = self.next_seq - 1;
                    self.state = State::Idle;
                    return Pulled::Silence;
                }
                self.conceal()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::adpcm::encode_frame;
    use crate::{FRAME_SAMPLES, FRAME_TICKS, FrameFlags};

    fn tone(seq: u32, amp: f64) -> PcmFrame {
        let mut f = [0i16; FRAME_SAMPLES];
        for (i, s) in f.iter_mut().enumerate() {
            let t = (seq as usize * FRAME_SAMPLES + i) as f64 / 16_000.0;
            *s = (amp * (2.0 * std::f64::consts::PI * 300.0 * t).sin()) as i16;
        }
        f
    }

    fn frame(seq: u32, flags: FrameFlags, amp: f64) -> AudioFrame {
        AudioFrame {
            codec: CodecId::ADPCM,
            flags,
            seq,
            timestamp: seq.wrapping_mul(FRAME_TICKS),
            payload: encode_frame(&tone(seq, amp)).to_vec(),
        }
    }

    /// Deterministic pseudo-random numbers in [0, 1).
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// Runs a simulation: `arrivals` are (arrival_ms, frame), pulls every 20 ms.
    /// Returns (pull time, result) pairs.
    fn run(
        jb: &mut JitterBuffer,
        mut arrivals: Vec<(u64, AudioFrame)>,
        until: u64,
    ) -> Vec<(u64, Pulled)> {
        arrivals.sort_by_key(|a| a.0);
        let mut out = Vec::new();
        let mut it = arrivals.into_iter().peekable();
        let mut t = 7; // pull phase unrelated to the capture clock
        while t < until {
            while let Some((a, _)) = it.peek() {
                if *a > t {
                    break;
                }
                let (a, f) = it.next().unwrap();
                jb.push(f, a);
            }
            out.push((t, jb.pull(t)));
            t += 20;
        }
        out
    }

    fn latencies(out: &[(u64, Pulled)], arrival_of: impl Fn(u32) -> u64) -> Vec<f64> {
        out.iter()
            .filter_map(|(t, p)| match p {
                Pulled::Audio { seq, .. } => Some(*t as f64 - arrival_of(*seq) as f64),
                _ => None,
            })
            .collect()
    }

    fn played_seqs(out: &[(u64, Pulled)]) -> Vec<u32> {
        out.iter()
            .filter_map(|(_, p)| match p {
                Pulled::Audio { seq, .. } => Some(*seq),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn media_4_1_steady_stream_plays_at_min_target() {
        let mut jb = JitterBuffer::new(CodecId::ADPCM).unwrap();
        let arrivals: Vec<_> = (0..250u32)
            .map(|s| (u64::from(s) * 20 + 40, frame(s, FrameFlags::NONE, 8000.0)))
            .collect();
        let out = run(&mut jb, arrivals, 5100);
        let seqs = played_seqs(&out);
        assert_eq!(seqs, (0..250).collect::<Vec<_>>());
        let lat = latencies(&out, |s| u64::from(s) * 20 + 40);
        for l in &lat {
            assert!((l - MIN_TARGET_MS).abs() <= 10.0 + 1e-9, "{l}");
        }
        assert_eq!(jb.stats().concealed, 0);
    }

    #[test]
    fn media_4_1_adapts_to_jitter_and_latency_stays_near_target() {
        let mut jb = JitterBuffer::new(CodecId::ADPCM).unwrap();
        let mut rng = Lcg(42);
        let delays: Vec<u64> = (0..500).map(|_| 30 + (rng.next() * 80.0) as u64).collect();
        let arrivals: Vec<_> = (0..500u32)
            .map(|s| {
                (
                    u64::from(s) * 20 + delays[s as usize],
                    frame(s, FrameFlags::NONE, 8000.0),
                )
            })
            .collect();
        let out = run(&mut jb, arrivals, 10_500);
        let st = jb.stats();
        // Uniform 0..80 ms delay: mean |D| = 80/3, target about 100 ms.
        assert!(st.jitter_ms > 18.0 && st.jitter_ms < 35.0, "{st:?}");
        // The target shrinks slowly after early peaks, so allow some headroom.
        assert!(st.target_ms > 74.0 && st.target_ms < 150.0, "{st:?}");
        // Over the last 5 s: few late frames and mean latency near target.
        let lat: Vec<f64> = latencies(&out[250..], |s| u64::from(s) * 20 + delays[s as usize]);
        let mean = lat.iter().sum::<f64>() / lat.len() as f64;
        assert!(
            (mean - st.target_ms).abs() < 25.0,
            "mean {mean} target {}",
            st.target_ms
        );
        assert!(st.late < 10, "{st:?}");
        assert!(st.concealed <= 3 + st.late, "{st:?}");
    }

    #[test]
    fn media_4_1_target_grows_fast_shrinks_slowly() {
        let mut jb = JitterBuffer::new(CodecId::ADPCM).unwrap();
        let mut arrivals = Vec::new();
        // 2 s of heavy jitter, then 20 s clean.
        let mut rng = Lcg(7);
        for s in 0..1100u32 {
            let d = if s < 100 {
                20 + (rng.next() * 200.0) as u64
            } else {
                20
            };
            arrivals.push((u64::from(s) * 20 + d, frame(s, FrameFlags::NONE, 8000.0)));
        }
        let mut peak = 0.0f64;
        let mut targets = Vec::new();
        let mut it = arrivals.into_iter();
        let mut pending = it.next();
        let mut t = 0;
        while t < 22_000 {
            while let Some((a, f)) = pending.take() {
                if a > t {
                    pending = Some((a, f));
                    break;
                }
                jb.push(f, a);
                pending = it.next();
            }
            jb.pull(t);
            targets.push((t, jb.target_ms()));
            peak = peak.max(jb.target_ms());
            t += 20;
        }
        assert!(peak > 150.0, "grew to {peak}");
        for w in targets.windows(2) {
            let drop = w[0].1 - w[1].1;
            assert!(
                drop <= 20.0 * 20.0 / 10_000.0 + 1e-9,
                "shrank {drop} in one pull"
            );
        }
        let end = targets.last().unwrap().1;
        assert!(end < peak && end >= MIN_TARGET_MS);
    }

    #[test]
    fn media_4_1_drops_duplicates_and_late_frames_reorders() {
        let mut jb = JitterBuffer::new(CodecId::ADPCM).unwrap();
        let mut arrivals = Vec::new();
        for s in 0..100u32 {
            // Swap pairs to reorder, and duplicate every 10th frame.
            let pos = if s % 2 == 0 { s + 1 } else { s - 1 };
            arrivals.push((u64::from(pos) * 20 + 40, frame(s, FrameFlags::NONE, 8000.0)));
            if s % 10 == 0 {
                arrivals.push((u64::from(pos) * 20 + 45, frame(s, FrameFlags::NONE, 8000.0)));
            }
        }
        // Frame 50 again, far too late.
        arrivals.push((2050, frame(50, FrameFlags::NONE, 8000.0)));
        let out = run(&mut jb, arrivals, 2400);
        assert_eq!(played_seqs(&out), (0..100).collect::<Vec<_>>());
        let st = jb.stats();
        assert!(st.duplicates >= 9, "{st:?}");
        assert_eq!(st.late, 1);
        // Every seq was played, so no slot was concealed; only the tail after
        // the stream ends is (at most 3 frames). Reordering raises the target,
        // realised by stretching.
        assert!(st.concealed <= 3, "{st:?}");
        assert!(st.stretched > 0, "{st:?}");
    }

    #[test]
    fn media_4_1_loss_while_open_is_concealed() {
        let mut jb = JitterBuffer::new(CodecId::ADPCM).unwrap();
        let arrivals: Vec<_> = (0..100u32)
            .filter(|s| ![30, 31, 60].contains(s))
            .map(|s| (u64::from(s) * 20 + 40, frame(s, FrameFlags::NONE, 8000.0)))
            .collect();
        // Stop before the end of the stream so its tail is not concealed.
        let out = run(&mut jb, arrivals, 2100);
        let concealed = out
            .iter()
            .filter(|(_, p)| matches!(p, Pulled::Concealed(_)))
            .count();
        assert_eq!(concealed, 3);
        assert_eq!(jb.stats().concealed, 3);
        assert_eq!(played_seqs(&out).len(), 97);
    }

    /// Station-side simulation of an activated gate: quiet, then a cry with
    /// 500 ms pre-roll delivered at once, then live frames.
    fn cry_arrivals(quiet_gap: bool) -> Vec<(u64, AudioFrame)> {
        let transit = 30u64;
        let mut a = Vec::new();
        if quiet_gap {
            // An earlier short burst (seq 0..49) so the buffer has history.
            for s in 0..50u32 {
                let flags = if s == 0 {
                    FrameFlags::BURST_START
                } else {
                    FrameFlags::NONE
                };
                a.push((u64::from(s) * 20 + transit, frame(s, flags, 8000.0)));
            }
        }
        let trigger = 300u32;
        let t_trigger = u64::from(trigger) * 20 + transit;
        for (k, s) in (trigger - 25..trigger).enumerate() {
            a.push((
                t_trigger + k as u64 / 5,
                frame(s, FrameFlags::PRE_ROLL, 300.0),
            ));
        }
        a.push((
            t_trigger + 5,
            frame(trigger, FrameFlags::BURST_START, 12_000.0),
        ));
        for s in trigger + 1..trigger + 400 {
            // Cry with breathing pauses: 400 ms loud, 200 ms quiet.
            let amp = if (s - trigger) % 30 < 20 {
                12_000.0
            } else {
                100.0
            };
            a.push((u64::from(s) * 20 + transit, frame(s, FrameFlags::NONE, amp)));
        }
        a
    }

    #[test]
    fn media_4_1_preroll_burst_played_then_skimmed() {
        let mut jb = JitterBuffer::new(CodecId::ADPCM).unwrap();
        let arrivals = cry_arrivals(true);
        let arrival_of: std::collections::HashMap<u32, u64> =
            arrivals.iter().map(|(t, f)| (f.seq, *t)).collect();
        let out = run(&mut jb, arrivals, 14_000);
        let seqs = played_seqs(&out);
        // Burst gap is not concealed; only the codec's tail after the first
        // burst ends (at most 3 frames).
        assert!(jb.stats().concealed <= 3, "{:?}", jb.stats());
        // Every pre-roll frame and the trigger are played, in order, at
        // normal speed (one per pull).
        let start = seqs.iter().position(|&s| s == 275).unwrap();
        assert_eq!(
            &seqs[start..start + 26],
            &(275..301).collect::<Vec<_>>()[..]
        );
        let t0 = out
            .iter()
            .position(|(_, p)| matches!(p, Pulled::Audio { seq: 275, .. }))
            .unwrap();
        for k in 0..26 {
            assert!(matches!(out[t0 + k].1, Pulled::Audio { seq, .. } if seq == 275 + k as u32));
        }
        // Burst arrivals did not inflate the jitter estimate.
        assert!(jb.stats().jitter_ms < 5.0, "{:?}", jb.stats());
        // Excess delay is skimmed back to target within a few seconds.
        let lat: Vec<(u32, f64)> = out
            .iter()
            .filter_map(|(t, p)| match p {
                Pulled::Audio { seq, .. } => Some((*seq, *t as f64 - arrival_of[seq] as f64)),
                _ => None,
            })
            .collect();
        let trig_lat = lat.iter().find(|(s, _)| *s == 300).unwrap().1;
        assert!(
            trig_lat > 400.0,
            "trigger waits behind the pre-roll: {trig_lat}"
        );
        let late: Vec<f64> = lat
            .iter()
            .filter(|(s, _)| *s > 300 + 200)
            .map(|l| l.1)
            .collect();
        assert!(!late.is_empty());
        for l in late {
            assert!(
                l <= MIN_TARGET_MS + 30.0,
                "latency {l} not back near target"
            );
        }
        assert!(jb.stats().skimmed >= 20, "{:?}", jb.stats());
    }

    #[test]
    fn media_4_1_burst_start_from_idle_has_no_concealment() {
        let mut jb = JitterBuffer::new(CodecId::ADPCM).unwrap();
        let out = run(&mut jb, cry_arrivals(false), 9000);
        assert_eq!(jb.stats().concealed, 0);
        assert_eq!(played_seqs(&out)[0], 275);
        assert!(matches!(out[0].1, Pulled::Silence));
    }

    #[test]
    fn media_4_1_idle_after_stream_ends() {
        let mut jb = JitterBuffer::new(CodecId::ADPCM).unwrap();
        let arrivals: Vec<_> = (0..10u32)
            .map(|s| (u64::from(s) * 20, frame(s, FrameFlags::NONE, 8000.0)))
            .collect();
        let out = run(&mut jb, arrivals, 1000);
        let tail: Vec<_> = out.iter().rev().take(20).collect();
        assert!(tail.iter().all(|(_, p)| *p == Pulled::Silence));
        assert!(jb.stats().concealed <= 3);
    }

    #[test]
    fn media_4_1_seq_wrap() {
        let mut jb = JitterBuffer::new(CodecId::ADPCM).unwrap();
        let base = u32::MAX - 5;
        let arrivals: Vec<_> = (0..20u32)
            .map(|k| {
                let mut f = frame(base.wrapping_add(k), FrameFlags::NONE, 8000.0);
                f.timestamp = (u32::MAX - 100).wrapping_add(k * FRAME_TICKS);
                (u64::from(k) * 20 + 40, f)
            })
            .collect();
        let out = run(&mut jb, arrivals, 600);
        assert_eq!(played_seqs(&out).len(), 20);
    }
}
