// SPDX-License-Identifier: AGPL-3.0-or-later
//! Audio and video processing for Lull (`specs/media.md`).
//!
//! Everything here is pure and sans-I/O: no devices, threads or async. Time
//! is passed in as milliseconds or derived from frame counts.
//!
//! - [`capture`]: resampling to 16 kHz mono, gain / AGC and level (§1).
//! - [`codec`]: 20 ms audio codecs, codec 2 IMA ADPCM and optional Opus (§2).
//! - [`gate`]: shared ring buffer and per-listener sound activation gate (§3).
//! - [`jitter`]: per-station jitter buffer with concealment (§4.1).
//! - [`mixer`]: multi-station mixer with soft limiter (§4.2).
//! - [`video`]: JPEG helpers, scaling, night filter, adaptation (§5).

pub mod capture;
pub mod codec;
mod error;
mod frame;

pub mod level;

pub mod resample;

pub use error::MediaError;
pub use frame::{AudioFrame, CodecId, FrameFlags};

/// Internal audio sample rate in Hz (§1).
pub const SAMPLE_RATE: u32 = 16_000;
/// Duration of one audio frame in milliseconds (§1).
pub const FRAME_MS: u32 = 20;
/// Samples per 20 ms frame at 16 kHz (§1).
pub const FRAME_SAMPLES: usize = 320;
/// Clock rate of AUDIO `timestamp` fields (protocol.md §6.1).
pub const TIMESTAMP_RATE: u32 = 48_000;
/// Timestamp increment per 20 ms frame.
pub const FRAME_TICKS: u32 = TIMESTAMP_RATE / 1000 * FRAME_MS;

/// One 20 ms frame of 16 kHz mono PCM.
pub type PcmFrame = [i16; FRAME_SAMPLES];
/// A frame of digital silence.
pub const SILENT_FRAME: PcmFrame = [0; FRAME_SAMPLES];
