// SPDX-License-Identifier: AGPL-3.0-or-later
//! Cameras through `nokhwa` (V4L2, AVFoundation, Media Foundation).
//!
//! nokhwa's own MJPEG decoding needs libjpeg; it is left off and frames are
//! converted here instead (MJPEG through lull-media, YUYV / NV12 / grey to
//! I420, raw RGB / BGR to RGB). Each stream owns its camera on a dedicated
//! thread.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use nokhwa::pixel_format::RgbFormat;
use nokhwa::utils::{
    ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType,
};

use super::{
    Camera, CameraInfo, CameraMode, CameraRequest, CameraStream, Facing, FpsLimiter, FrameCallback,
    PixelData, VideoFrame, choose_mode, gray_to_i420, nv12_to_i420, pick_camera, yuyv_to_i420,
};
use crate::PlatformError;

const OPEN_TIMEOUT: Duration = Duration::from_secs(10);
#[cfg(target_os = "macos")]
const PERMISSION_TIMEOUT: Duration = Duration::from_secs(60);

/// The nokhwa camera backend.
#[derive(Debug, Clone, Default)]
pub struct NokhwaCamera {
    _private: (),
}

impl NokhwaCamera {
    /// Creates the backend.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Asks for camera permission where the OS requires it (macOS). Blocks
/// until the user answers.
fn ensure_permission() -> Result<(), PlatformError> {
    #[cfg(target_os = "macos")]
    {
        if nokhwa::nokhwa_check() {
            return Ok(());
        }
        let (tx, rx) = mpsc::channel();
        nokhwa::nokhwa_initialize(move |granted| {
            let _ = tx.send(granted);
        });
        match rx.recv_timeout(PERMISSION_TIMEOUT) {
            Ok(true) => Ok(()),
            _ => Err(PlatformError::PermissionDenied(
                "camera access was not granted".into(),
            )),
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
}

fn map_error(e: nokhwa::NokhwaError) -> PlatformError {
    PlatformError::from_message(e.to_string())
}

fn query() -> Result<Vec<nokhwa::utils::CameraInfo>, PlatformError> {
    nokhwa::query(ApiBackend::Auto).map_err(map_error)
}

/// The initial format request when opening a camera; the real mode is
/// chosen afterwards from the camera's list.
fn any_colour_format() -> RequestedFormat<'static> {
    RequestedFormat::new::<RgbFormat>(RequestedFormatType::AbsoluteHighestFrameRate)
}

/// On Linux, V4L2 also lists metadata nodes and infrared sensors; only nodes
/// that open with a colour format are cameras. Opening does not stream, so
/// the camera indicator stays off. Elsewhere every listed camera is usable.
fn usable(index: &CameraIndex) -> bool {
    #[cfg(target_os = "linux")]
    {
        nokhwa::Camera::new(index.clone(), any_colour_format()).is_ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = index;
        true
    }
}

/// Converts one raw frame to [`PixelData`].
fn convert(
    format: FrameFormat,
    w: u32,
    h: u32,
    raw: &[u8],
) -> Result<(u32, u32, PixelData), String> {
    let px = (w as usize) * (h as usize);
    let data = match format {
        FrameFormat::MJPEG => {
            let img = lull_media::video::decode_to_rgba(raw).map_err(|e| e.to_string())?;
            return Ok((img.width, img.height, PixelData::Rgba(img.data)));
        }
        FrameFormat::YUYV => yuyv_to_i420(raw, w, h),
        FrameFormat::NV12 => nv12_to_i420(raw, w, h),
        FrameFormat::GRAY => gray_to_i420(raw, w, h),
        FrameFormat::RAWRGB => {
            (raw.len() >= px * 3).then(|| PixelData::Rgb(raw[..px * 3].to_vec()))
        }
        FrameFormat::RAWBGR => (raw.len() >= px * 3).then(|| {
            PixelData::Rgb(
                raw[..px * 3]
                    .chunks_exact(3)
                    .flat_map(|p| [p[2], p[1], p[0]])
                    .collect(),
            )
        }),
    };
    data.map(|d| (w, h, d))
        .ok_or_else(|| format!("short {format} frame ({} bytes for {w}×{h})", raw.len()))
}

/// Cheaper-to-convert formats rank higher.
fn rank(format: FrameFormat) -> u8 {
    match format {
        FrameFormat::YUYV | FrameFormat::NV12 => 3,
        FrameFormat::MJPEG => 2,
        FrameFormat::RAWRGB | FrameFormat::RAWBGR => 1,
        FrameFormat::GRAY => 0,
    }
}

impl Camera for NokhwaCamera {
    fn backend(&self) -> &'static str {
        "nokhwa"
    }

    fn cameras(&self) -> Result<Vec<CameraInfo>, PlatformError> {
        ensure_permission()?;
        let mut found = query()?;
        found.sort_by_key(|c| match c.index() {
            CameraIndex::Index(i) => (0, *i, String::new()),
            CameraIndex::String(s) => (1, 0, s.clone()),
        });
        Ok(found
            .into_iter()
            .filter(|c| usable(c.index()))
            .map(|c| {
                let name = c.human_name();
                CameraInfo {
                    id: c.index().to_string(),
                    facing: Facing::guess_from_name(&name),
                    name,
                }
            })
            .collect())
    }

    fn start(
        &self,
        request: &CameraRequest,
        mut callback: FrameCallback,
    ) -> Result<Box<dyn CameraStream>, PlatformError> {
        let cameras = self.cameras()?;
        let id = pick_camera(&cameras, request)?.id.clone();
        let index = query()?
            .into_iter()
            .find(|c| c.index().to_string() == id)
            .map(|c| c.index().clone())
            .unwrap_or(CameraIndex::String(id));
        let request = request.clone();
        let stopping = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stopping);
        let (ready_tx, ready_rx) = mpsc::channel::<Result<CameraMode, PlatformError>>();

        let thread = std::thread::Builder::new()
            .name("lull-nokhwa".into())
            .spawn(move || {
                let open = || -> Result<(nokhwa::Camera, CameraFormat), PlatformError> {
                    let mut cam =
                        nokhwa::Camera::new(index, any_colour_format()).map_err(map_error)?;
                    let formats = cam.compatible_camera_formats().map_err(map_error)?;
                    let chosen = choose_mode(
                        &formats,
                        |f| CameraMode {
                            width: f.width(),
                            height: f.height(),
                            fps: f.frame_rate(),
                        },
                        &request,
                        |f| rank(f.format()),
                    )
                    .ok_or_else(|| PlatformError::Unavailable("camera has no modes".into()))?;
                    cam.set_camera_requset(RequestedFormat::new::<RgbFormat>(
                        RequestedFormatType::Exact(chosen),
                    ))
                    .map_err(map_error)?;
                    cam.open_stream().map_err(map_error)?;
                    let actual = cam.camera_format();
                    Ok((cam, actual))
                };
                let (mut cam, format) = match open() {
                    Ok(v) => v,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let _ = ready_tx.send(Ok(CameraMode {
                    width: format.width(),
                    height: format.height(),
                    fps: format.frame_rate(),
                }));
                let started = Instant::now();
                let mut limiter = FpsLimiter::new(request.max_fps);
                while !thread_stop.load(Ordering::Relaxed) {
                    let buffer = match cam.frame() {
                        Ok(b) => b,
                        Err(e) => {
                            if !thread_stop.load(Ordering::Relaxed) {
                                callback(Err(map_error(e)));
                            }
                            break;
                        }
                    };
                    let ts = started.elapsed();
                    if !limiter.accept(ts) {
                        continue;
                    }
                    let res = buffer.resolution();
                    match convert(
                        buffer.source_frame_format(),
                        res.width(),
                        res.height(),
                        buffer.buffer(),
                    ) {
                        Ok((width, height, data)) => callback(Ok(VideoFrame {
                            width,
                            height,
                            timestamp: ts,
                            data,
                        })),
                        Err(e) => log::debug!("dropping camera frame: {e}"),
                    }
                }
                let _ = cam.stop_stream();
            })
            .map_err(PlatformError::Io)?;

        let mut stream = NokhwaStream {
            thread: Some(thread),
            stopping,
            mode: CameraMode {
                width: 0,
                height: 0,
                fps: 0,
            },
        };
        match ready_rx.recv_timeout(OPEN_TIMEOUT) {
            Ok(Ok(mode)) => {
                stream.mode = mode;
                Ok(Box::new(stream))
            }
            Ok(Err(e)) => {
                stream.stop();
                Err(e)
            }
            Err(_) => {
                // The thread may still be blocked opening the device; it
                // exits on its own once the open returns.
                stream.stopping.store(true, Ordering::Relaxed);
                stream.thread.take();
                Err(PlatformError::Unavailable(
                    "camera did not open in time".into(),
                ))
            }
        }
    }
}

struct NokhwaStream {
    thread: Option<JoinHandle<()>>,
    stopping: Arc<AtomicBool>,
    mode: CameraMode,
}

impl CameraStream for NokhwaStream {
    fn mode(&self) -> CameraMode {
        self.mode
    }

    fn stop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for NokhwaStream {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_raw_formats() {
        let (_, _, d) = convert(FrameFormat::RAWBGR, 1, 1, &[1, 2, 3]).unwrap();
        assert_eq!(d, PixelData::Rgb(vec![3, 2, 1]));
        assert!(convert(FrameFormat::RAWRGB, 2, 2, &[0; 3]).is_err());
        let (w, h, d) = convert(FrameFormat::YUYV, 2, 1, &[1, 2, 3, 4]).unwrap();
        assert_eq!((w, h), (2, 1));
        assert!(matches!(d, PixelData::I420 { .. }));
        let jpeg = lull_media::video::encode_rgb(&[90; 8 * 8 * 3], 8, 8, 80).unwrap();
        let (w, h, d) = convert(FrameFormat::MJPEG, 0, 0, &jpeg).unwrap();
        assert_eq!((w, h), (8, 8));
        assert!(matches!(d, PixelData::Rgba(_)));
    }
}
