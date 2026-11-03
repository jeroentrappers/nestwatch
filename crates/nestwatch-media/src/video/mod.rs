// SPDX-License-Identifier: AGPL-3.0-or-later
//! Video helpers (media.md §5): JPEG, scaling, night filter, adaptation.

pub mod adapt;
pub mod jpeg;
pub mod night;
pub mod scale;

pub use adapt::VideoAdaptation;
pub use jpeg::{I420Image, RgbaImage, decode_to_rgba, encode_i420, encode_rgb, encode_rgba};
pub use night::{night_filter, night_filter_i420};
pub use scale::{fit_within, scale_image};

/// Video codec 1: baseline JPEG (protocol.md §6.2).
pub const VIDEO_CODEC_JPEG: u8 = 1;
