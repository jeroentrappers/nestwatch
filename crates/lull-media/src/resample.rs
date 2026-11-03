// SPDX-License-Identifier: AGPL-3.0-or-later
//! Streaming windowed-sinc sample-rate converter (media.md §1).
//!
//! A Kaiser-windowed sinc kernel with 16 zero crossings per side is tabulated
//! once and linearly interpolated, so any rate pair works with exact rational
//! position tracking and no drift.

use crate::MediaError;

const ZERO_CROSSINGS: usize = 16;
const TABLE_RES: usize = 512;
const KAISER_BETA: f64 = 8.6;
/// Pass band edge as a fraction of the lower of the two Nyquist frequencies.
const CUTOFF: f64 = 0.92;
/// Highest supported input or output rate.
pub const MAX_RATE: u32 = 384_000;

/// Mono streaming resampler working on `f32` samples.
#[derive(Debug, Clone)]
pub struct Resampler {
    in_rate: u32,
    out_rate: u32,
    passthrough: bool,
    /// Kernel bandwidth `2·fc` in cycles per input sample.
    bw: f64,
    /// Kernel half width in whole input samples.
    half: usize,
    table: Vec<f32>,
    buf: Vec<f32>,
    pos_int: usize,
    pos_frac: u64,
}

fn bessel_i0(x: f64) -> f64 {
    let mut sum = 1.0;
    let mut term = 1.0;
    let q = x * x / 4.0;
    for k in 1..64 {
        term *= q / (k as f64 * k as f64);
        sum += term;
        if term < sum * 1e-17 {
            break;
        }
    }
    sum
}

impl Resampler {
    /// Creates a resampler from `in_rate` to `out_rate` Hz.
    pub fn new(in_rate: u32, out_rate: u32) -> Result<Self, MediaError> {
        if in_rate == 0 || out_rate == 0 || in_rate > MAX_RATE || out_rate > MAX_RATE {
            return Err(MediaError::InvalidParameter("sample rate"));
        }
        let bw = CUTOFF * (f64::from(out_rate) / f64::from(in_rate)).min(1.0);
        let half = (ZERO_CROSSINGS as f64 / bw).ceil() as usize;
        let n = ZERO_CROSSINGS * TABLE_RES;
        let i0b = bessel_i0(KAISER_BETA);
        let mut table = Vec::with_capacity(n + 2);
        for i in 0..=n + 1 {
            let x = i as f64 / TABLE_RES as f64;
            let sinc = if i == 0 {
                1.0
            } else {
                let px = std::f64::consts::PI * x;
                px.sin() / px
            };
            let r = x / ZERO_CROSSINGS as f64;
            let w = if r >= 1.0 {
                0.0
            } else {
                bessel_i0(KAISER_BETA * (1.0 - r * r).sqrt()) / i0b
            };
            table.push((sinc * w * bw) as f32);
        }
        Ok(Self {
            in_rate,
            out_rate,
            passthrough: in_rate == out_rate,
            bw,
            half,
            table,
            buf: vec![0.0; half],
            pos_int: half,
            pos_frac: 0,
        })
    }

    /// Input rate in Hz.
    pub fn in_rate(&self) -> u32 {
        self.in_rate
    }

    /// Output rate in Hz.
    pub fn out_rate(&self) -> u32 {
        self.out_rate
    }

    fn kernel(&self, d: f64) -> f32 {
        let u = d.abs() * self.bw * TABLE_RES as f64;
        let i = u as usize;
        if i >= ZERO_CROSSINGS * TABLE_RES {
            return 0.0;
        }
        let f = (u - i as f64) as f32;
        self.table[i] + (self.table[i + 1] - self.table[i]) * f
    }

    /// Feeds mono `input` and appends every output sample that is now
    /// computable to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.passthrough {
            out.extend_from_slice(input);
            return;
        }
        self.buf.extend_from_slice(input);
        let h = self.half;
        let out_rate = u64::from(self.out_rate);
        let in_rate = u64::from(self.in_rate);
        while self.pos_int + h < self.buf.len() {
            let frac = self.pos_frac as f64 / out_rate as f64;
            let start = self.pos_int + 1 - h;
            let end = self.pos_int + h;
            let mut acc = 0.0f32;
            for (k, &x) in self.buf[start..=end].iter().enumerate() {
                let d = (start + k) as f64 - (self.pos_int as f64 + frac);
                acc += x * self.kernel(d);
            }
            out.push(acc);
            self.pos_frac += in_rate;
            let carry = self.pos_frac / out_rate;
            self.pos_frac %= out_rate;
            self.pos_int += carry as usize;
        }
        let drop = (self.pos_int + 1).saturating_sub(h).min(self.buf.len());
        self.buf.drain(..drop);
        self.pos_int -= drop;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, freq: f32, n: usize, amp: f32) -> Vec<f32> {
        (0..n)
            .map(|i| amp * (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin())
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
    }

    /// Fits `a·sin + b·cos` at `freq` and returns (amplitude, residual rms).
    fn fit(x: &[f32], rate: u32, freq: f32) -> (f32, f32) {
        let w = 2.0 * std::f64::consts::PI * f64::from(freq) / f64::from(rate);
        let (mut a, mut b) = (0.0f64, 0.0f64);
        for (i, &v) in x.iter().enumerate() {
            a += f64::from(v) * (w * i as f64).sin();
            b += f64::from(v) * (w * i as f64).cos();
        }
        let n = x.len() as f64;
        a *= 2.0 / n;
        b *= 2.0 / n;
        let res: Vec<f32> = x
            .iter()
            .enumerate()
            .map(|(i, &v)| v - (a * (w * i as f64).sin() + b * (w * i as f64).cos()) as f32)
            .collect();
        ((a * a + b * b).sqrt() as f32, rms(&res))
    }

    #[test]
    fn media_1_resample_48k_preserves_tone_and_count() {
        let mut r = Resampler::new(48_000, 16_000).unwrap();
        let input = sine(48_000, 1000.0, 48_000, 0.5);
        let mut out = Vec::new();
        for chunk in input.chunks(441) {
            r.process(chunk, &mut out);
        }
        assert!((out.len() as i64 - 16_000).abs() < 40, "{}", out.len());
        let (amp, res) = fit(&out[200..15_800], 16_000, 1000.0);
        assert!((amp - 0.5).abs() < 0.01, "amp {amp}");
        assert!(res < 0.5 * 1e-3, "residual {res}");
    }

    #[test]
    fn media_1_resample_rejects_aliases() {
        // 12 kHz is above the 8 kHz output Nyquist: it must be strongly attenuated.
        let mut r = Resampler::new(44_100, 16_000).unwrap();
        let mut out = Vec::new();
        r.process(&sine(44_100, 12_000.0, 44_100, 0.5), &mut out);
        let level = rms(&out[200..out.len() - 200]);
        assert!(level < 0.5 * 1e-3, "alias rms {level}");
    }

    #[test]
    fn media_1_resample_upsampling_8k() {
        let mut r = Resampler::new(8_000, 16_000).unwrap();
        let mut out = Vec::new();
        r.process(&sine(8_000, 440.0, 8_000, 0.25), &mut out);
        let (amp, res) = fit(&out[200..out.len() - 200], 16_000, 440.0);
        assert!((amp - 0.25).abs() < 0.005, "amp {amp}");
        assert!(res < 1e-3, "residual {res}");
    }

    #[test]
    fn media_1_resample_passthrough() {
        let mut r = Resampler::new(16_000, 16_000).unwrap();
        let mut out = Vec::new();
        r.process(&[0.1, 0.2, 0.3], &mut out);
        assert_eq!(out, vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn media_1_resample_rejects_bad_rates() {
        assert!(Resampler::new(0, 16_000).is_err());
        assert!(Resampler::new(1_000_000, 16_000).is_err());
    }
}
