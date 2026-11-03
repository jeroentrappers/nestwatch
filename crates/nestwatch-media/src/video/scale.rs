// SPDX-License-Identifier: AGPL-3.0-or-later
//! Aspect-preserving scaling within the listener's limits (media.md §5).

/// Largest size within `max_w × max_h` with the aspect ratio of
/// `src_w × src_h`. Never upscales; each side is at least 1.
pub fn fit_within(src_w: u32, src_h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if src_w == 0 || src_h == 0 {
        return (0, 0);
    }
    let (max_w, max_h) = (max_w.max(1), max_h.max(1));
    if src_w <= max_w && src_h <= max_h {
        return (src_w, src_h);
    }
    let (sw, sh) = (u64::from(src_w), u64::from(src_h));
    if sw * u64::from(max_h) > u64::from(max_w) * sh {
        let h = (sh * u64::from(max_w) + sw / 2) / sw;
        (max_w, (h as u32).clamp(1, max_h))
    } else {
        let w = (sw * u64::from(max_h) + sh / 2) / sh;
        ((w as u32).clamp(1, max_w), max_h)
    }
}

/// Resizes a tightly packed image with `channels` bytes per pixel using area
/// averaging (box filter), which suits the downscaling done here.
///
/// # Panics
/// If `src` is shorter than `src_w × src_h × channels`.
pub fn scale_image(
    src: &[u8],
    src_w: u32,
    src_h: u32,
    channels: usize,
    dst_w: u32,
    dst_h: u32,
) -> Vec<u8> {
    let (sw, sh, dw, dh) = (
        src_w as usize,
        src_h as usize,
        dst_w as usize,
        dst_h as usize,
    );
    assert!(src.len() >= sw * sh * channels, "source buffer too small");
    let mut out = vec![0u8; dw * dh * channels];
    if sw == 0 || sh == 0 {
        return out;
    }
    let span = |d: usize, dn: usize, sn: usize| {
        let a = d * sn / dn;
        let b = ((d + 1) * sn / dn).max(a + 1).min(sn);
        (a, b)
    };
    let mut acc = vec![0u32; channels];
    for dy in 0..dh {
        let (y0, y1) = span(dy, dh, sh);
        for dx in 0..dw {
            let (x0, x1) = span(dx, dw, sw);
            acc.iter_mut().for_each(|a| *a = 0);
            for y in y0..y1 {
                let row = &src[(y * sw + x0) * channels..(y * sw + x1) * channels];
                for px in row.chunks_exact(channels) {
                    for (a, &v) in acc.iter_mut().zip(px) {
                        *a += u32::from(v);
                    }
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let o = (dy * dw + dx) * channels;
            for (c, a) in acc.iter().enumerate() {
                out[o + c] = ((a + n / 2) / n) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_5_fit_within_preserves_aspect() {
        assert_eq!(fit_within(1920, 1080, 640, 480), (640, 360));
        assert_eq!(fit_within(1080, 1920, 640, 480), (270, 480));
        assert_eq!(fit_within(640, 480, 640, 480), (640, 480));
        assert_eq!(fit_within(320, 240, 640, 480), (320, 240), "never upscales");
        assert_eq!(fit_within(4000, 10, 100, 100), (100, 1));
        assert_eq!(fit_within(1280, 720, 0, 0), (1, 1));
    }

    #[test]
    fn media_5_scale_box_filter() {
        // 4x2 → 2x1: each output averages a 2x2 block.
        let src: Vec<u8> = vec![0, 10, 20, 30, 40, 50, 60, 70];
        assert_eq!(scale_image(&src, 4, 2, 1, 2, 1), vec![25, 45]);
        // RGB solid colour stays solid.
        let rgb: Vec<u8> = [9u8, 99, 199].repeat(100 * 50);
        let out = scale_image(&rgb, 100, 50, 3, 37, 19);
        assert_eq!(out.len(), 37 * 19 * 3);
        assert!(out.chunks(3).all(|p| p == [9, 99, 199]));
    }
}
