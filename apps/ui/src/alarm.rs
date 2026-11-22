// SPDX-License-Identifier: AGPL-3.0-or-later
//! The escalating alarm sound (product.md FR-AL-5), generated in code: an
//! original repeating pattern of three rising tones and a pause, mixed into
//! the listener's 16 kHz output by the desktop and browser hosts. The level
//! ramps from 30 % to full over 30 s; it does not depend on any station's
//! volume or mute.

/// Output sample rate of the listener (CORE-12).
pub const RATE: u32 = 16_000;
/// One cycle of the pattern: three 140 ms tones, 60 ms gaps, then a pause.
pub const CYCLE_SAMPLES: u64 = RATE as u64 * 1_300 / 1_000;
/// Ramp from [`START_GAIN`] to full.
pub const RAMP_SAMPLES: u64 = RATE as u64 * 30;
/// Starting level of the ramp.
pub const START_GAIN: f32 = 0.3;
/// Peak amplitude at full level (of full scale).
pub const PEAK: f32 = 0.7;

const TONE: u64 = RATE as u64 * 140 / 1_000;
const STEP: u64 = RATE as u64 * 200 / 1_000;
const FREQS: [f32; 3] = [740.0, 988.0, 1_245.0];
const FADE: u64 = 80;

/// The ramp gain after `pos` samples of ringing.
pub fn ramp(pos: u64) -> f32 {
    let f = (pos.min(RAMP_SAMPLES) as f32) / RAMP_SAMPLES as f32;
    START_GAIN + (1.0 - START_GAIN) * f
}

/// The alarm sample (−1..1) at `pos` samples since it started ringing.
pub fn sample(pos: u64) -> f32 {
    let in_cycle = pos % CYCLE_SAMPLES;
    let tone = in_cycle / STEP;
    let in_tone = in_cycle % STEP;
    if tone >= FREQS.len() as u64 || in_tone >= TONE {
        return 0.0;
    }
    let env = (in_tone.min(TONE - 1 - in_tone) as f32 / FADE as f32).min(1.0);
    let t = in_tone as f32 / RATE as f32;
    let f = FREQS[tone as usize];
    // A fundamental with a soft third harmonic: bright but not harsh.
    let w = 2.0 * std::f32::consts::PI * f * t;
    let v = w.sin() * 0.8 + (3.0 * w).sin() * 0.2;
    v * env * PEAK * ramp(pos)
}

/// Mixes the alarm into 16 kHz `i16` samples; `pos` advances.
pub fn mix_i16(frame: &mut [i16], pos: &mut u64) {
    for x in frame.iter_mut() {
        let v = f32::from(*x) + sample(*pos) * 32_767.0;
        *x = v.clamp(-32_768.0, 32_767.0) as i16;
        *pos += 1;
    }
}

/// Mixes the alarm into 16 kHz `f32` samples (−1..1); `pos` advances.
pub fn mix_f32(frame: &mut [f32], pos: &mut u64) {
    for x in frame.iter_mut() {
        *x = (*x + sample(*pos)).clamp(-1.0, 1.0);
        *pos += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn peak(from: u64, len: u64) -> f32 {
        (from..from + len)
            .map(|p| sample(p).abs())
            .fold(0.0, f32::max)
    }

    #[test]
    fn the_pattern_repeats_with_a_pause() {
        assert!(peak(0, TONE) > 0.1);
        assert_eq!(peak(3 * STEP, CYCLE_SAMPLES - 3 * STEP), 0.0, "pause");
        assert_eq!(peak(TONE, STEP - TONE), 0.0, "gap between tones");
        let c = CYCLE_SAMPLES;
        assert!((sample(500) / ramp(500) - sample(500 + c) / ramp(500 + c)).abs() < 1e-3);
    }

    #[test]
    fn the_level_ramps_from_30_percent_over_30_seconds() {
        assert!((ramp(0) - 0.3).abs() < 1e-6);
        assert!((ramp(RAMP_SAMPLES / 2) - 0.65).abs() < 1e-3);
        assert_eq!(ramp(RAMP_SAMPLES), 1.0);
        assert_eq!(ramp(RAMP_SAMPLES * 4), 1.0);
        let start = peak(0, CYCLE_SAMPLES);
        let full = peak(
            RAMP_SAMPLES - RAMP_SAMPLES % CYCLE_SAMPLES + CYCLE_SAMPLES,
            CYCLE_SAMPLES,
        );
        assert!(full / start > 3.0 && full <= PEAK + 1e-3, "{start} {full}");
    }

    #[test]
    fn mixing_adds_and_clamps() {
        let mut pos = 0;
        let mut f = [0i16; 320];
        mix_i16(&mut f, &mut pos);
        assert_eq!(pos, 320);
        assert!(f.iter().any(|s| s.abs() > 1_000));
        let mut g = [1.0f32; 320];
        let mut p2 = 0;
        mix_f32(&mut g, &mut p2);
        assert!(g.iter().all(|v| *v <= 1.0));
    }
}
