// SPDX-License-Identifier: AGPL-3.0-or-later
//! Per-frame level metering (media.md §1).

/// Lowest reported level in dBFS.
pub const LEVEL_FLOOR_DBFS: f32 = -90.0;

/// RMS level of `samples` in dBFS: `20·log10(rms / 32768)`, floored at -90.
pub fn level_dbfs(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return LEVEL_FLOOR_DBFS;
    }
    let sum: f64 = samples
        .iter()
        .map(|&s| {
            let v = f64::from(s);
            v * v
        })
        .sum();
    let rms = (sum / samples.len() as f64).sqrt();
    if rms <= 0.0 {
        return LEVEL_FLOOR_DBFS;
    }
    let db = 20.0 * (rms / 32768.0).log10();
    (db as f32).max(LEVEL_FLOOR_DBFS)
}

/// Converts decibels to a linear amplitude factor.
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Converts a linear amplitude factor to decibels (floored at -90 dB).
pub fn gain_to_db(gain: f32) -> f32 {
    if gain <= 0.0 {
        LEVEL_FLOOR_DBFS
    } else {
        (20.0 * gain.log10()).max(LEVEL_FLOOR_DBFS)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_1_level_of_silence_is_floor() {
        assert_eq!(level_dbfs(&[0; 320]), LEVEL_FLOOR_DBFS);
        assert_eq!(level_dbfs(&[]), LEVEL_FLOOR_DBFS);
        // rms 1 LSB is about -90.3 dBFS, still floored.
        assert_eq!(level_dbfs(&[1; 320]), LEVEL_FLOOR_DBFS);
    }

    #[test]
    fn media_1_level_of_full_scale_square_is_zero() {
        let frame: Vec<i16> = (0..320)
            .map(|i| if i % 2 == 0 { -32768 } else { 32767 })
            .collect();
        assert!(level_dbfs(&frame).abs() < 0.01);
    }

    #[test]
    fn media_1_level_of_sine() {
        // A sine of amplitude A has rms A/sqrt(2): -3.01 dB below its peak.
        let amp = 32768.0 * db_to_gain(-6.0);
        let frame: Vec<i16> = (0..320)
            .map(|i| {
                (amp * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 16000.0).sin()) as i16
            })
            .collect();
        let l = level_dbfs(&frame);
        assert!((l - (-9.03)).abs() < 0.1, "{l}");
    }
}
