// SPDX-License-Identifier: AGPL-3.0-or-later
//! Camera capture (media.md §5, protocol.md §7.7).
//!
//! Backends deliver decoded frames; the station scales them with
//! `lull_media::video::fit_within` / `scale_image` and encodes JPEG. A
//! backend picks the camera mode closest to the request but may deliver a
//! larger frame when the camera has no smaller mode. Errors map to the
//! protocol `video-state` through [`PlatformError::video_state`].

use std::fmt;
use std::time::Duration;

use crate::PlatformError;

#[cfg(target_os = "linux")]
pub mod ffmpeg;
pub mod mjpeg;
#[cfg(feature = "camera")]
pub mod nokhwa_backend;
pub(crate) mod unavailable;

/// Which way a camera points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Facing {
    /// Towards the user (built-in laptop or phone front camera).
    Front,
    /// Away from the user.
    Back,
    /// A separate camera (USB, network).
    External,
    /// Not known.
    Unknown,
}

impl Facing {
    /// Guesses the facing from a device name. Built-in desktop cameras face
    /// the user; everything else is unknown.
    pub fn guess_from_name(name: &str) -> Self {
        let n = name.to_ascii_lowercase();
        if n.contains("back") || n.contains("rear") {
            Self::Back
        } else if n.contains("front")
            || n.contains("integrated")
            || n.contains("built-in")
            || n.contains("facetime")
        {
            Self::Front
        } else {
            Self::Unknown
        }
    }

    /// The protocol `video-options camera` value this camera satisfies, if
    /// any (protocol.md §7.7).
    pub fn protocol_name(self) -> Option<&'static str> {
        match self {
            Self::Front => Some("front"),
            Self::Back | Self::External => Some("back"),
            Self::Unknown => None,
        }
    }
}

/// A camera found on the system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraInfo {
    /// Backend-specific id, passed back in a request.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// Facing, where known.
    pub facing: Facing,
}

/// What to capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraRequest {
    /// Camera id; `None` picks by `facing`, then the first camera.
    pub camera: Option<String>,
    /// Preferred facing when `camera` is `None`.
    pub facing: Option<Facing>,
    /// Largest useful width.
    pub max_width: u32,
    /// Largest useful height.
    pub max_height: u32,
    /// Highest useful frame rate; faster frames are dropped.
    pub max_fps: u32,
}

impl Default for CameraRequest {
    /// The protocol default request: 640×480 at 10 fps (media.md §5).
    fn default() -> Self {
        Self {
            camera: None,
            facing: None,
            max_width: 640,
            max_height: 480,
            max_fps: 10,
        }
    }
}

/// Pixels of a frame.
#[derive(Clone, PartialEq, Eq)]
pub enum PixelData {
    /// Packed RGB, 3 bytes per pixel.
    Rgb(Vec<u8>),
    /// Packed RGBA, 4 bytes per pixel.
    Rgba(Vec<u8>),
    /// Planar 4:2:0, full-range BT.601; chroma planes `ceil(w/2) × ceil(h/2)`.
    I420 {
        /// Luma.
        y: Vec<u8>,
        /// Cb.
        u: Vec<u8>,
        /// Cr.
        v: Vec<u8>,
    },
}

impl fmt::Debug for PixelData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never dump image bytes (PRIV-2).
        match self {
            Self::Rgb(d) => write!(f, "Rgb({} bytes)", d.len()),
            Self::Rgba(d) => write!(f, "Rgba({} bytes)", d.len()),
            Self::I420 { y, u, v } => write!(f, "I420({} bytes)", y.len() + u.len() + v.len()),
        }
    }
}

/// One decoded camera frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Capture time since the stream started.
    pub timestamp: Duration,
    /// Pixels.
    pub data: PixelData,
}

impl VideoFrame {
    /// Encodes the frame as baseline JPEG 4:2:0 (video codec 1).
    pub fn to_jpeg(&self, quality: u8) -> Result<Vec<u8>, PlatformError> {
        use lull_media::video;
        let (w, h) = (
            u16::try_from(self.width).map_err(|_| PlatformError::InvalidArgument("frame width"))?,
            u16::try_from(self.height)
                .map_err(|_| PlatformError::InvalidArgument("frame height"))?,
        );
        match &self.data {
            PixelData::Rgb(d) => video::encode_rgb(d, w, h, quality),
            PixelData::Rgba(d) => video::encode_rgba(d, w, h, quality),
            PixelData::I420 { y, u, v } => video::encode_i420(
                &video::I420Image {
                    width: w,
                    height: h,
                    y,
                    u,
                    v,
                },
                quality,
            ),
        }
        .map_err(|e| PlatformError::Backend(e.to_string()))
    }
}

/// Frame callback, called on a backend thread. An `Err` ends the stream.
pub type FrameCallback = Box<dyn FnMut(Result<VideoFrame, PlatformError>) + Send + 'static>;

/// A camera mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CameraMode {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Frame rate, 0 when unknown.
    pub fps: u32,
}

/// A running capture. Dropping it stops the camera.
pub trait CameraStream: Send {
    /// The mode the camera runs at.
    fn mode(&self) -> CameraMode;
    /// Stops the camera (idempotent).
    fn stop(&mut self);
}

/// Camera access.
pub trait Camera: Send + Sync {
    /// Backend name for diagnostics.
    fn backend(&self) -> &'static str;
    /// Lists cameras.
    fn cameras(&self) -> Result<Vec<CameraInfo>, PlatformError>;
    /// Starts capturing. Returns once the camera is open, or fails with
    /// [`PlatformError::PermissionDenied`] / [`PlatformError::Unavailable`].
    fn start(
        &self,
        request: &CameraRequest,
        callback: FrameCallback,
    ) -> Result<Box<dyn CameraStream>, PlatformError>;
}

/// Picks the camera for a request: by id, then by facing, then the first.
pub fn pick_camera<'a>(
    cameras: &'a [CameraInfo],
    request: &CameraRequest,
) -> Result<&'a CameraInfo, PlatformError> {
    if let Some(id) = &request.camera {
        return cameras
            .iter()
            .find(|c| &c.id == id)
            .ok_or_else(|| PlatformError::NoSuchDevice(id.clone()));
    }
    request
        .facing
        .and_then(|f| cameras.iter().find(|c| c.facing == f))
        .or_else(|| cameras.first())
        .ok_or_else(|| PlatformError::Unavailable("no camera found".into()))
}

/// Chooses a mode for a request. Prefers the largest mode that fits within
/// the requested box, then one reaching the requested frame rate (closest
/// above it), then the caller's `rank` (higher is better). When nothing
/// fits, the smallest mode wins.
pub fn choose_mode<M: Copy>(
    modes: &[M],
    mode_of: impl Fn(&M) -> CameraMode,
    request: &CameraRequest,
    rank: impl Fn(&M) -> u8,
) -> Option<M> {
    let fits = |m: &CameraMode| m.width <= request.max_width && m.height <= request.max_height;
    let fps_score = |m: &CameraMode| -> (bool, i64) {
        if m.fps == 0 {
            (true, 0)
        } else if m.fps >= request.max_fps {
            (true, -i64::from(m.fps - request.max_fps))
        } else {
            (false, i64::from(m.fps))
        }
    };
    let area = |m: &CameraMode| u64::from(m.width) * u64::from(m.height);
    let fitting: Vec<&M> = modes.iter().filter(|m| fits(&mode_of(m))).collect();
    if fitting.is_empty() {
        return modes
            .iter()
            .min_by_key(|m| {
                let c = mode_of(m);
                (
                    area(&c),
                    std::cmp::Reverse(fps_score(&c)),
                    std::cmp::Reverse(rank(m)),
                )
            })
            .copied();
    }
    fitting
        .into_iter()
        .max_by_key(|m| {
            let c = mode_of(m);
            (area(&c), fps_score(&c), rank(m))
        })
        .copied()
}

/// Drops frames arriving faster than a target rate.
#[derive(Debug, Clone)]
pub struct FpsLimiter {
    interval: Duration,
    next: Option<Duration>,
}

impl FpsLimiter {
    /// A limiter for `max_fps` (0 disables limiting).
    pub fn new(max_fps: u32) -> Self {
        Self {
            interval: if max_fps == 0 {
                Duration::ZERO
            } else {
                Duration::from_secs(1) / max_fps
            },
            next: None,
        }
    }

    /// Whether a frame captured at `ts` should be delivered.
    pub fn accept(&mut self, ts: Duration) -> bool {
        match self.next {
            // 10 % tolerance so a camera running exactly at the target rate
            // is not decimated by timestamp jitter.
            Some(next) if ts + self.interval / 10 < next => false,
            Some(next) => {
                // Schedule from the ideal slot, but never fall more than a
                // slot behind.
                let base = if ts > next + self.interval { ts } else { next };
                self.next = Some(base + self.interval);
                true
            }
            None => {
                self.next = Some(ts + self.interval);
                true
            }
        }
    }
}

/// Converts packed YUYV 4:2:2 to I420 (averaging chroma over row pairs).
pub fn yuyv_to_i420(src: &[u8], width: u32, height: u32) -> Option<PixelData> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || w % 2 != 0 || src.len() < w * h * 2 {
        return None;
    }
    let (cw, ch) = (w / 2, h.div_ceil(2));
    let mut y = Vec::with_capacity(w * h);
    let mut u = vec![0u8; cw * ch];
    let mut v = vec![0u8; cw * ch];
    for row in 0..h {
        let line = &src[row * w * 2..(row + 1) * w * 2];
        for px in line.chunks_exact(4) {
            y.push(px[0]);
            y.push(px[2]);
        }
    }
    for crow in 0..ch {
        let r0 = crow * 2;
        let r1 = (r0 + 1).min(h - 1);
        for cx in 0..cw {
            let a = r0 * w * 2 + cx * 4;
            let b = r1 * w * 2 + cx * 4;
            u[crow * cw + cx] = avg(src[a + 1], src[b + 1]);
            v[crow * cw + cx] = avg(src[a + 3], src[b + 3]);
        }
    }
    Some(PixelData::I420 { y, u, v })
}

/// Converts NV12 (Y plane, interleaved CbCr plane) to I420.
pub fn nv12_to_i420(src: &[u8], width: u32, height: u32) -> Option<PixelData> {
    let (w, h) = (width as usize, height as usize);
    let (cw, ch) = (w.div_ceil(2), h.div_ceil(2));
    if w == 0 || h == 0 || src.len() < w * h + cw * ch * 2 {
        return None;
    }
    let y = src[..w * h].to_vec();
    let uv = &src[w * h..w * h + cw * ch * 2];
    let u = uv.iter().step_by(2).copied().collect();
    let v = uv.iter().skip(1).step_by(2).copied().collect();
    Some(PixelData::I420 { y, u, v })
}

/// Converts 8-bit grey to I420 with neutral chroma.
pub fn gray_to_i420(src: &[u8], width: u32, height: u32) -> Option<PixelData> {
    let (w, h) = (width as usize, height as usize);
    if w == 0 || h == 0 || src.len() < w * h {
        return None;
    }
    let c = w.div_ceil(2) * h.div_ceil(2);
    Some(PixelData::I420 {
        y: src[..w * h].to_vec(),
        u: vec![128; c],
        v: vec![128; c],
    })
}

fn avg(a: u8, b: u8) -> u8 {
    ((u16::from(a) + u16::from(b)).div_ceil(2)) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(width: u32, height: u32, fps: u32) -> CameraMode {
        CameraMode { width, height, fps }
    }

    #[test]
    fn choose_mode_prefers_largest_fitting() {
        let modes = [
            m(1920, 1080, 30),
            m(1280, 720, 30),
            m(640, 480, 30),
            m(320, 240, 30),
        ];
        let req = CameraRequest::default();
        assert_eq!(
            choose_mode(&modes, |x| *x, &req, |_| 0),
            Some(m(640, 480, 30))
        );
        let big = CameraRequest {
            max_width: 1280,
            max_height: 720,
            ..CameraRequest::default()
        };
        assert_eq!(
            choose_mode(&modes, |x| *x, &big, |_| 0),
            Some(m(1280, 720, 30))
        );
    }

    #[test]
    fn choose_mode_falls_back_to_smallest() {
        let modes = [m(1920, 1080, 30), m(1280, 720, 30)];
        let req = CameraRequest {
            max_width: 320,
            max_height: 240,
            ..CameraRequest::default()
        };
        assert_eq!(
            choose_mode(&modes, |x| *x, &req, |_| 0),
            Some(m(1280, 720, 30))
        );
        assert_eq!(choose_mode::<CameraMode>(&[], |x| *x, &req, |_| 0), None);
    }

    #[test]
    fn choose_mode_prefers_enough_fps_then_closest() {
        let modes = [m(640, 480, 5), m(640, 480, 30), m(640, 480, 15)];
        let req = CameraRequest::default(); // 10 fps
        assert_eq!(
            choose_mode(&modes, |x| *x, &req, |_| 0),
            Some(m(640, 480, 15))
        );
    }

    #[test]
    fn choose_mode_uses_rank_as_tie_break() {
        let modes = [(m(640, 480, 30), "yuyv"), (m(640, 480, 30), "mjpeg")];
        let pick = choose_mode(
            &modes,
            |x| x.0,
            &CameraRequest::default(),
            |x| u8::from(x.1 == "mjpeg"),
        );
        assert_eq!(pick.unwrap().1, "mjpeg");
    }

    #[test]
    fn pick_camera_by_id_facing_and_default() {
        let cams = vec![
            CameraInfo {
                id: "a".into(),
                name: "USB".into(),
                facing: Facing::Unknown,
            },
            CameraInfo {
                id: "b".into(),
                name: "Integrated Camera".into(),
                facing: Facing::Front,
            },
        ];
        let mut req = CameraRequest::default();
        assert_eq!(pick_camera(&cams, &req).unwrap().id, "a");
        req.facing = Some(Facing::Front);
        assert_eq!(pick_camera(&cams, &req).unwrap().id, "b");
        req.camera = Some("a".into());
        assert_eq!(pick_camera(&cams, &req).unwrap().id, "a");
        req.camera = Some("zz".into());
        assert!(matches!(
            pick_camera(&cams, &req),
            Err(PlatformError::NoSuchDevice(_))
        ));
        assert!(pick_camera(&[], &CameraRequest::default()).is_err());
    }

    #[test]
    fn facing_guess() {
        assert_eq!(
            Facing::guess_from_name("Integrated Camera: Integrated C"),
            Facing::Front
        );
        assert_eq!(Facing::guess_from_name("FaceTime HD Camera"), Facing::Front);
        assert_eq!(Facing::guess_from_name("Logitech C920"), Facing::Unknown);
        assert_eq!(Facing::guess_from_name("Rear camera"), Facing::Back);
        assert_eq!(Facing::Front.protocol_name(), Some("front"));
        assert_eq!(Facing::Unknown.protocol_name(), None);
    }

    #[test]
    fn fps_limiter_decimates_30_to_10() {
        let mut l = FpsLimiter::new(10);
        let accepted = (0..90)
            .filter(|i| l.accept(Duration::from_micros(i * 33_333)))
            .count();
        assert_eq!(accepted, 30);
    }

    #[test]
    fn fps_limiter_keeps_matching_rate_with_jitter() {
        let mut l = FpsLimiter::new(10);
        let ts = [0, 98, 201, 299, 400, 502, 597, 700];
        let accepted = ts
            .iter()
            .filter(|&&t| l.accept(Duration::from_millis(t)))
            .count();
        assert_eq!(accepted, ts.len());
        assert!(FpsLimiter::new(0).accept(Duration::ZERO));
    }

    #[test]
    fn fps_limiter_recovers_after_gap() {
        let mut l = FpsLimiter::new(10);
        assert!(l.accept(Duration::from_millis(0)));
        assert!(l.accept(Duration::from_secs(5)));
        assert!(!l.accept(Duration::from_millis(5050)));
        assert!(l.accept(Duration::from_millis(5100)));
    }

    #[test]
    fn yuyv_conversion() {
        // 2×2: row 0 Y=10,20 U=100 V=200; row 1 Y=30,40 U=110 V=210.
        let src = [10, 100, 20, 200, 30, 110, 40, 210];
        let PixelData::I420 { y, u, v } = yuyv_to_i420(&src, 2, 2).unwrap() else {
            panic!()
        };
        assert_eq!(y, vec![10, 20, 30, 40]);
        assert_eq!(u, vec![105]);
        assert_eq!(v, vec![205]);
        assert!(yuyv_to_i420(&src, 3, 1).is_none());
        assert!(yuyv_to_i420(&src[..4], 2, 2).is_none());
    }

    #[test]
    fn nv12_and_gray_conversion() {
        let src = [1, 2, 3, 4, 50, 60];
        let PixelData::I420 { y, u, v } = nv12_to_i420(&src, 2, 2).unwrap() else {
            panic!()
        };
        assert_eq!((y, u, v), (vec![1, 2, 3, 4], vec![50], vec![60]));
        let PixelData::I420 { y, u, .. } = gray_to_i420(&[9; 9], 3, 3).unwrap() else {
            panic!()
        };
        assert_eq!(y.len(), 9);
        assert_eq!(u, vec![128; 4]);
    }

    #[test]
    fn frame_encodes_to_jpeg_and_debug_hides_pixels() {
        let f = VideoFrame {
            width: 16,
            height: 8,
            timestamp: Duration::ZERO,
            data: PixelData::Rgb(vec![200; 16 * 8 * 3]),
        };
        let jpeg = f.to_jpeg(70).unwrap();
        assert_eq!(&jpeg[..2], &[0xFF, 0xD8]);
        assert_eq!(format!("{:?}", f.data), "Rgb(384 bytes)");
    }
}
