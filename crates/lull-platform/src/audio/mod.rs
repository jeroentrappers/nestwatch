// SPDX-License-Identifier: AGPL-3.0-or-later
//! Audio capture and playback (media.md §1, §4).
//!
//! Capture delivers interleaved samples at whatever rate and channel count
//! the device runs at; `lull_media::capture::CaptureChain` resamples to the
//! internal 16 kHz mono format. Playback either pulls 20 ms frames of 16 kHz
//! mono PCM (the backend converts) or hands the device-rate buffer to the
//! caller.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};

use lull_media::PcmFrame;

use crate::PlatformError;

#[cfg(any(feature = "cpal", not(target_os = "linux")))]
pub mod cpal_backend;
pub mod pacing;
#[cfg(target_os = "linux")]
pub mod pulse;
pub(crate) mod unavailable;

/// An audio endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDevice {
    /// Backend-specific id, passed back in a request.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Whether this is the system default for its direction.
    pub is_default: bool,
}

/// Sample representation of a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleType {
    /// 32-bit float in `-1.0..=1.0`.
    F32,
    /// 16-bit signed integer.
    I16,
}

/// The format a stream actually runs at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamFormat {
    /// Frames per second.
    pub sample_rate: u32,
    /// Interleaved channel count.
    pub channels: u16,
    /// Sample type delivered to callbacks.
    pub sample_type: SampleType,
}

/// A block of interleaved captured samples.
#[derive(Debug, Clone, Copy)]
pub enum Samples<'a> {
    /// Float samples.
    F32(&'a [f32]),
    /// Integer samples.
    I16(&'a [i16]),
}

impl Samples<'_> {
    /// Number of samples (all channels).
    pub fn len(&self) -> usize {
        match self {
            Self::F32(s) => s.len(),
            Self::I16(s) => s.len(),
        }
    }

    /// Whether the block is empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Feeds the block into a capture chain.
    pub fn push_into(&self, chain: &mut lull_media::capture::CaptureChain) {
        match self {
            Self::F32(s) => chain.push_f32(s),
            Self::I16(s) => chain.push_i16(s),
        }
    }
}

/// Capture callback, called on a backend thread with each block.
pub type CaptureCallback = Box<dyn FnMut(StreamFormat, Samples<'_>) + Send + 'static>;

/// What to capture.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureRequest {
    /// Device id from [`AudioInput::devices`]; `None` for the default.
    pub device: Option<String>,
    /// Preferred rate. Backends that resample (sound servers) honour it;
    /// others use the device rate.
    pub sample_rate: Option<u32>,
    /// Preferred channel count, same rules as `sample_rate`.
    pub channels: Option<u16>,
}

/// What to play.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaybackRequest {
    /// Device id from [`AudioOutput::devices`]; `None` for the default.
    pub device: Option<String>,
    /// Target output latency in milliseconds (a hint).
    pub latency_ms: u32,
}

impl Default for PlaybackRequest {
    fn default() -> Self {
        Self {
            device: None,
            latency_ms: 60,
        }
    }
}

/// Pulls one 20 ms frame of 16 kHz mono PCM; `false` means no audio.
pub type PcmPull = Box<dyn FnMut(&mut PcmFrame) -> bool + Send + 'static>;
/// Fills an interleaved `f32` buffer in the given stream format.
pub type DeviceFill = Box<dyn FnMut(StreamFormat, &mut [f32]) + Send + 'static>;

/// Where playback data comes from. Called on a backend thread.
pub enum PlaybackSource {
    /// Pulls one 20 ms frame of 16 kHz mono PCM. Returns `false` when no
    /// audio is available; the backend then plays silence and counts a
    /// starved frame.
    Pcm16k(PcmPull),
    /// Fills an interleaved `f32` buffer at the stream format.
    Device(DeviceFill),
}

impl fmt::Debug for PlaybackSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pcm16k(_) => "PlaybackSource::Pcm16k(..)",
            Self::Device(_) => "PlaybackSource::Device(..)",
        })
    }
}

/// Counters of a running stream.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamStats {
    /// Device or sound-server underruns (playback) reported by the backend.
    pub underruns: u64,
    /// Overruns or dropped capture blocks.
    pub overruns: u64,
    /// Frames for which a [`PlaybackSource::Pcm16k`] source had no audio.
    pub starved_frames: u64,
    /// The error that ended the stream, if it ended.
    pub error: Option<String>,
}

/// A running capture or playback stream. Dropping it stops the stream.
pub trait AudioStream: Send {
    /// The format the stream runs at.
    fn format(&self) -> StreamFormat;
    /// Current counters.
    fn stats(&self) -> StreamStats;
    /// Stops the stream (idempotent).
    fn stop(&mut self);
}

/// Microphone capture.
pub trait AudioInput: Send + Sync {
    /// Backend name for diagnostics.
    fn backend(&self) -> &'static str;
    /// Lists capture devices.
    fn devices(&self) -> Result<Vec<AudioDevice>, PlatformError>;
    /// Starts capturing; `callback` receives every block on a backend thread.
    fn start(
        &self,
        request: &CaptureRequest,
        callback: CaptureCallback,
    ) -> Result<Box<dyn AudioStream>, PlatformError>;
}

/// Speaker playback.
pub trait AudioOutput: Send + Sync {
    /// Backend name for diagnostics.
    fn backend(&self) -> &'static str;
    /// Lists playback devices.
    fn devices(&self) -> Result<Vec<AudioDevice>, PlatformError>;
    /// Starts playback pulling from `source`.
    fn start(
        &self,
        request: &PlaybackRequest,
        source: PlaybackSource,
    ) -> Result<Box<dyn AudioStream>, PlatformError>;
}

/// An owned captured block, as delivered by [`channel_callback`].
#[derive(Debug, Clone, PartialEq)]
pub struct CapturedBlock {
    /// Format of the samples.
    pub format: StreamFormat,
    /// Interleaved samples converted to `f32`.
    pub samples: Vec<f32>,
}

/// A capture callback that forwards blocks over a bounded channel instead.
/// When the receiver falls behind, blocks are dropped and counted in the
/// returned counter rather than blocking the audio thread.
pub fn channel_callback(
    capacity: usize,
) -> (CaptureCallback, Receiver<CapturedBlock>, Arc<AtomicU64>) {
    let (tx, rx): (SyncSender<CapturedBlock>, _) = sync_channel(capacity.max(1));
    let dropped = Arc::new(AtomicU64::new(0));
    let counter = Arc::clone(&dropped);
    let callback: CaptureCallback = Box::new(move |format, samples| {
        let samples = match samples {
            Samples::F32(s) => s.to_vec(),
            Samples::I16(s) => s.iter().map(|&v| f32::from(v) / 32768.0).collect(),
        };
        if let Err(TrySendError::Full(_)) = tx.try_send(CapturedBlock { format, samples }) {
            counter.fetch_add(1, Ordering::Relaxed);
        }
    });
    (callback, rx, dropped)
}

/// Shared counters between a stream handle and its backend thread.
#[derive(Debug, Default)]
pub(crate) struct SharedStats {
    pub(crate) underruns: AtomicU64,
    pub(crate) overruns: AtomicU64,
    pub(crate) starved: AtomicU64,
    pub(crate) error: Mutex<Option<String>>,
}

impl SharedStats {
    pub(crate) fn snapshot(&self) -> StreamStats {
        StreamStats {
            underruns: self.underruns.load(Ordering::Relaxed),
            overruns: self.overruns.load(Ordering::Relaxed),
            starved_frames: self.starved.load(Ordering::Relaxed),
            error: self.error.lock().ok().and_then(|e| e.clone()),
        }
    }

    pub(crate) fn fail(&self, message: impl Into<String>) {
        if let Ok(mut e) = self.error.lock()
            && e.is_none()
        {
            *e = Some(message.into());
        }
    }
}

/// Adapts a 16 kHz mono frame source to arbitrary device formats.
#[cfg_attr(not(any(feature = "cpal", not(target_os = "linux"))), allow(dead_code))]
pub(crate) struct Pcm16kAdapter {
    source: PcmPull,
    resampler: Option<lull_media::resample::Resampler>,
    pending: std::collections::VecDeque<f32>,
    scratch_in: Vec<f32>,
    scratch_out: Vec<f32>,
    stats: Arc<SharedStats>,
}

#[cfg_attr(not(any(feature = "cpal", not(target_os = "linux"))), allow(dead_code))]
impl Pcm16kAdapter {
    pub(crate) fn new(
        source: PcmPull,
        device_rate: u32,
        stats: Arc<SharedStats>,
    ) -> Result<Self, PlatformError> {
        let resampler = if device_rate == lull_media::SAMPLE_RATE {
            None
        } else {
            Some(
                lull_media::resample::Resampler::new(lull_media::SAMPLE_RATE, device_rate)
                    .map_err(|e| PlatformError::Backend(e.to_string()))?,
            )
        };
        Ok(Self {
            source,
            resampler,
            pending: std::collections::VecDeque::new(),
            scratch_in: Vec::with_capacity(lull_media::FRAME_SAMPLES),
            scratch_out: Vec::new(),
            stats,
        })
    }

    /// Fills `out` (interleaved, `channels` wide) with mono audio duplicated
    /// to every channel.
    pub(crate) fn fill(&mut self, out: &mut [f32], channels: usize) {
        let channels = channels.max(1);
        let needed = out.len() / channels;
        while self.pending.len() < needed {
            let mut frame = lull_media::SILENT_FRAME;
            if !(self.source)(&mut frame) {
                frame = lull_media::SILENT_FRAME;
                self.stats.starved.fetch_add(1, Ordering::Relaxed);
            }
            self.scratch_in.clear();
            self.scratch_in
                .extend(frame.iter().map(|&s| f32::from(s) / 32768.0));
            match &mut self.resampler {
                Some(r) => {
                    self.scratch_out.clear();
                    r.process(&self.scratch_in, &mut self.scratch_out);
                    self.pending.extend(self.scratch_out.iter().copied());
                }
                None => self.pending.extend(self.scratch_in.iter().copied()),
            }
        }
        for frame in out.chunks_mut(channels) {
            let v = self.pending.pop_front().unwrap_or(0.0);
            frame.fill(v);
        }
    }
}

/// Converts `f32` samples to `i16` with clipping.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn f32_to_i16(v: f32) -> i16 {
    (v.clamp(-1.0, 1.0) * 32767.0).round() as i16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adapter_duplicates_and_counts_starvation() {
        let stats = Arc::new(SharedStats::default());
        let mut calls = 0;
        let source = Box::new(move |f: &mut PcmFrame| {
            calls += 1;
            if calls == 1 {
                f.fill(16384);
                true
            } else {
                false
            }
        });
        let mut a = Pcm16kAdapter::new(source, 16_000, Arc::clone(&stats)).unwrap();
        let mut out = vec![1.0f32; 640 * 2];
        a.fill(&mut out, 2);
        assert!(out[..640].iter().all(|&v| (v - 0.5).abs() < 1e-6));
        assert!(out[640..].iter().all(|&v| v == 0.0));
        assert_eq!(stats.snapshot().starved_frames, 1);
    }

    #[test]
    fn adapter_resamples_to_device_rate() {
        let stats = Arc::new(SharedStats::default());
        let source = Box::new(|f: &mut PcmFrame| {
            f.fill(8000);
            true
        });
        let mut a = Pcm16kAdapter::new(source, 48_000, stats).unwrap();
        // 100 ms at 48 kHz stereo.
        let mut out = vec![0.0f32; 4800 * 2];
        a.fill(&mut out, 2);
        let tail = &out[2000..];
        assert!(tail.iter().all(|&v| (v - 8000.0 / 32768.0).abs() < 0.01));
    }

    #[test]
    fn channel_callback_drops_when_full() {
        let (mut cb, rx, dropped) = channel_callback(1);
        let fmt = StreamFormat {
            sample_rate: 16_000,
            channels: 1,
            sample_type: SampleType::I16,
        };
        cb(fmt, Samples::I16(&[16384, -16384]));
        cb(fmt, Samples::I16(&[0]));
        assert_eq!(dropped.load(Ordering::Relaxed), 1);
        let block = rx.recv().unwrap();
        assert_eq!(block.samples, vec![0.5, -0.5]);
    }

    #[test]
    fn f32_to_i16_clips() {
        assert_eq!(f32_to_i16(2.0), 32767);
        assert_eq!(f32_to_i16(-2.0), -32767);
        assert_eq!(f32_to_i16(0.0), 0);
    }
}
