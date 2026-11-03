// SPDX-License-Identifier: AGPL-3.0-or-later
//! Running a station on desktop devices (product.md §3.1, platforms.md §2.4).

use std::sync::{Arc, Mutex};
use std::time::Duration;

use nestwatch_core::station::{
    AssetSource, BindScope, CameraAvailability, CameraFrame, ListenConfig, PixelFormat,
    StationConfig, StationEngine, StationError, StationEvent, WebConfig, WebOptions,
};
use nestwatch_core::{CameraFacing, Clock, Storage};
use nestwatch_platform::{
    AudioStream, CameraRequest, CameraStream, CaptureRequest, DesktopPlatform, Facing, PixelData,
    PlatformError, PlaybackRequest, PlaybackSource, Samples, SleepInhibition, StreamFormat,
    VideoFrame,
};
use nestwatch_ui::adapter::station::RunningStation;

/// Station settings from the environment (tests, servers): `NESTWATCH_STATION_NAME`,
/// `NESTWATCH_TCP_PORT`, `NESTWATCH_HTTP_PORT`, `NESTWATCH_HTTPS_PORT`, `NESTWATCH_LOOPBACK=1`,
/// `NESTWATCH_NO_MDNS=1`.
#[derive(Debug, Clone, Default)]
pub struct StationOptions {
    /// Name used until the user renames the station.
    pub name: Option<String>,
    /// TCP port (default 47100 with fallbacks).
    pub tcp_port: Option<u16>,
    /// HTTP port of the browser app (default 47180).
    pub http_port: Option<u16>,
    /// HTTPS port of the browser app (default 47181).
    pub https_port: Option<u16>,
    /// Bind to loopback only.
    pub loopback: bool,
    /// Do not advertise over mDNS.
    pub no_mdns: bool,
}

fn env_port(name: &str) -> Option<u16> {
    std::env::var(name).ok()?.parse().ok()
}

fn env_flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| v == "1" || v == "true")
}

impl StationOptions {
    /// Reads the environment.
    pub fn from_env() -> Self {
        Self {
            name: std::env::var("NESTWATCH_STATION_NAME").ok(),
            tcp_port: env_port("NESTWATCH_TCP_PORT"),
            http_port: env_port("NESTWATCH_HTTP_PORT"),
            https_port: env_port("NESTWATCH_HTTPS_PORT"),
            loopback: env_flag("NESTWATCH_LOOPBACK"),
            no_mdns: env_flag("NESTWATCH_NO_MDNS"),
        }
    }
}

fn port(p: Option<u16>, default: ListenConfig, loopback: bool) -> ListenConfig {
    let mut c = match p {
        Some(0) => ListenConfig {
            port: 0,
            attempts: 1,
            scope: BindScope::All,
        },
        Some(p) => ListenConfig::new(p),
        None => default,
    };
    if loopback {
        c.scope = BindScope::Loopback;
    }
    c
}

const PLACEHOLDER_APP: &[u8] = b"<!doctype html><meta charset=utf-8><title>Nestwatch</title>\
<p>This station was built without the browser listener app (apps/web/build.sh).</p>";

/// The station's device handles; dropping it stops them.
struct Devices {
    _mic: Option<Box<dyn AudioStream>>,
    _speaker: Option<Box<dyn AudioStream>>,
    _sleep: Option<SleepInhibition>,
    camera: Arc<Mutex<Option<Box<dyn CameraStream>>>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Devices {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
        if let Ok(mut c) = self.camera.lock()
            && let Some(mut s) = c.take()
        {
            s.stop();
        }
    }
}

/// Starts the station engine and its devices.
pub async fn start(
    platform: DesktopPlatform,
    storage: Arc<dyn Storage>,
    clock: Arc<dyn Clock>,
    opts: StationOptions,
) -> Result<RunningStation, StationError> {
    let mut cfg = StationConfig::new(opts.name.clone().unwrap_or_else(default_name));
    cfg.tcp = port(opts.tcp_port, ListenConfig::default(), opts.loopback);
    cfg.mdns = !opts.no_mdns;
    let defaults = WebConfig::default();
    let assets = match crate::web_dir() {
        Some(d) => {
            log::info!("serving the browser listener from {}", d.display());
            AssetSource::Dir(d)
        }
        None => {
            log::warn!("no browser listener bundle found; serving a placeholder");
            AssetSource::memory([("index.html", PLACEHOLDER_APP)])
        }
    };
    cfg.web = Some(WebOptions {
        config: WebConfig {
            http: Some(port(
                opts.http_port,
                defaults.http.unwrap_or_default(),
                opts.loopback,
            )),
            https: Some(port(
                opts.https_port,
                defaults.https.unwrap_or_default(),
                opts.loopback,
            )),
            ..defaults
        },
        assets,
    });
    let engine = StationEngine::start(cfg, storage, clock).await?;
    let devices = start_devices(&platform, &engine);
    Ok(RunningStation {
        engine,
        guard: Box::new(devices),
    })
}

fn default_name() -> String {
    "Nestwatch station".into()
}

fn start_devices(platform: &DesktopPlatform, engine: &StationEngine) -> Devices {
    // FR-ST-1: capture continuously.
    let mic = {
        let e = engine.clone();
        let cb = Box::new(move |f: StreamFormat, s: Samples<'_>| match s {
            Samples::F32(x) => e.push_capture_f32(x, f.sample_rate, f.channels),
            Samples::I16(x) => e.push_capture(x, f.sample_rate, f.channels),
        });
        let req = CaptureRequest {
            device: None,
            sample_rate: Some(48_000),
            channels: Some(1),
        };
        match platform.audio_input.start(&req, cb) {
            Ok(s) => Some(s),
            Err(e) => {
                log::warn!("no microphone ({}): the station sends silence", e);
                None
            }
        }
    };
    // FR-TB-3: talk-back to the speaker at the user's volume.
    let speaker = {
        let e = engine.clone();
        let src = PlaybackSource::Pcm16k(Box::new(move |frame| e.pull_talkback(frame)));
        match platform
            .audio_output
            .start(&PlaybackRequest::default(), src)
        {
            Ok(s) => Some(s),
            Err(e) => {
                log::warn!("no speaker for talk-back ({e})");
                None
            }
        }
    };
    // platforms.md §2.4, PLATFORM-4.
    let sleep = match platform.power.inhibit_sleep("Nestwatch station is monitoring") {
        Ok(s) => Some(s),
        Err(e) => {
            log::warn!("cannot prevent system sleep ({e})");
            None
        }
    };
    let camera = Arc::new(Mutex::new(None));
    let tasks = vec![
        tokio::spawn(camera_task(
            platform.clone(),
            engine.clone(),
            camera.clone(),
        )),
        tokio::spawn(battery_task(platform.clone(), engine.clone())),
    ];
    Devices {
        _mic: mic,
        _speaker: speaker,
        _sleep: sleep,
        camera,
        tasks,
    }
}

/// FR-ST-6: battery every 30 s.
async fn battery_task(platform: DesktopPlatform, engine: StationEngine) {
    loop {
        match platform.battery.status() {
            Ok(Some(b)) => engine.set_battery(Some(b.percent), Some(b.charging)),
            Ok(None) => engine.set_battery(None, None),
            Err(e) => log::debug!("battery: {e}"),
        }
        tokio::time::sleep(Duration::from_secs(30)).await;
    }
}

/// FR-CAM-1: the camera runs exactly while someone watches.
async fn camera_task(
    platform: DesktopPlatform,
    engine: StationEngine,
    slot: Arc<Mutex<Option<Box<dyn CameraStream>>>>,
) {
    let mut events = engine.events();
    loop {
        let event = match events.recv().await {
            Ok(e) => e,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => break,
        };
        match event {
            StationEvent::CameraWanted(req) => {
                stop_camera(&slot);
                let platform = platform.clone();
                let e = engine.clone();
                let started = tokio::task::spawn_blocking(move || {
                    let request = CameraRequest {
                        camera: None,
                        facing: Some(match req.facing {
                            CameraFacing::Front => Facing::Front,
                            CameraFacing::Back => Facing::Back,
                        }),
                        max_width: req.max_w,
                        max_height: req.max_h,
                        max_fps: req.max_fps,
                    };
                    let pushing = e.clone();
                    platform.camera.start(
                        &request,
                        Box::new(move |f: Result<VideoFrame, PlatformError>| {
                            if let Ok(f) = f
                                && let Some(frame) = camera_frame(&f)
                            {
                                pushing.push_camera_frame(&frame);
                            }
                        }),
                    )
                })
                .await;
                match started {
                    Ok(Ok(stream)) => {
                        engine.set_camera_availability(CameraAvailability::Available);
                        if let Ok(mut s) = slot.lock() {
                            *s = Some(stream);
                        }
                    }
                    Ok(Err(PlatformError::PermissionDenied(m))) => {
                        log::warn!("camera permission denied: {m}");
                        engine.set_camera_availability(CameraAvailability::Denied);
                    }
                    Ok(Err(e)) => {
                        log::warn!("camera unavailable: {e}");
                        engine.set_camera_availability(CameraAvailability::Unavailable);
                    }
                    Err(e) => log::error!("camera task failed: {e}"),
                }
            }
            StationEvent::CameraNotWanted => stop_camera(&slot),
            _ => {}
        }
    }
    stop_camera(&slot);
}

fn stop_camera(slot: &Arc<Mutex<Option<Box<dyn CameraStream>>>>) {
    if let Ok(mut s) = slot.lock()
        && let Some(mut stream) = s.take()
    {
        stream.stop();
    }
}

/// Converts a platform frame for the engine (RGB or RGBA; I420 is
/// converted with full-range BT.601).
pub fn camera_frame(f: &VideoFrame) -> Option<CameraFrame> {
    let (format, data) = match &f.data {
        PixelData::Rgb(d) => (PixelFormat::Rgb, d.clone()),
        PixelData::Rgba(d) => (PixelFormat::Rgba, d.clone()),
        PixelData::I420 { y, u, v } => (PixelFormat::Rgb, i420_to_rgb(y, u, v, f.width, f.height)?),
    };
    Some(CameraFrame {
        width: f.width,
        height: f.height,
        format,
        data,
    })
}

fn i420_to_rgb(y: &[u8], u: &[u8], v: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    let (w, h) = (w as usize, h as usize);
    let cw = w.div_ceil(2);
    if y.len() < w * h || u.len() < cw * h.div_ceil(2) || v.len() < cw * h.div_ceil(2) {
        return None;
    }
    let mut out = Vec::with_capacity(w * h * 3);
    for row in 0..h {
        for col in 0..w {
            let yy = f32::from(y[row * w + col]);
            let ci = (row / 2) * cw + col / 2;
            let cb = f32::from(u[ci]) - 128.0;
            let cr = f32::from(v[ci]) - 128.0;
            let r = yy + 1.402 * cr;
            let g = yy - 0.344_136 * cb - 0.714_136 * cr;
            let b = yy + 1.772 * cb;
            out.extend([r, g, b].map(|c| c.round().clamp(0.0, 255.0) as u8));
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn i420_grey_and_colours_convert() {
        // 2×2 mid grey.
        let rgb = i420_to_rgb(&[128; 4], &[128], &[128], 2, 2).unwrap();
        assert_eq!(rgb, vec![128; 12]);
        // Pure red-ish chroma.
        let rgb = i420_to_rgb(&[76; 4], &[85], &[255], 2, 2).unwrap();
        assert!(
            rgb[0] > 240 && rgb[1] < 10 && rgb[2] < 10,
            "{:?}",
            &rgb[..3]
        );
        assert!(i420_to_rgb(&[0; 3], &[0], &[0], 2, 2).is_none());
    }

    #[test]
    fn frames_keep_their_format() {
        let f = VideoFrame {
            width: 2,
            height: 1,
            timestamp: Duration::ZERO,
            data: PixelData::Rgba(vec![1; 8]),
        };
        let c = camera_frame(&f).unwrap();
        assert_eq!((c.width, c.height, c.format), (2, 1, PixelFormat::Rgba));
    }
}
