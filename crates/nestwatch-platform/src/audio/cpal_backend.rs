// SPDX-License-Identifier: AGPL-3.0-or-later
//! Audio through `cpal` (CoreAudio, WASAPI, ALSA).
//!
//! cpal streams are not `Send` on every platform, so each stream lives on its
//! own thread; the returned handle only signals that thread to stop.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::thread::JoinHandle;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};

use super::{
    AudioDevice, AudioInput, AudioOutput, AudioStream, CaptureCallback, CaptureRequest,
    Pcm16kAdapter, PlaybackRequest, PlaybackSource, SampleType, Samples, SharedStats, StreamFormat,
    StreamStats,
};
use crate::PlatformError;

/// Fills an interleaved device buffer.
type FillFn = Box<dyn FnMut(&mut [f32]) + Send>;

/// The cpal backend on the platform's default host.
#[derive(Debug, Clone, Default)]
pub struct CpalAudio {
    _private: (),
}

impl CpalAudio {
    /// Creates the backend; fails when the default host has no devices at all.
    pub fn detect() -> Result<Self, PlatformError> {
        let host = cpal::default_host();
        let any = host.default_input_device().is_some() || host.default_output_device().is_some();
        if any {
            Ok(Self { _private: () })
        } else {
            Err(PlatformError::Unavailable(
                "cpal found no audio devices".into(),
            ))
        }
    }
}

fn list(input: bool) -> Result<Vec<AudioDevice>, PlatformError> {
    let host = cpal::default_host();
    let default = if input {
        host.default_input_device()
    } else {
        host.default_output_device()
    }
    .and_then(|d| d.name().ok());
    let devices = if input {
        host.input_devices().map(|d| d.collect::<Vec<_>>())
    } else {
        host.output_devices().map(|d| d.collect::<Vec<_>>())
    }
    .map_err(|e| PlatformError::Unavailable(e.to_string()))?;
    Ok(devices
        .into_iter()
        .filter_map(|d| d.name().ok())
        .map(|name| AudioDevice {
            is_default: default.as_deref() == Some(name.as_str()),
            id: name.clone(),
            name,
        })
        .collect())
}

fn find_device(input: bool, id: Option<&str>) -> Result<cpal::Device, PlatformError> {
    let host = cpal::default_host();
    match id {
        None => if input {
            host.default_input_device()
        } else {
            host.default_output_device()
        }
        .ok_or_else(|| PlatformError::Unavailable("no default audio device".into())),
        Some(id) => {
            let mut devices = if input {
                host.input_devices().map(|d| d.collect::<Vec<_>>())
            } else {
                host.output_devices().map(|d| d.collect::<Vec<_>>())
            }
            .map_err(|e| PlatformError::Unavailable(e.to_string()))?;
            let pos = devices
                .iter()
                .position(|d| d.name().is_ok_and(|n| n == id))
                .ok_or_else(|| PlatformError::NoSuchDevice(id.to_owned()))?;
            Ok(devices.swap_remove(pos))
        }
    }
}

fn build_error(e: cpal::BuildStreamError) -> PlatformError {
    match e {
        cpal::BuildStreamError::DeviceNotAvailable => {
            PlatformError::Unavailable("audio device not available".into())
        }
        other => PlatformError::from_message(other.to_string()),
    }
}

/// Runs `build` on a dedicated thread that owns the stream until stopped.
fn spawn_stream<F>(name: &str, build: F) -> Result<(StreamFormat, CpalStream), PlatformError>
where
    F: FnOnce(Arc<SharedStats>) -> Result<(cpal::Stream, StreamFormat), PlatformError>
        + Send
        + 'static,
{
    let stats = Arc::new(SharedStats::default());
    let (ready_tx, ready_rx) = mpsc::channel();
    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let thread_stats = Arc::clone(&stats);
    let thread = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let stream = match build(thread_stats) {
                Ok((stream, format)) => match stream.play() {
                    Ok(()) => {
                        let _ = ready_tx.send(Ok(format));
                        stream
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(PlatformError::from_message(e.to_string())));
                        return;
                    }
                },
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                    return;
                }
            };
            // Blocks until the handle is stopped or dropped.
            let _ = stop_rx.recv();
            drop(stream);
        })
        .map_err(PlatformError::Io)?;
    let format = ready_rx
        .recv()
        .map_err(|_| PlatformError::Backend("audio thread exited".into()))??;
    Ok((
        format,
        CpalStream {
            stop: Some(stop_tx),
            thread: Some(thread),
            stats,
            format,
        },
    ))
}

fn stream_format(cfg: &cpal::StreamConfig, sample_type: SampleType) -> StreamFormat {
    StreamFormat {
        sample_rate: cfg.sample_rate.0,
        channels: cfg.channels,
        sample_type,
    }
}

impl AudioInput for CpalAudio {
    fn backend(&self) -> &'static str {
        "cpal"
    }

    fn devices(&self) -> Result<Vec<AudioDevice>, PlatformError> {
        list(true)
    }

    fn start(
        &self,
        request: &CaptureRequest,
        mut callback: CaptureCallback,
    ) -> Result<Box<dyn AudioStream>, PlatformError> {
        let device_id = request.device.clone();
        let (_, stream) = spawn_stream("nestwatch-cpal-in", move |stats| {
            let device = find_device(true, device_id.as_deref())?;
            let supported = device
                .default_input_config()
                .map_err(|e| PlatformError::Unavailable(e.to_string()))?;
            let cfg = supported.config();
            let err_stats = Arc::clone(&stats);
            let on_error = move |e: cpal::StreamError| match e {
                cpal::StreamError::DeviceNotAvailable => err_stats.fail(e.to_string()),
                cpal::StreamError::BackendSpecific { .. } => {
                    err_stats.overruns.fetch_add(1, Ordering::Relaxed);
                }
            };
            let (stream, sample_type) = match supported.sample_format() {
                SampleFormat::F32 => {
                    let fmt = stream_format(&cfg, SampleType::F32);
                    let s = device.build_input_stream(
                        &cfg,
                        move |data: &[f32], _: &_| callback(fmt, Samples::F32(data)),
                        on_error,
                        None,
                    );
                    (s, SampleType::F32)
                }
                SampleFormat::I16 => {
                    let fmt = stream_format(&cfg, SampleType::I16);
                    let s = device.build_input_stream(
                        &cfg,
                        move |data: &[i16], _: &_| callback(fmt, Samples::I16(data)),
                        on_error,
                        None,
                    );
                    (s, SampleType::I16)
                }
                SampleFormat::I8 => (
                    input_as_f32::<i8>(&device, &cfg, callback, on_error),
                    SampleType::F32,
                ),
                SampleFormat::I32 => (
                    input_as_f32::<i32>(&device, &cfg, callback, on_error),
                    SampleType::F32,
                ),
                SampleFormat::U8 => (
                    input_as_f32::<u8>(&device, &cfg, callback, on_error),
                    SampleType::F32,
                ),
                SampleFormat::U16 => (
                    input_as_f32::<u16>(&device, &cfg, callback, on_error),
                    SampleType::F32,
                ),
                SampleFormat::F64 => (
                    input_as_f32::<f64>(&device, &cfg, callback, on_error),
                    SampleType::F32,
                ),
                _ => return Err(PlatformError::Unsupported("capture sample format")),
            };
            Ok((
                stream.map_err(build_error)?,
                stream_format(&cfg, sample_type),
            ))
        })?;
        Ok(Box::new(stream))
    }
}

fn input_as_f32<T>(
    device: &cpal::Device,
    cfg: &cpal::StreamConfig,
    mut callback: CaptureCallback,
    on_error: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let fmt = stream_format(cfg, SampleType::F32);
    let mut scratch = Vec::new();
    device.build_input_stream(
        cfg,
        move |data: &[T], _: &_| {
            scratch.clear();
            scratch.extend(data.iter().map(|&s| s.to_sample::<f32>()));
            callback(fmt, Samples::F32(&scratch));
        },
        on_error,
        None,
    )
}

impl AudioOutput for CpalAudio {
    fn backend(&self) -> &'static str {
        "cpal"
    }

    fn devices(&self) -> Result<Vec<AudioDevice>, PlatformError> {
        list(false)
    }

    fn start(
        &self,
        request: &PlaybackRequest,
        source: PlaybackSource,
    ) -> Result<Box<dyn AudioStream>, PlatformError> {
        let device_id = request.device.clone();
        let (_, stream) = spawn_stream("nestwatch-cpal-out", move |stats| {
            let device = find_device(false, device_id.as_deref())?;
            let supported = device
                .default_output_config()
                .map_err(|e| PlatformError::Unavailable(e.to_string()))?;
            let cfg = supported.config();
            let fmt = stream_format(&cfg, SampleType::F32);
            let channels = usize::from(cfg.channels);
            let mut fill: FillFn = match source {
                PlaybackSource::Pcm16k(pull) => {
                    let mut adapter =
                        Pcm16kAdapter::new(pull, fmt.sample_rate, Arc::clone(&stats))?;
                    Box::new(move |out| adapter.fill(out, channels))
                }
                PlaybackSource::Device(mut f) => Box::new(move |out| f(fmt, out)),
            };
            let err_stats = Arc::clone(&stats);
            let on_error = move |e: cpal::StreamError| match e {
                cpal::StreamError::DeviceNotAvailable => err_stats.fail(e.to_string()),
                cpal::StreamError::BackendSpecific { .. } => {
                    err_stats.underruns.fetch_add(1, Ordering::Relaxed);
                }
            };
            let stream = match supported.sample_format() {
                SampleFormat::F32 => device.build_output_stream(
                    &cfg,
                    move |data: &mut [f32], _: &_| {
                        data.fill(0.0);
                        fill(data);
                    },
                    on_error,
                    None,
                ),
                SampleFormat::I16 => output_from_f32::<i16>(&device, &cfg, fill, on_error),
                SampleFormat::I32 => output_from_f32::<i32>(&device, &cfg, fill, on_error),
                SampleFormat::U16 => output_from_f32::<u16>(&device, &cfg, fill, on_error),
                SampleFormat::U8 => output_from_f32::<u8>(&device, &cfg, fill, on_error),
                SampleFormat::F64 => output_from_f32::<f64>(&device, &cfg, fill, on_error),
                _ => return Err(PlatformError::Unsupported("playback sample format")),
            };
            Ok((stream.map_err(build_error)?, fmt))
        })?;
        Ok(Box::new(stream))
    }
}

fn output_from_f32<T>(
    device: &cpal::Device,
    cfg: &cpal::StreamConfig,
    mut fill: FillFn,
    on_error: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, cpal::BuildStreamError>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let mut scratch = Vec::new();
    device.build_output_stream(
        cfg,
        move |data: &mut [T], _: &_| {
            scratch.clear();
            scratch.resize(data.len(), 0.0);
            fill(&mut scratch);
            for (d, &s) in data.iter_mut().zip(&scratch) {
                *d = T::from_sample(s);
            }
        },
        on_error,
        None,
    )
}

/// Handle of a cpal stream living on its own thread.
struct CpalStream {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<JoinHandle<()>>,
    stats: Arc<SharedStats>,
    format: StreamFormat,
}

impl AudioStream for CpalStream {
    fn format(&self) -> StreamFormat {
        self.format
    }

    fn stats(&self) -> StreamStats {
        self.stats.snapshot()
    }

    fn stop(&mut self) {
        drop(self.stop.take());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for CpalStream {
    fn drop(&mut self) {
        self.stop();
    }
}
