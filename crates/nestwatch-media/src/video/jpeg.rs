// SPDX-License-Identifier: AGPL-3.0-or-later
//! Baseline JPEG, YCbCr 4:2:0 (media.md §5).

use jpeg_encoder::{ColorType, Encoder, SamplingFactor};
use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::DecoderOptions;

use crate::MediaError;

/// Largest complete video frame (protocol.md §6.2).
pub const MAX_FRAME_BYTES: usize = 2 * 1024 * 1024;

/// A decoded image, 4 bytes per pixel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Row-major RGBA bytes.
    pub data: Vec<u8>,
}

/// A planar YUV 4:2:0 image with tight strides. Samples are full-range
/// (JFIF) BT.601; chroma planes are `ceil(w/2) × ceil(h/2)`.
#[derive(Debug, Clone, Copy)]
pub struct I420Image<'a> {
    /// Width in pixels.
    pub width: u16,
    /// Height in pixels.
    pub height: u16,
    /// Luma plane.
    pub y: &'a [u8],
    /// Cb plane.
    pub u: &'a [u8],
    /// Cr plane.
    pub v: &'a [u8],
}

fn encode(
    data: &[u8],
    w: u16,
    h: u16,
    color: ColorType,
    quality: u8,
) -> Result<Vec<u8>, MediaError> {
    if w == 0 || h == 0 {
        return Err(MediaError::InvalidParameter("image size"));
    }
    let mut out = Vec::new();
    let mut enc = Encoder::new(&mut out, quality.clamp(1, 100));
    enc.set_sampling_factor(SamplingFactor::F_2_2);
    enc.encode(data, w, h, color)
        .map_err(|e| MediaError::Jpeg(e.to_string()))?;
    Ok(out)
}

/// Encodes tightly packed RGB as baseline JPEG 4:2:0.
pub fn encode_rgb(rgb: &[u8], width: u16, height: u16, quality: u8) -> Result<Vec<u8>, MediaError> {
    encode(rgb, width, height, ColorType::Rgb, quality)
}

/// Encodes tightly packed RGBA (alpha ignored) as baseline JPEG 4:2:0.
pub fn encode_rgba(
    rgba: &[u8],
    width: u16,
    height: u16,
    quality: u8,
) -> Result<Vec<u8>, MediaError> {
    encode(rgba, width, height, ColorType::Rgba, quality)
}

/// Encodes an I420 image as baseline JPEG 4:2:0.
pub fn encode_i420(img: &I420Image<'_>, quality: u8) -> Result<Vec<u8>, MediaError> {
    let (w, h) = (usize::from(img.width), usize::from(img.height));
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    if img.y.len() < w * h || img.u.len() < cw * ch || img.v.len() < cw * ch {
        return Err(MediaError::InvalidParameter("i420 plane size"));
    }
    let mut ycc = Vec::with_capacity(w * h * 3);
    for row in 0..h {
        for col in 0..w {
            let c = (row / 2) * cw + col / 2;
            ycc.extend_from_slice(&[img.y[row * w + col], img.u[c], img.v[c]]);
        }
    }
    encode(&ycc, img.width, img.height, ColorType::Ycbcr, quality)
}

/// Decodes a JPEG to RGBA.
pub fn decode_to_rgba(jpeg: &[u8]) -> Result<RgbaImage, MediaError> {
    if jpeg.len() > MAX_FRAME_BYTES {
        return Err(MediaError::InvalidParameter("jpeg larger than 2 MiB"));
    }
    let options = DecoderOptions::default()
        .jpeg_set_out_colorspace(ColorSpace::RGBA)
        .set_max_width(8192)
        .set_max_height(8192);
    let mut dec = JpegDecoder::new_with_options(ZCursor::new(jpeg), options);
    let data = dec
        .decode()
        .map_err(|e| MediaError::Jpeg(format!("{e:?}")))?;
    let info = dec
        .info()
        .ok_or_else(|| MediaError::Jpeg("missing header".into()))?;
    let (width, height) = (u32::from(info.width), u32::from(info.height));
    if data.len() != width as usize * height as usize * 4 {
        return Err(MediaError::Jpeg("unexpected output size".into()));
    }
    Ok(RgbaImage {
        width,
        height,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: usize, h: usize) -> Vec<u8> {
        let mut v = Vec::with_capacity(w * h * 3);
        for y in 0..h {
            for x in 0..w {
                v.extend_from_slice(&[(x * 255 / w) as u8, (y * 255 / h) as u8, 128]);
            }
        }
        v
    }

    fn mean_abs_diff_rgb_rgba(rgb: &[u8], rgba: &[u8]) -> f64 {
        let mut sum = 0u64;
        for (p, q) in rgb.chunks(3).zip(rgba.chunks(4)) {
            for c in 0..3 {
                sum += u64::from(p[c].abs_diff(q[c]));
            }
            assert_eq!(q[3], 255);
        }
        sum as f64 / rgb.len() as f64
    }

    #[test]
    fn media_5_jpeg_round_trip_rgb() {
        let (w, h) = (64, 48);
        let img = gradient(w, h);
        let jpg = encode_rgb(&img, w as u16, h as u16, 70).unwrap();
        assert_eq!(&jpg[..2], &[0xFF, 0xD8]);
        // Baseline: SOF0 present, 3 components, luma sampled 2x2 (4:2:0).
        let sof = jpg
            .windows(2)
            .position(|m| m == [0xFF, 0xC0])
            .expect("SOF0");
        assert_eq!(jpg[sof + 9], 3);
        assert_eq!(jpg[sof + 11], 0x22);
        let dec = decode_to_rgba(&jpg).unwrap();
        assert_eq!((dec.width, dec.height), (64, 48));
        assert!(mean_abs_diff_rgb_rgba(&img, &dec.data) < 4.0);
    }

    #[test]
    fn media_5_jpeg_quality_affects_size() {
        let mut img = gradient(64, 64);
        for (i, b) in img.iter_mut().enumerate() {
            *b = b.wrapping_add((i * 37 % 23) as u8);
        }
        let hi = encode_rgb(&img, 64, 64, 70).unwrap();
        let lo = encode_rgb(&img, 64, 64, 30).unwrap();
        assert!(lo.len() < hi.len());
    }

    #[test]
    fn media_5_jpeg_i420_and_rgba_input() {
        let (w, h) = (33usize, 17usize);
        let y = vec![200u8; w * h];
        let u = vec![128u8; 17 * 9];
        let v = vec![128u8; 17 * 9];
        let img = I420Image {
            width: w as u16,
            height: h as u16,
            y: &y,
            u: &u,
            v: &v,
        };
        let dec = decode_to_rgba(&encode_i420(&img, 70).unwrap()).unwrap();
        assert_eq!((dec.width, dec.height), (33, 17));
        assert!(
            dec.data
                .chunks(4)
                .all(|p| p[0].abs_diff(200) <= 3 && p[1].abs_diff(200) <= 3)
        );
        let rgba = vec![10u8; 8 * 8 * 4];
        assert!(encode_rgba(&rgba, 8, 8, 50).is_ok());
        assert!(encode_i420(&I420Image { y: &y[..10], ..img }, 70).is_err());
    }

    #[test]
    fn media_5_jpeg_rejects_garbage() {
        assert!(decode_to_rgba(&[0, 1, 2, 3]).is_err());
        assert!(encode_rgb(&[], 0, 0, 70).is_err());
    }
}
