// SPDX-License-Identifier: AGPL-3.0-or-later
//! Capture chain: resample to 16 kHz mono, gain / AGC, level (media.md §1).

use crate::level::{db_to_gain, level_dbfs};
use crate::resample::Resampler;
use crate::{FRAME_MS, FRAME_SAMPLES, MediaError, PcmFrame, SAMPLE_RATE};

/// AGC target level for loud events (§1.1).
pub const AGC_TARGET_DBFS: f32 = -24.0;
/// Maximum AGC gain (§1.1).
pub const AGC_MAX_GAIN_DB: f32 = 24.0;
/// Maximum AGC gain increase per second (§1.1).
pub const AGC_RAISE_DB_PER_S: f32 = 3.0;
/// Frames below this input level are never amplified (§1.1).
pub const AGC_NOISE_FLOOR_DBFS: f32 = -70.0;
/// A frame is a sound event when it is this far above the tracked room noise
/// floor (§1.1); only events move the AGC gain up.
pub const AGC_EVENT_MARGIN_DB: f32 = 12.0;
/// How fast the tracked noise floor may rise, in dB per second (§1.1). It
/// follows the room down immediately.
pub const NOISE_FLOOR_RISE_DB_PER_S: f32 = 0.5;

/// Manual microphone gain step (§1.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GainStep {
    /// 0 dB.
    #[default]
    Db0,
    /// +6 dB.
    Db6,
    /// +12 dB.
    Db12,
    /// +18 dB.
    Db18,
}

impl GainStep {
    /// All steps in increasing order.
    pub const ALL: [Self; 4] = [Self::Db0, Self::Db6, Self::Db12, Self::Db18];

    /// Gain in dB.
    pub fn db(self) -> f32 {
        match self {
            Self::Db0 => 0.0,
            Self::Db6 => 6.0,
            Self::Db12 => 12.0,
            Self::Db18 => 18.0,
        }
    }
}

/// Gain mode of the capture chain (§1.1). AGC is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GainSetting {
    /// Automatic gain control.
    #[default]
    Auto,
    /// Fixed gain.
    Manual(GainStep),
}

/// Gain stage implementing manual steps and the AGC of §1.1.
///
/// The AGC gain moves between 0 and +24 dB and only follows sound events:
/// frames at or above -70 dBFS and at least [`AGC_EVENT_MARGIN_DB`] above the
/// tracked room noise floor. For an event the desired gain brings the frame
/// to -24 dBFS; decreases take effect on that frame (well within the 50 ms
/// attack), increases are limited to +3 dB/s. Other frames keep the current
/// gain, so a quiet or steady room is never pumped up between events.
#[derive(Debug, Clone)]
pub struct GainControl {
    setting: GainSetting,
    agc_db: f32,
    applied: f32,
    floor_db: Option<f32>,
}

impl GainControl {
    /// Creates a gain stage.
    pub fn new(setting: GainSetting) -> Self {
        Self {
            setting,
            agc_db: 0.0,
            applied: 1.0,
            floor_db: None,
        }
    }

    /// Current setting.
    pub fn setting(&self) -> GainSetting {
        self.setting
    }

    /// Changes the setting; the AGC restarts from 0 dB.
    pub fn set_setting(&mut self, setting: GainSetting) {
        if setting != self.setting {
            self.setting = setting;
            self.agc_db = 0.0;
            self.floor_db = None;
        }
    }

    /// Current AGC gain in dB (0 in manual mode).
    pub fn agc_gain_db(&self) -> f32 {
        match self.setting {
            GainSetting::Auto => self.agc_db,
            GainSetting::Manual(_) => 0.0,
        }
    }

    /// Tracked room noise floor in dBFS (AGC only).
    pub fn noise_floor_dbfs(&self) -> Option<f32> {
        self.floor_db
    }

    /// Applies gain to one frame in place and returns the room level: the
    /// frame's level after the manual step but before AGC (§1). Thresholds,
    /// meters and level reports use this value, so they do not drift while
    /// the AGC adapts.
    pub fn process(&mut self, frame: &mut PcmFrame) -> f32 {
        let input = level_dbfs(frame);
        let (target_db, room_level) = match self.setting {
            GainSetting::Manual(step) => (step.db(), input + step.db()),
            GainSetting::Auto => {
                let rise = NOISE_FLOOR_RISE_DB_PER_S * FRAME_MS as f32 / 1000.0;
                let floor = match self.floor_db {
                    None => input,
                    Some(f) if input < f => input,
                    Some(f) => (f + rise).min(input),
                };
                self.floor_db = Some(floor);
                let is_event =
                    input >= AGC_NOISE_FLOOR_DBFS && input >= floor + AGC_EVENT_MARGIN_DB;
                if input < AGC_NOISE_FLOOR_DBFS {
                    (0.0, input)
                } else {
                    let desired = (AGC_TARGET_DBFS - input).clamp(0.0, AGC_MAX_GAIN_DB);
                    let max_raise = AGC_RAISE_DB_PER_S * FRAME_MS as f32 / 1000.0;
                    if desired < self.agc_db {
                        // Never let a frame clip past the target, event or not.
                        self.agc_db = desired;
                    } else if is_event {
                        self.agc_db = (self.agc_db + max_raise).min(desired);
                    }
                    (self.agc_db, input)
                }
            }
        };
        let target = db_to_gain(target_db);
        let start = if target < self.applied {
            target
        } else {
            self.applied
        };
        let n = frame.len() as f32;
        for (i, s) in frame.iter_mut().enumerate() {
            let g = start + (target - start) * (i as f32 + 1.0) / n;
            *s = (f32::from(*s) * g).round().clamp(-32768.0, 32767.0) as i16;
        }
        self.applied = target;
        room_level.max(crate::level::LEVEL_FLOOR_DBFS)
    }
}

/// One processed capture frame.
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedFrame {
    /// 16 kHz mono PCM after gain (manual step or AGC).
    pub pcm: PcmFrame,
    /// Room level (§1): after the manual step, before AGC. Used for meters,
    /// `level` records and the sound-activation gate.
    pub level_dbfs: f32,
}

/// Device audio → 20 ms frames of 16 kHz mono with gain and level (§1).
#[derive(Debug, Clone)]
pub struct CaptureChain {
    channels: usize,
    partial: Vec<f32>,
    mono: Vec<f32>,
    resampled: Vec<f32>,
    resampler: Resampler,
    gain: GainControl,
}

impl CaptureChain {
    /// Creates a chain for interleaved device audio of `device_rate` Hz with
    /// `channels` channels.
    pub fn new(device_rate: u32, channels: u16, gain: GainSetting) -> Result<Self, MediaError> {
        if channels == 0 {
            return Err(MediaError::InvalidParameter("channel count"));
        }
        Ok(Self {
            channels: usize::from(channels),
            partial: Vec::new(),
            mono: Vec::new(),
            resampled: Vec::new(),
            resampler: Resampler::new(device_rate, SAMPLE_RATE)?,
            gain: GainControl::new(gain),
        })
    }

    /// Gain stage, e.g. to read the AGC gain.
    pub fn gain(&self) -> &GainControl {
        &self.gain
    }

    /// Changes the gain setting.
    pub fn set_gain(&mut self, setting: GainSetting) {
        self.gain.set_setting(setting);
    }

    /// Feeds interleaved samples in `[-1, 1]`.
    pub fn push_f32(&mut self, interleaved: &[f32]) {
        self.partial.extend_from_slice(interleaved);
        let whole = self.partial.len() / self.channels * self.channels;
        self.mono.clear();
        let scale = 1.0 / self.channels as f32;
        self.mono.extend(
            self.partial[..whole]
                .chunks_exact(self.channels)
                .map(|c| c.iter().sum::<f32>() * scale),
        );
        self.partial.drain(..whole);
        self.resampler.process(&self.mono, &mut self.resampled);
    }

    /// Feeds interleaved 16-bit samples.
    pub fn push_i16(&mut self, interleaved: &[i16]) {
        let as_f32: Vec<f32> = interleaved
            .iter()
            .map(|&s| f32::from(s) / 32768.0)
            .collect();
        self.push_f32(&as_f32);
    }

    /// Number of complete frames ready.
    pub fn frames_ready(&self) -> usize {
        self.resampled.len() / FRAME_SAMPLES
    }

    /// Returns the next complete frame, if any.
    pub fn pop_frame(&mut self) -> Option<CapturedFrame> {
        if self.resampled.len() < FRAME_SAMPLES {
            return None;
        }
        let mut pcm = [0i16; FRAME_SAMPLES];
        for (d, s) in pcm.iter_mut().zip(self.resampled.drain(..FRAME_SAMPLES)) {
            *d = (s * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
        }
        let level_dbfs = self.gain.process(&mut pcm);
        Some(CapturedFrame { pcm, level_dbfs })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone_frame(level_dbfs: f32, phase: &mut f32) -> PcmFrame {
        // Sine with rms at `level_dbfs`.
        let amp = 32768.0 * db_to_gain(level_dbfs) * std::f32::consts::SQRT_2;
        let mut f = [0i16; FRAME_SAMPLES];
        for s in f.iter_mut() {
            *s = (amp * phase.sin()).round() as i16;
            *phase += 2.0 * std::f32::consts::PI * 500.0 / 16_000.0;
        }
        f
    }

    #[test]
    fn media_1_1_manual_gain_steps() {
        let mut phase = 0.0;
        for step in GainStep::ALL {
            let mut g = GainControl::new(GainSetting::Manual(step));
            // Second frame is at steady state (first ramps up).
            let mut f = tone_frame(-40.0, &mut phase);
            g.process(&mut f);
            let mut f = tone_frame(-40.0, &mut phase);
            g.process(&mut f);
            let out = level_dbfs(&f);
            assert!((out - (-40.0 + step.db())).abs() < 0.2, "{step:?}: {out}");
        }
    }

    /// Feeds one frame and returns (room level reported, AGC gain after it).
    fn feed(g: &mut GainControl, level: f32, phase: &mut f32) -> (f32, f32) {
        let mut f = tone_frame(level, phase);
        let room = g.process(&mut f);
        (room, g.agc_gain_db())
    }

    #[test]
    fn media_1_1_agc_raises_slowly_to_target_on_events() {
        let mut g = GainControl::new(GainSetting::Auto);
        let mut phase = 0.0;
        // A quiet room sets the noise floor; then a -40 dBFS sound event wants
        // +16 dB, reached at 3 dB/s after 16/3 s.
        for _ in 0..50 {
            feed(&mut g, -65.0, &mut phase);
        }
        let mut gains = Vec::new();
        for _ in 0..500 {
            gains.push(feed(&mut g, -40.0, &mut phase).1);
        }
        assert!(gains[49] <= 3.0 + 1e-3 && gains[49] > 2.9, "{}", gains[49]);
        for w in gains.windows(2) {
            assert!(w[1] - w[0] <= 0.06 + 1e-4);
        }
        assert!((gains[499] - 16.0).abs() < 0.05, "{}", gains[499]);
        let mut f = tone_frame(-40.0, &mut phase);
        let room = g.process(&mut f);
        assert!((level_dbfs(&f) - AGC_TARGET_DBFS).abs() < 0.3);
        // The reported room level is the level before AGC (§1).
        assert!((room - (-40.0)).abs() < 0.3, "{room}");
    }

    #[test]
    fn media_1_1_agc_max_gain_is_24_db() {
        let mut g = GainControl::new(GainSetting::Auto);
        let mut phase = 0.0;
        // Quiet frames keep the noise floor low between -60 dBFS events.
        for _ in 0..1000 {
            feed(&mut g, -75.0, &mut phase);
            feed(&mut g, -60.0, &mut phase);
        }
        assert!((g.agc_gain_db() - AGC_MAX_GAIN_DB).abs() < 1e-3);
    }

    #[test]
    fn media_1_1_agc_attacks_within_50_ms() {
        let mut g = GainControl::new(GainSetting::Auto);
        let mut phase = 0.0;
        for _ in 0..1000 {
            feed(&mut g, -75.0, &mut phase);
            feed(&mut g, -50.0, &mut phase);
        }
        assert!(g.agc_gain_db() > 23.9);
        // A loud event at -10 dBFS: within 50 ms (2.5 frames) the output is at
        // or below the target level, and the gain never amplifies it.
        let mut f = tone_frame(-10.0, &mut phase);
        g.process(&mut f);
        assert!(level_dbfs(&f) <= -10.0 + 0.1);
        assert_eq!(g.agc_gain_db(), 0.0);
        let mut f = tone_frame(-30.0, &mut phase);
        g.process(&mut f);
        assert!(level_dbfs(&f) < -29.0, "{}", level_dbfs(&f));
    }

    #[test]
    fn media_1_1_agc_never_pumps_up_a_steady_room() {
        // Regression: a steady -60 dBFS room used to be raised by up to 24 dB
        // until it crossed listeners' sound-activation thresholds.
        let mut g = GainControl::new(GainSetting::Auto);
        let mut phase = 0.0;
        for _ in 0..1500 {
            let (room, gain) = feed(&mut g, -60.0, &mut phase);
            assert_eq!(gain, 0.0);
            assert!((room - (-60.0)).abs() < 0.3, "{room}");
        }
        // After a sound event raised the gain, a quiet room keeps it, not more.
        for _ in 0..100 {
            feed(&mut g, -40.0, &mut phase);
        }
        let held = g.agc_gain_db();
        assert!(held > 1.0);
        for _ in 0..500 {
            let (room, gain) = feed(&mut g, -60.0, &mut phase);
            assert_eq!(gain, held);
            assert!(
                (room - (-60.0)).abs() < 0.3,
                "room level stays before AGC: {room}"
            );
        }
    }

    #[test]
    fn media_1_1_agc_does_not_amplify_below_minus_70() {
        let mut g = GainControl::new(GainSetting::Auto);
        let mut phase = 0.0;
        for _ in 0..200 {
            feed(&mut g, -75.0, &mut phase);
            feed(&mut g, -40.0, &mut phase);
        }
        let before = g.agc_gain_db();
        assert!(before > 3.0);
        // Frames below -70 dBFS pass at unity and freeze the AGC state.
        for _ in 0..3 {
            let mut f = tone_frame(-75.0, &mut phase);
            g.process(&mut f);
            assert!((level_dbfs(&f) - (-75.0)).abs() < 0.5, "{}", level_dbfs(&f));
        }
        assert_eq!(g.agc_gain_db(), before);
    }

    #[test]
    fn media_1_capture_chain_stereo_48k_to_frames() {
        let mut c = CaptureChain::new(48_000, 2, GainSetting::Manual(GainStep::Db0)).unwrap();
        let amp = 0.25f32;
        let mut input = Vec::new();
        for i in 0..48_000 {
            let s = amp * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 48_000.0).sin();
            input.push(s);
            input.push(s);
        }
        // Odd chunk sizes split interleaved pairs.
        for chunk in input.chunks(333) {
            c.push_f32(chunk);
        }
        let mut frames = Vec::new();
        while let Some(f) = c.pop_frame() {
            frames.push(f);
        }
        assert!(frames.len() >= 49, "{}", frames.len());
        // rms of the sine: 0.25/sqrt(2) → -15.05 dBFS.
        let l = frames[10].level_dbfs;
        assert!((l - (-15.05)).abs() < 0.1, "{l}");
    }

    #[test]
    fn media_1_capture_chain_i16_mono() {
        let mut c = CaptureChain::new(16_000, 1, GainSetting::Manual(GainStep::Db6)).unwrap();
        c.push_i16(&[1000; 640]);
        assert_eq!(c.frames_ready(), 2);
        c.pop_frame().unwrap();
        let f = c.pop_frame().unwrap();
        assert!((i32::from(f.pcm[100]) - 1995).abs() <= 1, "{}", f.pcm[100]);
        assert!(c.pop_frame().is_none());
    }
}
