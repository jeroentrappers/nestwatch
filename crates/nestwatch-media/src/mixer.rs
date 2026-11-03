// SPDX-License-Identifier: AGPL-3.0-or-later
//! Listener mixer with per-station volume, mute, solo, talk-back mute and a
//! soft limiter (media.md §4.2, product.md FR-AU-6, FR-TB-2).

use crate::level::{LEVEL_FLOOR_DBFS, db_to_gain, level_dbfs};
use crate::{FRAME_SAMPLES, PcmFrame};

/// Limiter knee start: -3 dBFS (§4.2).
pub const LIMITER_THRESHOLD_DBFS: f32 = -3.0;

/// Static soft-limiter curve on normalised samples: identity up to -3 dBFS,
/// then a tanh knee with unit slope at the threshold that approaches but
/// never reaches full scale.
pub fn soft_limit(x: f32) -> f32 {
    let t = db_to_gain(LIMITER_THRESHOLD_DBFS);
    let a = x.abs();
    if a <= t {
        x
    } else {
        let room = 1.0 - t;
        let y = t + room * ((a - t) / room).tanh();
        y.copysign(x)
    }
}

/// Handle of a station in the mixer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StationHandle(u32);

#[derive(Debug, Clone)]
struct Channel {
    handle: StationHandle,
    volume: f32,
    muted: bool,
    talking: bool,
    level_dbfs: f32,
    gain: f32,
}

/// Mixes the decoded frames of several stations into one output frame.
#[derive(Debug, Clone, Default)]
pub struct Mixer {
    channels: Vec<Channel>,
    next: u32,
    solo: Option<StationHandle>,
}

impl Mixer {
    /// Creates an empty mixer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a station at full volume.
    pub fn add_station(&mut self) -> StationHandle {
        let handle = StationHandle(self.next);
        self.next += 1;
        self.channels.push(Channel {
            handle,
            volume: 1.0,
            muted: false,
            talking: false,
            level_dbfs: LEVEL_FLOOR_DBFS,
            gain: 0.0,
        });
        handle
    }

    /// Removes a station.
    pub fn remove_station(&mut self, h: StationHandle) {
        self.channels.retain(|c| c.handle != h);
        if self.solo == Some(h) {
            self.solo = None;
        }
    }

    fn channel(&mut self, h: StationHandle) -> Option<&mut Channel> {
        self.channels.iter_mut().find(|c| c.handle == h)
    }

    /// Sets a station's linear volume, clamped to `0..=1`.
    pub fn set_volume(&mut self, h: StationHandle, volume: f32) {
        if let Some(c) = self.channel(h) {
            c.volume = if volume.is_nan() {
                0.0
            } else {
                volume.clamp(0.0, 1.0)
            };
        }
    }

    /// Mutes or unmutes a station. Levels keep updating while muted.
    pub fn set_muted(&mut self, h: StationHandle, muted: bool) {
        if let Some(c) = self.channel(h) {
            c.muted = muted;
        }
    }

    /// Marks whether the user is talking to this station; its playback is
    /// muted meanwhile (FR-TB-2).
    pub fn set_talking(&mut self, h: StationHandle, talking: bool) {
        if let Some(c) = self.channel(h) {
            c.talking = talking;
        }
    }

    /// Makes only `h` audible, or clears solo with `None`.
    pub fn set_solo(&mut self, h: Option<StationHandle>) {
        self.solo = h;
    }

    /// Level of the station's last input frame, before volume and mute.
    pub fn station_level_dbfs(&self, h: StationHandle) -> Option<f32> {
        self.channels
            .iter()
            .find(|c| c.handle == h)
            .map(|c| c.level_dbfs)
    }

    /// Whether the station is currently audible.
    pub fn is_audible(&self, h: StationHandle) -> bool {
        self.channels
            .iter()
            .find(|c| c.handle == h)
            .is_some_and(|c| self.target_gain(c) > 0.0)
    }

    fn target_gain(&self, c: &Channel) -> f32 {
        if c.muted || c.talking || self.solo.is_some_and(|s| s != c.handle) {
            0.0
        } else {
            c.volume
        }
    }

    /// Mixes one frame. Stations without an entry in `inputs` contribute
    /// silence. Gain changes ramp over the frame to avoid clicks.
    pub fn mix(&mut self, inputs: &[(StationHandle, &PcmFrame)]) -> PcmFrame {
        let mut acc = [0f32; FRAME_SAMPLES];
        let targets: Vec<f32> = self.channels.iter().map(|c| self.target_gain(c)).collect();
        for (c, target) in self.channels.iter_mut().zip(targets) {
            let input = inputs.iter().find(|(h, _)| *h == c.handle).map(|(_, f)| *f);
            c.level_dbfs = input.map_or(LEVEL_FLOOR_DBFS, |f| level_dbfs(f));
            let start = c.gain;
            c.gain = target;
            let Some(frame) = input else {
                continue;
            };
            if start == 0.0 && target == 0.0 {
                continue;
            }
            for (i, (a, &s)) in acc.iter_mut().zip(frame.iter()).enumerate() {
                let g = start + (target - start) * (i as f32 + 1.0) / FRAME_SAMPLES as f32;
                *a += f32::from(s) / 32768.0 * g;
            }
        }
        let mut out = [0i16; FRAME_SAMPLES];
        for (o, a) in out.iter_mut().zip(acc) {
            *o = (soft_limit(a) * 32767.0).round() as i16;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(amp: f32) -> PcmFrame {
        let mut f = [0i16; FRAME_SAMPLES];
        for (i, s) in f.iter_mut().enumerate() {
            *s = (amp * (2.0 * std::f32::consts::PI * 400.0 * i as f32 / 16_000.0).sin()) as i16;
        }
        f
    }

    #[test]
    fn media_4_2_limiter_curve() {
        let t = db_to_gain(-3.0);
        assert_eq!(soft_limit(0.5), 0.5);
        assert_eq!(soft_limit(-t), -t);
        // Smooth: slope just above the knee is close to 1.
        let e = 1e-3;
        let slope = (soft_limit(t + e) - soft_limit(t)) / e;
        assert!((slope - 1.0).abs() < 0.01, "{slope}");
        // Monotonic and bounded below full scale.
        let mut prev = 0.0;
        for i in 0..1000 {
            let y = soft_limit(i as f32 / 100.0);
            assert!(y >= prev && y <= 1.0);
            prev = y;
        }
    }

    #[test]
    fn media_4_2_several_loud_stations_do_not_clip_harshly() {
        let mut m = Mixer::new();
        let hs: Vec<_> = (0..2).map(|_| m.add_station()).collect();
        let f = tone(30_000.0);
        let inputs: Vec<_> = hs.iter().map(|&h| (h, &f)).collect();
        m.mix(&inputs);
        let out = m.mix(&inputs);
        let peak = out.iter().map(|s| i32::from(*s).abs()).max().unwrap();
        assert!(peak < 32767, "hard clip: {peak}");
        assert!(peak > 30_000);
        // Four stations saturate smoothly without wrapping.
        let hs4: Vec<_> = (0..2).map(|_| m.add_station()).collect();
        let all: Vec<_> = hs.iter().chain(&hs4).map(|&h| (h, &f)).collect();
        m.mix(&all);
        let out4 = m.mix(&all);
        assert!(
            out4.iter()
                .zip(f.iter())
                .all(|(&o, &i)| o.signum() == i.signum())
        );
        // Quiet signals pass unchanged.
        let q = tone(1000.0);
        let mut m2 = Mixer::new();
        let h = m2.add_station();
        m2.mix(&[(h, &q)]);
        assert_eq!(m2.mix(&[(h, &q)]), q);
    }

    #[test]
    fn media_4_2_volume_mute_and_levels() {
        let mut m = Mixer::new();
        let a = m.add_station();
        let b = m.add_station();
        let f = tone(10_000.0);
        m.set_volume(a, 0.5);
        m.set_muted(b, true);
        m.mix(&[(a, &f), (b, &f)]);
        let out = m.mix(&[(a, &f), (b, &f)]);
        assert!((level_dbfs(&out) - (level_dbfs(&f) - 6.02)).abs() < 0.1);
        // The muted station's level is still measured (FR-AU-6).
        assert!((m.station_level_dbfs(b).unwrap() - level_dbfs(&f)).abs() < 1e-3);
        assert!(!m.is_audible(b));
    }

    #[test]
    fn media_4_2_solo_keeps_levels() {
        let mut m = Mixer::new();
        let a = m.add_station();
        let b = m.add_station();
        let fa = tone(8000.0);
        let fb = tone(-8000.0);
        m.set_solo(Some(a));
        m.mix(&[(a, &fa), (b, &fb)]);
        let out = m.mix(&[(a, &fa), (b, &fb)]);
        assert_eq!(out, fa);
        assert!(m.station_level_dbfs(b).unwrap() > -20.0);
        m.set_solo(None);
        m.mix(&[(a, &fa), (b, &fb)]);
        let out = m.mix(&[(a, &fa), (b, &fb)]);
        assert!(out.iter().all(|&s| s.abs() <= 1), "opposite tones cancel");
    }

    #[test]
    fn media_4_2_talking_mutes_that_station() {
        let mut m = Mixer::new();
        let a = m.add_station();
        let b = m.add_station();
        let f = tone(8000.0);
        m.set_talking(a, true);
        m.mix(&[(a, &f), (b, &f)]);
        let out = m.mix(&[(a, &f), (b, &f)]);
        assert_eq!(out, f, "only b audible");
        m.set_talking(a, false);
        assert!(m.is_audible(a));
    }

    #[test]
    fn media_4_2_gain_changes_ramp() {
        let mut m = Mixer::new();
        let a = m.add_station();
        let f = [10_000i16; FRAME_SAMPLES];
        let out = m.mix(&[(a, &f)]);
        // Ramps up from 0 within the first frame.
        assert!(out[0] < 100 && out[FRAME_SAMPLES - 1] == 10_000);
        m.remove_station(a);
        assert!(m.station_level_dbfs(a).is_none());
    }
}
