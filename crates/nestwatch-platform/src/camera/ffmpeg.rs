// SPDX-License-Identifier: AGPL-3.0-or-later
//! Linux camera fallback through an installed `ffmpeg`.
//!
//! `ffmpeg -f v4l2` emits MJPEG on a pipe (copied when the camera produces
//! MJPEG, encoded otherwise); [`JpegSplitter`] cuts the stream into images
//! and nestwatch-media's JPEG decoder turns them into RGBA frames. Cameras are
//! found in `/sys/class/video4linux`; nodes without a colour capture format
//! (metadata nodes, infrared sensors) are skipped.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::mjpeg::JpegSplitter;
use super::{
    Camera, CameraInfo, CameraMode, CameraRequest, CameraStream, Facing, FpsLimiter, FrameCallback,
    PixelData, VideoFrame, choose_mode, pick_camera,
};
use crate::PlatformError;
use crate::process::{StderrTail, find_program, kill, run_stderr};

const LIST_TIMEOUT: Duration = Duration::from_secs(3);
const START_TIMEOUT: Duration = Duration::from_secs(8);
/// Consecutive undecodable frames after which the stream fails.
const MAX_BAD_FRAMES: u32 = 30;

/// The ffmpeg camera backend.
#[derive(Debug, Clone)]
pub struct FfmpegCamera {
    ffmpeg: PathBuf,
    sysfs: PathBuf,
}

/// One capture format of a V4L2 device, as listed by ffmpeg.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct V4l2Format {
    /// ffmpeg's `-input_format` name (`mjpeg`, `yuyv422`, `gray`, ...).
    pub(crate) name: String,
    /// Whether the format is compressed.
    pub(crate) compressed: bool,
    /// Discrete sizes; empty for stepwise or continuous ranges.
    pub(crate) sizes: Vec<(u32, u32)>,
}

impl V4l2Format {
    fn is_colour(&self) -> bool {
        !matches!(
            self.name.as_str(),
            "gray" | "gray10le" | "gray12le" | "gray16le" | "y10bpack"
        )
    }
}

impl FfmpegCamera {
    /// Finds `ffmpeg` on `PATH`.
    pub fn detect() -> Result<Self, PlatformError> {
        let ffmpeg = find_program("ffmpeg")
            .ok_or_else(|| PlatformError::Unavailable("ffmpeg not installed".into()))?;
        Ok(Self {
            ffmpeg,
            sysfs: PathBuf::from("/sys/class/video4linux"),
        })
    }

    fn formats(&self, device: &str) -> Vec<V4l2Format> {
        let mut cmd = Command::new(&self.ffmpeg);
        cmd.args([
            "-hide_banner",
            "-nostdin",
            "-f",
            "v4l2",
            "-list_formats",
            "all",
            "-i",
        ])
        .arg(device);
        run_stderr(&mut cmd, LIST_TIMEOUT)
            .map(|out| parse_formats(&String::from_utf8_lossy(&out)))
            .unwrap_or_default()
    }
}

/// Lists `videoN` nodes with their sysfs names, sorted by N.
fn video_nodes(sysfs: &Path) -> Vec<(u32, String)> {
    let Ok(entries) = std::fs::read_dir(sysfs) else {
        return Vec::new();
    };
    let mut nodes: Vec<(u32, String)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let n = e
                .file_name()
                .to_str()?
                .strip_prefix("video")?
                .parse()
                .ok()?;
            let name = std::fs::read_to_string(e.path().join("name"))
                .map(|s| s.trim().to_owned())
                .unwrap_or_else(|_| format!("Camera {n}"));
            Some((n, name))
        })
        .collect();
    nodes.sort();
    nodes
}

/// Parses `ffmpeg -f v4l2 -list_formats all` output.
pub(crate) fn parse_formats(output: &str) -> Vec<V4l2Format> {
    output
        .lines()
        .filter_map(|line| {
            let body = line.split_once("] ")?.1;
            let (kind, rest) = body.split_once(':')?;
            let compressed = match kind.trim() {
                "Compressed" => true,
                "Raw" => false,
                _ => return None,
            };
            let name = rest.split(':').next()?.trim().to_owned();
            let sizes_part = rest.rsplit_once(" : ")?.1;
            let sizes = sizes_part
                .split_whitespace()
                .filter_map(|t| {
                    let (w, h) = t.split_once('x')?;
                    Some((w.parse().ok()?, h.parse().ok()?))
                })
                .collect();
            (!name.is_empty()).then_some(V4l2Format {
                name,
                compressed,
                sizes,
            })
        })
        .collect()
}

/// Candidate (mode, input format) pairs for a device. Stepwise formats are
/// offered at the requested size.
fn candidates(formats: &[V4l2Format], request: &CameraRequest) -> Vec<(CameraMode, usize)> {
    let mut out = Vec::new();
    for (i, f) in formats.iter().enumerate().filter(|(_, f)| f.is_colour()) {
        if f.sizes.is_empty() {
            out.push((
                CameraMode {
                    width: request.max_width,
                    height: request.max_height,
                    fps: 0,
                },
                i,
            ));
        }
        for &(width, height) in &f.sizes {
            out.push((
                CameraMode {
                    width,
                    height,
                    fps: 0,
                },
                i,
            ));
        }
    }
    out
}

impl Camera for FfmpegCamera {
    fn backend(&self) -> &'static str {
        "ffmpeg (v4l2)"
    }

    fn cameras(&self) -> Result<Vec<CameraInfo>, PlatformError> {
        Ok(video_nodes(&self.sysfs)
            .into_iter()
            .filter_map(|(n, name)| {
                let id = format!("/dev/video{n}");
                let formats = self.formats(&id);
                formats
                    .iter()
                    .any(V4l2Format::is_colour)
                    .then(|| CameraInfo {
                        facing: Facing::guess_from_name(&name),
                        name,
                        id,
                    })
            })
            .collect())
    }

    fn start(
        &self,
        request: &CameraRequest,
        mut callback: FrameCallback,
    ) -> Result<Box<dyn CameraStream>, PlatformError> {
        let cameras = self.cameras()?;
        let camera = pick_camera(&cameras, request)?;
        let formats = self.formats(&camera.id);
        let modes = candidates(&formats, request);
        let (mode, fi) = choose_mode(
            &modes,
            |m| m.0,
            request,
            |m| {
                // Prefer passing the camera's own MJPEG through.
                u8::from(formats[m.1].name == "mjpeg")
            },
        )
        .ok_or_else(|| PlatformError::Unavailable("camera has no usable format".into()))?;
        let format = &formats[fi];
        let max_fps = request.max_fps.max(1);

        let mut cmd = Command::new(&self.ffmpeg);
        cmd.args(["-hide_banner", "-nostdin", "-loglevel", "error"])
            .args(["-f", "v4l2", "-input_format", &format.name])
            .args(["-framerate", &max_fps.to_string()])
            .args(["-video_size", &format!("{}x{}", mode.width, mode.height)])
            .arg("-i")
            .arg(&camera.id)
            .arg("-an");
        if format.name == "mjpeg" {
            cmd.args(["-c:v", "copy"]);
        } else {
            cmd.args(["-c:v", "mjpeg", "-q:v", "3", "-pix_fmt", "yuvj420p"]);
        }
        cmd.args(["-f", "mjpeg", "pipe:1"]);
        let mut child: Child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| PlatformError::Unavailable(format!("cannot start ffmpeg: {e}")))?;
        let mut stdout = child.stdout.take().expect("piped stdout");
        let stderr = StderrTail::spawn(child.stderr.take().expect("piped stderr"), |_| true);
        let tail = stderr.handle();
        let stopping = Arc::new(AtomicBool::new(false));
        let thread_stop = Arc::clone(&stopping);
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), PlatformError>>();

        let reader = std::thread::Builder::new()
            .name("nestwatch-ffmpeg-camera".into())
            .spawn(move || {
                let started = Instant::now();
                let mut splitter = JpegSplitter::new();
                let mut limiter = FpsLimiter::new(max_fps);
                let mut chunk = vec![0u8; 64 * 1024];
                let mut bad = 0u32;
                let mut first = true;
                let failure = loop {
                    let n = match stdout.read(&mut chunk) {
                        Ok(0) => break None,
                        Ok(n) => n,
                        Err(e) => break Some(PlatformError::Io(e)),
                    };
                    for jpeg in splitter.push(&chunk[..n]) {
                        let ts = started.elapsed();
                        if !limiter.accept(ts) {
                            continue;
                        }
                        match nestwatch_media::video::decode_to_rgba(&jpeg) {
                            Ok(img) => {
                                bad = 0;
                                if first {
                                    first = false;
                                    let _ = ready_tx.send(Ok(()));
                                }
                                callback(Ok(VideoFrame {
                                    width: img.width,
                                    height: img.height,
                                    timestamp: ts,
                                    data: PixelData::Rgba(img.data),
                                }));
                            }
                            Err(e) => {
                                bad += 1;
                                log::debug!("undecodable camera frame: {e}");
                            }
                        }
                    }
                    if bad >= MAX_BAD_FRAMES {
                        break Some(PlatformError::Unavailable(
                            "camera frames cannot be decoded".into(),
                        ));
                    }
                };
                if thread_stop.load(Ordering::Relaxed) {
                    return;
                }
                // Give stderr a moment to deliver the reason.
                std::thread::sleep(Duration::from_millis(100));
                let why = tail.tail();
                let err = match failure {
                    Some(e) if why.is_empty() => e,
                    _ if !why.is_empty() => PlatformError::from_message(why),
                    _ => PlatformError::Unavailable("camera stream ended".into()),
                };
                if first {
                    let _ = ready_tx.send(Err(err));
                } else {
                    callback(Err(err));
                }
            })
            .map_err(PlatformError::Io)?;

        let mut stream = FfmpegStream {
            child,
            reader: Some(reader),
            stderr,
            stopping,
            mode: CameraMode {
                fps: max_fps,
                ..mode
            },
        };
        match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => Ok(Box::new(stream)),
            Ok(Err(e)) => {
                stream.stop();
                Err(e)
            }
            Err(_) => {
                stream.stop();
                Err(PlatformError::Unavailable(
                    "camera did not deliver a frame".into(),
                ))
            }
        }
    }
}

struct FfmpegStream {
    child: Child,
    reader: Option<JoinHandle<()>>,
    stderr: StderrTail,
    stopping: Arc<AtomicBool>,
    mode: CameraMode,
}

impl CameraStream for FfmpegStream {
    fn mode(&self) -> CameraMode {
        self.mode
    }

    fn stop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        kill(&mut self.child);
        if let Some(r) = self.reader.take() {
            let _ = r.join();
        }
        self.stderr.join();
    }
}

impl Drop for FfmpegStream {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LISTING: &str = "\
[video4linux2,v4l2 @ 0x55a0] Compressed:       mjpeg :          Motion-JPEG : 1280x720 640x480 640x360 1920x1080
[video4linux2,v4l2 @ 0x55a0] Raw       :     yuyv422 :           YUYV 4:2:2 : 640x480 640x360 1280x720
[in#0 @ 0x55a0] Error opening input: Immediate exit requested
Error opening input file /dev/video0.
";

    #[test]
    fn parses_ffmpeg_format_listing() {
        let f = parse_formats(LISTING);
        assert_eq!(f.len(), 2);
        assert_eq!(f[0].name, "mjpeg");
        assert!(f[0].compressed);
        assert_eq!(f[0].sizes[0], (1280, 720));
        assert_eq!(f[1].name, "yuyv422");
        assert!(!f[1].compressed);
        assert_eq!(f[1].sizes, vec![(640, 480), (640, 360), (1280, 720)]);
    }

    #[test]
    fn gray_only_and_metadata_nodes_are_not_cameras() {
        let ir = parse_formats(
            "[video4linux2,v4l2 @ 0x1] Raw       :        gray :      8-bit Greyscale : 640x360\n",
        );
        assert_eq!(ir.len(), 1);
        assert!(!ir.iter().any(V4l2Format::is_colour));
        let meta = parse_formats(
            "[video4linux2,v4l2 @ 0x1] ioctl(VIDIOC_G_INPUT): Inappropriate ioctl for device\n",
        );
        assert!(meta.is_empty());
    }

    #[test]
    fn prefers_mjpeg_at_equal_size() {
        let f = parse_formats(LISTING);
        let req = CameraRequest::default();
        let modes = candidates(&f, &req);
        let (mode, fi) =
            choose_mode(&modes, |m| m.0, &req, |m| u8::from(f[m.1].name == "mjpeg")).unwrap();
        assert_eq!((mode.width, mode.height), (640, 480));
        assert_eq!(f[fi].name, "mjpeg");
    }

    #[test]
    fn stepwise_format_offered_at_requested_size() {
        let f = vec![V4l2Format {
            name: "yuyv422".into(),
            compressed: false,
            sizes: vec![],
        }];
        let req = CameraRequest {
            max_width: 320,
            max_height: 240,
            ..CameraRequest::default()
        };
        let modes = candidates(&f, &req);
        assert_eq!((modes[0].0.width, modes[0].0.height), (320, 240));
    }

    #[test]
    fn lists_sysfs_nodes_in_order() {
        let dir = std::env::temp_dir().join(format!("nestwatch-v4l-{}", std::process::id()));
        for (n, name) in [(10, "B"), (2, "A")] {
            let d = dir.join(format!("video{n}"));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("name"), format!("{name}\n")).unwrap();
        }
        std::fs::create_dir_all(dir.join("v4l-subdev0")).unwrap();
        let nodes = video_nodes(&dir);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(nodes, vec![(2, "A".into()), (10, "B".into())]);
    }
}
