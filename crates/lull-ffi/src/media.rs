// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sample and pixel conversions at the FFI boundary.

use lull_core::station::{CameraFrame, PixelFormat};

/// Little-endian 16-bit PCM bytes → samples (a trailing odd byte is
/// dropped).
pub(crate) fn i16_from_le(bytes: &[u8]) -> Vec<i16> {
    bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect()
}

/// Little-endian 32-bit float PCM bytes → samples.
pub(crate) fn f32_from_le(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Samples → little-endian 16-bit PCM bytes.
pub(crate) fn i16_to_le(samples: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// Pixel layout of a [`CameraImage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ImageFormat {
    /// 4 bytes per pixel (R, G, B, A), alpha ignored.
    Rgba,
    /// 3 bytes per pixel.
    Rgb,
    /// Planar YUV 4:2:0: full-size Y plane, then quarter-size U and V
    /// planes, tight strides, full-range BT.601.
    I420,
}

/// One camera image, tightly packed (no row padding).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CameraImage {
    /// Width in pixels as stored.
    pub width: u32,
    /// Height in pixels as stored.
    pub height: u32,
    /// Layout of `data`.
    pub format: ImageFormat,
    /// Clockwise rotation in degrees (0, 90, 180, 270) that makes the
    /// image upright, e.g. CameraX `ImageInfo.rotationDegrees`.
    pub rotation: u16,
    /// Pixels.
    pub data: Vec<u8>,
}

/// Converts to the engine's frame type; `None` if the data size does not
/// match the dimensions.
pub(crate) fn to_frame(img: CameraImage) -> Option<CameraFrame> {
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 || w > 8192 || h > 8192 {
        return None;
    }
    let (format, data) = match img.format {
        ImageFormat::Rgba if img.data.len() == w * h * 4 => (PixelFormat::Rgba, img.data),
        ImageFormat::Rgb if img.data.len() == w * h * 3 => (PixelFormat::Rgb, img.data),
        ImageFormat::I420 => (PixelFormat::Rgb, i420_to_rgb(&img.data, w, h)?),
        _ => return None,
    };
    let bpp = match format {
        PixelFormat::Rgba => 4,
        PixelFormat::Rgb => 3,
    };
    let (data, w, h) = rotate(data, w, h, bpp, img.rotation);
    Some(CameraFrame {
        width: w as u32,
        height: h as u32,
        format,
        data,
    })
}

/// Full-range BT.601 YUV 4:2:0 → RGB.
fn i420_to_rgb(data: &[u8], w: usize, h: usize) -> Option<Vec<u8>> {
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    if data.len() != w * h + 2 * cw * ch {
        return None;
    }
    let (y, rest) = data.split_at(w * h);
    let (u, v) = rest.split_at(cw * ch);
    let mut out = Vec::with_capacity(w * h * 3);
    for row in 0..h {
        for col in 0..w {
            let yy = f32::from(y[row * w + col]);
            let ci = (row / 2) * cw + col / 2;
            let uu = f32::from(u[ci]) - 128.0;
            let vv = f32::from(v[ci]) - 128.0;
            let clamp = |x: f32| x.round().clamp(0.0, 255.0) as u8;
            out.push(clamp(yy + 1.402 * vv));
            out.push(clamp(yy - 0.344_136 * uu - 0.714_136 * vv));
            out.push(clamp(yy + 1.772 * uu));
        }
    }
    Some(out)
}

/// Rotates clockwise by 90°, 180° or 270°; other values leave the image.
fn rotate(data: Vec<u8>, w: usize, h: usize, bpp: usize, deg: u16) -> (Vec<u8>, usize, usize) {
    let src = |x: usize, y: usize| (y * w + x) * bpp;
    match deg % 360 {
        90 => {
            // New size h × w; new (x, y) takes old (y, h - 1 - x).
            let mut out = vec![0; data.len()];
            for ny in 0..w {
                for nx in 0..h {
                    let s = src(ny, h - 1 - nx);
                    let d = (ny * h + nx) * bpp;
                    out[d..d + bpp].copy_from_slice(&data[s..s + bpp]);
                }
            }
            (out, h, w)
        }
        180 => {
            let mut out = vec![0; data.len()];
            for y in 0..h {
                for x in 0..w {
                    let s = src(x, y);
                    let d = src(w - 1 - x, h - 1 - y);
                    out[d..d + bpp].copy_from_slice(&data[s..s + bpp]);
                }
            }
            (out, w, h)
        }
        270 => {
            // New (x, y) takes old (w - 1 - y, x).
            let mut out = vec![0; data.len()];
            for ny in 0..w {
                for nx in 0..h {
                    let s = src(w - 1 - ny, nx);
                    let d = (ny * h + nx) * bpp;
                    out[d..d + bpp].copy_from_slice(&data[s..s + bpp]);
                }
            }
            (out, h, w)
        }
        _ => (data, w, h),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcm_roundtrip() {
        let s = [0i16, 1, -1, i16::MAX, i16::MIN];
        assert_eq!(i16_from_le(&i16_to_le(&s)), s);
        assert_eq!(f32_from_le(&0.5f32.to_le_bytes()), [0.5]);
    }

    #[test]
    fn i420_grey_and_size_checks() {
        // 2×2 mid-grey.
        let data = [128, 128, 128, 128, 128, 128];
        let f = to_frame(CameraImage {
            width: 2,
            height: 2,
            format: ImageFormat::I420,
            rotation: 0,
            data: data.to_vec(),
        })
        .unwrap();
        assert_eq!(f.format, PixelFormat::Rgb);
        assert!(f.data.iter().all(|&b| b == 128));
        assert!(
            to_frame(CameraImage {
                width: 2,
                height: 2,
                format: ImageFormat::Rgba,
                rotation: 0,
                data: vec![0; 15],
            })
            .is_none()
        );
    }

    #[test]
    fn rotations() {
        // 2×1 RGB: red, green.
        let img = vec![255, 0, 0, 0, 255, 0];
        let (r90, w, h) = rotate(img.clone(), 2, 1, 3, 90);
        // Upright after 90° clockwise: 1 wide, 2 high; red on top.
        assert_eq!((w, h), (1, 2));
        assert_eq!(r90, vec![255, 0, 0, 0, 255, 0]);
        let (r270, _, _) = rotate(img.clone(), 2, 1, 3, 270);
        assert_eq!(r270, vec![0, 255, 0, 255, 0, 0]);
        let (r180, w, h) = rotate(img, 2, 1, 3, 180);
        assert_eq!((w, h), (2, 1));
        assert_eq!(r180, vec![0, 255, 0, 255, 0, 0]);
    }
}
