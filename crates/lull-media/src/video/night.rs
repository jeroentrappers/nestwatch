// SPDX-License-Identifier: AGPL-3.0-or-later
//! Night image: grayscale with a shadow-lifting gamma curve (media.md §5).

/// Exponent of the tone curve `out = in^NIGHT_GAMMA` (on 0..1); below 1 it
/// lifts shadows while keeping black and white fixed.
pub const NIGHT_GAMMA: f32 = 0.6;

fn lut() -> [u8; 256] {
    let mut t = [0u8; 256];
    for (i, v) in t.iter_mut().enumerate() {
        *v = (255.0 * (i as f32 / 255.0).powf(NIGHT_GAMMA)).round() as u8;
    }
    t
}

/// Applies the night filter in place to RGB (`channels` = 3) or RGBA (4)
/// pixels; alpha is untouched.
pub fn night_filter(pixels: &mut [u8], channels: usize) {
    if channels < 3 {
        return;
    }
    let t = lut();
    for px in pixels.chunks_exact_mut(channels) {
        let y =
            (299 * u32::from(px[0]) + 587 * u32::from(px[1]) + 114 * u32::from(px[2]) + 500) / 1000;
        let v = t[y as usize];
        px[..3].fill(v);
    }
}

/// Applies the night filter in place to I420 planes.
pub fn night_filter_i420(y: &mut [u8], u: &mut [u8], v: &mut [u8]) {
    let t = lut();
    y.iter_mut().for_each(|p| *p = t[usize::from(*p)]);
    u.fill(128);
    v.fill(128);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_5_night_grayscale_lifts_shadows() {
        let mut px = vec![
            200, 30, 30, 255, 20, 20, 20, 7, 0, 0, 0, 0, 255, 255, 255, 1,
        ];
        night_filter(&mut px, 4);
        for p in px.chunks(4) {
            assert!(p[0] == p[1] && p[1] == p[2]);
        }
        assert!(px[4] > 40, "shadow lifted: {}", px[4]);
        assert_eq!(px[7], 7, "alpha untouched");
        assert_eq!(px[8], 0);
        assert_eq!(px[12], 255);
        let (mut y, mut u, mut v) = (vec![16u8, 128, 255], vec![90u8], vec![200u8]);
        night_filter_i420(&mut y, &mut u, &mut v);
        assert!(y[0] > 16 && y[1] > 128 && y[2] == 255);
        assert_eq!((u[0], v[0]), (128, 128));
    }
}
