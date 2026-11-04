// SPDX-License-Identifier: AGPL-3.0-or-later
//! The browser station (product.md §3.10, platforms.md §2.5): the
//! nestwatch-core station engine, relay-only (rendezvous.md FR-RV-7), with
//! the page's devices.
//!
//! - Microphone: `getUserMedia` without the browser's echo cancellation,
//!   noise suppression or gain (the engine's capture chain does gain and AGC,
//!   media.md §1), in its own `AudioContext` at the device rate (UI-9), fed
//!   to `push_capture_f32` (FR-WEBST-4).
//! - Talk-back: a 16 kHz `AudioContext` pulling `pull_talkback` through the
//!   playback worklet (FR-WEBST-6). Both contexts start on a user gesture;
//!   until then the station screen asks for a tap.
//! - Camera: on `CameraWanted` only, `getUserMedia` video drawn into a canvas
//!   at the requested size and rate; the RGBA pixels go to
//!   `push_camera_frame`, which scales, filters and JPEG-encodes per listener
//!   (media.md §5, FR-WEBST-5). Hidden page: camera unavailable (FR-WEBST-9).
//! - Battery Status API (FR-WEBST-7), Screen Wake Lock (FR-WEBST-8),
//!   `beforeunload` (FR-WEBST-10), persistent storage (security.md §7.1).
//!
//! Counters are mirrored to `window.__nestwatchStation` for diagnostics and
//! tests (headless browsers cannot be watched or heard).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use js_sys::{Function, Object, Promise, Reflect};
use nestwatch_core::station::{
    CameraAvailability, CameraFrame, CameraRequest, PixelFormat, StationConfig, StationEngine,
    StationEvent,
};
use nestwatch_core::{CameraFacing, Clock, Storage};
use nestwatch_ui::adapter::station::RunningStation;
use nestwatch_ui::model::StationDevices;
use tokio::sync::{mpsc, watch};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    AudioContext, AudioContextOptions, AudioContextState, CanvasRenderingContext2d,
    HtmlCanvasElement, HtmlVideoElement, MediaStream, MediaStreamConstraints,
};

use crate::audio::{self, Node, PullFrame, WorkletStats};
use crate::host::HostMsg;

/// Talk-back frame: 20 ms at 16 kHz.
const FRAME: usize = 320;
/// Battery poll interval (FR-ST-6; the desktop polls every 30 s too).
const BATTERY_POLL: Duration = Duration::from_secs(30);

thread_local! {
    /// The running station's devices, for the page-wide gesture handler.
    static ACTIVE: RefCell<Weak<Devices>> = const { RefCell::new(Weak::new()) };
}

fn set(o: &Object, k: &str, v: impl Into<JsValue>) {
    let _ = Reflect::set(o, &JsValue::from_str(k), &v.into());
}

fn get(o: &JsValue, k: &str) -> JsValue {
    Reflect::get(o, &JsValue::from_str(k)).unwrap_or(JsValue::UNDEFINED)
}

/// Calls `o[name](args...)` if it is a function.
fn call(o: &JsValue, name: &str, args: &[JsValue]) -> Option<JsValue> {
    let f = get(o, name).dyn_into::<Function>().ok()?;
    let a = js_sys::Array::new();
    for v in args {
        a.push(v);
    }
    f.apply(o, &a).ok()
}

fn log(msg: &str) {
    web_sys::console::log_1(&JsValue::from_str(msg));
}

fn document_hidden() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .is_some_and(|d| d.hidden())
}

/// One camera capture (FR-CAM-1): its stream and request.
struct Camera {
    stream: MediaStream,
    request: CameraRequest,
    _video: HtmlVideoElement,
}

/// The microphone and the talk-back speaker.
struct Sound {
    mic_ctx: Option<AudioContext>,
    mic_stream: Option<MediaStream>,
    _mic_node: Option<Node>,
    speaker_ctx: Option<AudioContext>,
    _speaker_node: Option<Node>,
}

/// Everything a running browser station holds; dropping the guard (stop
/// the station) releases it all.
pub struct Devices {
    engine: StationEngine,
    tx: mpsc::UnboundedSender<HostMsg>,
    model: RefCell<StationDevices>,
    stop: watch::Sender<bool>,
    sound: RefCell<Sound>,
    camera: RefCell<Option<Camera>>,
    camera_wanted: RefCell<Option<CameraRequest>>,
    camera_gen: Cell<u64>,
    camera_denied: Cell<bool>,
    wake_lock: RefCell<Option<JsValue>>,
    battery: RefCell<Option<JsValue>>,
    listeners: RefCell<Vec<(web_sys::EventTarget, &'static str, JsValue)>>,
    stats: Object,
}

impl std::fmt::Debug for Devices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Devices").finish_non_exhaustive()
    }
}

/// The guard handed to the UI: dropping it stops the devices.
struct Guard(Rc<Devices>);

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.release();
    }
}

/// Resumes the station's audio contexts; call from any user gesture.
pub fn resume_audio() {
    if let Some(d) = ACTIVE.with(|a| a.borrow().upgrade()) {
        d.resume();
    }
}

/// The browser's name for default device names.
pub fn browser_name() -> &'static str {
    let ua = web_sys::window()
        .and_then(|w| w.navigator().user_agent().ok())
        .unwrap_or_default();
    if ua.contains("Firefox") {
        "Firefox"
    } else if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("Chrome") {
        "Chrome"
    } else if ua.contains("Safari") {
        "Safari"
    } else {
        "Browser"
    }
}

/// Starts the relay-only station engine and its devices. `relay_origin` is
/// the page's origin when a relay server serves the app (FR-WEBST-2);
/// otherwise the stored (or default) server is used.
pub async fn start(
    storage: Arc<dyn Storage>,
    clock: Arc<dyn Clock>,
    relay_origin: Option<String>,
    tx: mpsc::UnboundedSender<HostMsg>,
) -> Result<RunningStation, String> {
    let cfg = StationConfig::new(nestwatch_ui::strings::browser_station_name(browser_name()));
    let engine = StationEngine::start(cfg, storage, clock)
        .await
        .map_err(|e| e.to_string())?;
    if let Some(origin) = &relay_origin {
        engine.set_relay_server(origin).map_err(|e| e.to_string())?;
    }
    // FR-WEBST-1: the relay is the only way in, so it is always on.
    engine.set_relay_enabled(true).map_err(|e| e.to_string())?;
    let stats = Object::new();
    if let Some(w) = web_sys::window() {
        let _ = Reflect::set(&w, &JsValue::from_str("__nestwatchStation"), &stats);
    }
    let (stop, _) = watch::channel(false);
    let devices = Rc::new(Devices {
        engine: engine.clone(),
        tx,
        model: RefCell::new(StationDevices::default()),
        stop,
        sound: RefCell::new(Sound {
            mic_ctx: None,
            mic_stream: None,
            _mic_node: None,
            speaker_ctx: None,
            _speaker_node: None,
        }),
        camera: RefCell::new(None),
        camera_wanted: RefCell::new(None),
        camera_gen: Cell::new(0),
        camera_denied: Cell::new(false),
        wake_lock: RefCell::new(None),
        battery: RefCell::new(None),
        listeners: RefCell::new(Vec::new()),
        stats,
    });
    ACTIVE.with(|a| *a.borrow_mut() = Rc::downgrade(&devices));
    devices.start();
    Ok(RunningStation {
        engine,
        guard: Box::new(Guard(devices)),
    })
}

impl Devices {
    fn start(self: &Rc<Self>) {
        set(&self.stats, "running", true);
        self.start_speaker();
        self.start_mic();
        self.spawn_events();
        self.spawn_battery();
        self.request_wake_lock();
        self.watch_page();
        persist_storage();
        if document_hidden() {
            self.page_hidden();
        }
    }

    fn publish(&self, f: impl FnOnce(&mut StationDevices)) {
        let m = {
            let mut m = self.model.borrow_mut();
            f(&mut m);
            m.clone()
        };
        set(&self.stats, "needsTap", m.needs_tap);
        let _ = self.tx.send(HostMsg::StationDevices(m));
    }

    fn stopped(&self) -> bool {
        *self.stop.borrow()
    }

    /// Stops everything (the station was stopped or the page closes).
    fn release(&self) {
        let _ = self.stop.send(true);
        set(&self.stats, "running", false);
        self.stop_camera();
        {
            let mut s = self.sound.borrow_mut();
            if let Some(st) = s.mic_stream.take() {
                audio::stop_tracks(&st);
            }
            if let Some(c) = s.mic_ctx.take() {
                let _ = c.close();
            }
            if let Some(c) = s.speaker_ctx.take() {
                let _ = c.close();
            }
            s._mic_node = None;
            s._speaker_node = None;
        }
        if let Some(lock) = self.wake_lock.borrow_mut().take() {
            let _ = call(&lock, "release", &[]);
        }
        for (target, event, f) in self.listeners.borrow_mut().drain(..) {
            let _ = target.remove_event_listener_with_callback(event, f.unchecked_ref());
        }
        let _ = self
            .tx
            .send(HostMsg::StationDevices(StationDevices::default()));
    }

    // ----- sound -------------------------------------------------------------

    /// Whether either audio context still waits for a gesture.
    fn sound_locked(&self) -> bool {
        let s = self.sound.borrow();
        [&s.mic_ctx, &s.speaker_ctx]
            .into_iter()
            .flatten()
            .any(|c| c.state() != AudioContextState::Running)
    }

    fn update_lock(&self) {
        let locked = self.sound_locked();
        if self.model.borrow().needs_tap != locked {
            self.publish(|m| m.needs_tap = locked);
        }
        let s = self.sound.borrow();
        let state = |c: &Option<AudioContext>| match c.as_ref().map(|c| c.state()) {
            Some(AudioContextState::Running) => "running",
            Some(_) => "suspended",
            None => "none",
        };
        set(&self.stats, "micContext", state(&s.mic_ctx));
        set(&self.stats, "speakerContext", state(&s.speaker_ctx));
    }

    /// Resumes both contexts (a gesture happened) and updates the prompt.
    fn resume(self: &Rc<Self>) {
        let promises: Vec<Promise> = {
            let s = self.sound.borrow();
            [&s.mic_ctx, &s.speaker_ctx]
                .into_iter()
                .flatten()
                .filter(|c| c.state() != AudioContextState::Running)
                .filter_map(|c| c.resume().ok())
                .collect()
        };
        if promises.is_empty() {
            return;
        }
        let this = self.clone();
        spawn_local(async move {
            for p in promises {
                let _ = JsFuture::from(p).await;
            }
            this.update_lock();
        });
    }

    /// FR-WEBST-6: talk-back through a 16 kHz context and the playback
    /// worklet, pulled at the output's pace (CORE-12).
    fn start_speaker(self: &Rc<Self>) {
        let opts = AudioContextOptions::new();
        opts.set_sample_rate(16_000.0);
        let Ok(ctx) = AudioContext::new_with_context_options(&opts) else {
            log("Nestwatch station: no 16 kHz AudioContext for talk-back");
            return;
        };
        let _ = ctx.resume();
        self.sound.borrow_mut().speaker_ctx = Some(ctx.clone());
        let this = self.clone();
        spawn_local(async move {
            let engine = this.engine.clone();
            let stats = this.stats.clone();
            let (pulled, voiced) = (Rc::new(Cell::new(0.0)), Rc::new(Cell::new(0.0)));
            let (p2, v2) = (pulled.clone(), voiced.clone());
            let pull: Rc<PullFrame> = Rc::new(move |out: &mut [f32]| {
                let mut pcm = [0i16; FRAME];
                let talking = engine.pull_talkback(&mut pcm);
                let mut loud = false;
                for (o, s) in out.iter_mut().zip(pcm) {
                    *o = f32::from(s) / 32768.0;
                    loud |= s != 0;
                }
                p2.set(p2.get() + 1.0);
                if talking && loud {
                    v2.set(v2.get() + 1.0);
                }
            });
            let node = if audio::load_worklet(&ctx).await {
                let stats = stats.clone();
                let (pulled, voiced) = (pulled.clone(), voiced.clone());
                let on_stats: Rc<WorkletStats> = Rc::new(move |underruns, played| {
                    set(&stats, "talkPulled", pulled.get());
                    set(&stats, "talkVoiced", voiced.get());
                    set(&stats, "talkUnderruns", underruns);
                    set(&stats, "talkPlayed", played);
                });
                audio::playback_worklet(&ctx, pull.clone(), on_stats)
            } else {
                None
            };
            let node = node.or_else(|| {
                let on_played: Rc<dyn Fn(usize)> = Rc::new(move |_| {
                    set(&stats, "talkPulled", pulled.get());
                    set(&stats, "talkVoiced", voiced.get());
                });
                audio::playback_script_processor(&ctx, pull, on_played)
            });
            if this.stopped() {
                let _ = ctx.close();
                return;
            }
            this.sound.borrow_mut()._speaker_node = node;
            this.update_lock();
        });
    }

    /// FR-WEBST-4: the room as it is, continuously.
    fn start_mic(self: &Rc<Self>) {
        let Some(promise) = audio::microphone(false) else {
            self.publish(|m| m.mic = Some(false));
            return;
        };
        let this = self.clone();
        spawn_local(async move {
            let stream: MediaStream = match JsFuture::from(promise).await {
                Ok(s) => s.unchecked_into(),
                Err(e) => {
                    web_sys::console::warn_2(
                        &JsValue::from_str("Nestwatch station: microphone unavailable"),
                        &e,
                    );
                    this.publish(|m| m.mic = Some(false));
                    return;
                }
            };
            if this.stopped() {
                audio::stop_tracks(&stream);
                return;
            }
            let Ok(ctx) = AudioContext::new() else {
                audio::stop_tracks(&stream);
                this.publish(|m| m.mic = Some(false));
                return;
            };
            let _ = ctx.resume();
            let rate = ctx.sample_rate() as u32;
            set(&this.stats, "micRate", rate);
            let worklet = audio::load_worklet(&ctx).await;
            if this.stopped() {
                audio::stop_tracks(&stream);
                let _ = ctx.close();
                return;
            }
            let engine = this.engine.clone();
            let stats = this.stats.clone();
            let (blocks, samples, peak) = (Cell::new(0.0), Cell::new(0.0), Cell::new(0.0f32));
            let feed: Rc<audio::MicFeed> = Rc::new(move |block: &[f32]| {
                engine.push_capture_f32(block, rate, 1);
                blocks.set(blocks.get() + 1.0);
                samples.set(samples.get() + block.len() as f64);
                peak.set(block.iter().fold(peak.get(), |p, s| p.max(s.abs())));
                set(&stats, "micBlocks", blocks.get());
                set(&stats, "micSamples", samples.get());
                set(&stats, "micPeak", f64::from(peak.get()));
            });
            let Some(node) = audio::capture(&ctx, &stream, worklet, feed) else {
                audio::stop_tracks(&stream);
                let _ = ctx.close();
                this.publish(|m| m.mic = Some(false));
                return;
            };
            {
                let mut s = this.sound.borrow_mut();
                s.mic_ctx = Some(ctx);
                s.mic_stream = Some(stream);
                s._mic_node = Some(node);
            }
            log("Nestwatch station: microphone on");
            this.publish(|m| m.mic = Some(true));
            this.update_lock();
        });
    }

    // ----- engine events -----------------------------------------------------

    fn spawn_events(self: &Rc<Self>) {
        let this = self.clone();
        let mut stop = self.stop.subscribe();
        let mut events = self.engine.events();
        spawn_local(async move {
            loop {
                let ev = tokio::select! {
                    e = events.recv() => e,
                    _ = stop.wait_for(|s| *s) => break,
                };
                match ev {
                    Ok(StationEvent::CameraWanted(req)) => this.camera_wanted(req),
                    Ok(StationEvent::CameraNotWanted) => {
                        *this.camera_wanted.borrow_mut() = None;
                        this.stop_camera();
                    }
                    // A talker needs the speaker running.
                    Ok(StationEvent::TalkStarted { .. }) => this.resume(),
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
        });
    }

    // ----- camera ------------------------------------------------------------

    fn stop_camera(&self) {
        self.camera_gen.set(self.camera_gen.get() + 1);
        if let Some(c) = self.camera.borrow_mut().take() {
            audio::stop_tracks(&c.stream);
            set(&self.stats, "camera", "off");
        }
    }

    /// FR-CAM-1, CORE-8: (re)start the camera for the aggregate request.
    fn camera_wanted(self: &Rc<Self>, req: CameraRequest) {
        *self.camera_wanted.borrow_mut() = Some(req.clone());
        if document_hidden() || self.camera_denied.get() {
            return;
        }
        if self
            .camera
            .borrow()
            .as_ref()
            .is_some_and(|c| c.request == req)
        {
            return;
        }
        self.stop_camera();
        let gen_ = self.camera_gen.get();
        let Some(devices) = web_sys::window().and_then(|w| w.navigator().media_devices().ok())
        else {
            self.engine
                .set_camera_availability(CameraAvailability::Unavailable);
            return;
        };
        let video = Object::new();
        let ideal = |v: u32| {
            let o = Object::new();
            set(&o, "ideal", v);
            o
        };
        set(
            &video,
            "facingMode",
            match req.facing {
                CameraFacing::Front => "user",
                CameraFacing::Back => "environment",
            },
        );
        set(&video, "width", ideal(req.max_w));
        set(&video, "height", ideal(req.max_h));
        set(&video, "frameRate", ideal(req.max_fps));
        let constraints = MediaStreamConstraints::new();
        constraints.set_video(&video);
        constraints.set_audio(&JsValue::FALSE);
        let Ok(promise) = devices.get_user_media_with_constraints(&constraints) else {
            self.engine
                .set_camera_availability(CameraAvailability::Unavailable);
            return;
        };
        set(&self.stats, "camera", "starting");
        let this = self.clone();
        spawn_local(async move {
            let stream: MediaStream = match JsFuture::from(promise).await {
                Ok(s) => s.unchecked_into(),
                Err(e) => {
                    let name = get(&e, "name").as_string().unwrap_or_default();
                    web_sys::console::warn_2(
                        &JsValue::from_str("Nestwatch station: camera unavailable"),
                        &e,
                    );
                    set(&this.stats, "camera", name.as_str());
                    if name == "NotAllowedError" || name == "SecurityError" {
                        this.camera_denied.set(true);
                        this.engine
                            .set_camera_availability(CameraAvailability::Denied);
                    } else {
                        this.engine
                            .set_camera_availability(CameraAvailability::Unavailable);
                    }
                    return;
                }
            };
            if this.camera_gen.get() != gen_ || this.stopped() {
                audio::stop_tracks(&stream);
                return;
            }
            let Some((video, canvas, ctx2d)) = video_and_canvas(&stream) else {
                audio::stop_tracks(&stream);
                this.engine
                    .set_camera_availability(CameraAvailability::Unavailable);
                return;
            };
            if let Ok(p) = video.play() {
                let _ = JsFuture::from(p).await;
            }
            *this.camera.borrow_mut() = Some(Camera {
                stream,
                request: req.clone(),
                _video: video.clone(),
            });
            this.engine
                .set_camera_availability(CameraAvailability::Available);
            set(&this.stats, "camera", "on");
            log("Nestwatch station: camera on");
            this.camera_loop(gen_, req, video, canvas, ctx2d).await;
        });
    }

    /// Grabs frames at the requested rate while this capture is current.
    async fn camera_loop(
        &self,
        gen_: u64,
        req: CameraRequest,
        video: HtmlVideoElement,
        canvas: HtmlCanvasElement,
        ctx2d: CanvasRenderingContext2d,
    ) {
        let period = Duration::from_millis(1000 / u64::from(req.max_fps.clamp(1, 30)));
        let mut frames = 0.0;
        while self.camera_gen.get() == gen_ && !self.stopped() {
            gloo_timers::future::sleep(period).await;
            if self.camera_gen.get() != gen_ {
                break;
            }
            let (vw, vh) = (video.video_width(), video.video_height());
            if video.ready_state() < 2 || vw == 0 || vh == 0 {
                continue;
            }
            // Never larger than requested, aspect kept (media.md §5); the
            // engine scales further per listener.
            let scale = (f64::from(req.max_w) / f64::from(vw))
                .min(f64::from(req.max_h) / f64::from(vh))
                .min(1.0);
            let w = ((f64::from(vw) * scale).round() as u32).max(16);
            let h = ((f64::from(vh) * scale).round() as u32).max(16);
            if canvas.width() != w || canvas.height() != h {
                canvas.set_width(w);
                canvas.set_height(h);
            }
            if ctx2d
                .draw_image_with_html_video_element_and_dw_and_dh(
                    &video,
                    0.0,
                    0.0,
                    f64::from(w),
                    f64::from(h),
                )
                .is_err()
            {
                continue;
            }
            let Ok(img) = ctx2d.get_image_data(0.0, 0.0, f64::from(w), f64::from(h)) else {
                continue;
            };
            self.engine.push_camera_frame(&CameraFrame {
                width: w,
                height: h,
                format: PixelFormat::Rgba,
                data: img.data().0,
            });
            frames += 1.0;
            set(&self.stats, "cameraFrames", frames);
            set(&self.stats, "cameraWidth", w);
            set(&self.stats, "cameraHeight", h);
        }
    }

    // ----- page --------------------------------------------------------------

    /// FR-WEBST-9: hidden pages lose the camera.
    fn page_hidden(&self) {
        self.publish(|m| m.hidden = true);
        self.stop_camera();
        if !self.camera_denied.get() {
            self.engine
                .set_camera_availability(CameraAvailability::Unavailable);
        }
    }

    fn page_visible(self: &Rc<Self>) {
        self.publish(|m| m.hidden = false);
        if !self.camera_denied.get() {
            self.engine
                .set_camera_availability(CameraAvailability::Available);
        }
        // Viewers were told "unavailable" and ask again; a request that
        // is still standing restarts the capture.
        if let Some(req) = self.camera_wanted.borrow().clone() {
            self.camera_wanted(req);
        }
        self.request_wake_lock();
        self.resume();
    }

    fn listen(&self, target: web_sys::EventTarget, event: &'static str, f: JsValue) {
        let _ = target.add_event_listener_with_callback(event, f.unchecked_ref());
        self.listeners.borrow_mut().push((target, event, f));
    }

    fn watch_page(self: &Rc<Self>) {
        let Some(window) = web_sys::window() else {
            return;
        };
        if let Some(doc) = window.document() {
            let weak = Rc::downgrade(self);
            let f = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
                if let Some(this) = weak.upgrade() {
                    if document_hidden() {
                        this.page_hidden();
                    } else {
                        this.page_visible();
                    }
                }
            });
            self.listen(doc.into(), "visibilitychange", f.into_js_value());
        }
        // FR-WEBST-10: ask before the page goes away.
        let f = Closure::<dyn FnMut(web_sys::Event)>::new(move |e: web_sys::Event| {
            e.prevent_default();
            let _ = Reflect::set(
                &e,
                &JsValue::from_str("returnValue"),
                &JsValue::from_str(""),
            );
        });
        self.listen(window.into(), "beforeunload", f.into_js_value());
    }

    /// FR-WEBST-8: a screen wake lock while running (again after every
    /// return to the visible state: browsers release it when hidden).
    fn request_wake_lock(self: &Rc<Self>) {
        let Some(nav) = web_sys::window().map(|w| JsValue::from(w.navigator())) else {
            return;
        };
        let wl = get(&nav, "wakeLock");
        let Some(p) = (!wl.is_undefined())
            .then(|| call(&wl, "request", &[JsValue::from_str("screen")]))
            .flatten()
            .and_then(|p| p.dyn_into::<Promise>().ok())
        else {
            set(&self.stats, "wakeLock", "unsupported");
            self.publish(|m| m.wake_lock = Some(false));
            return;
        };
        let this = self.clone();
        spawn_local(async move {
            match JsFuture::from(p).await {
                Ok(lock) if !this.stopped() => {
                    *this.wake_lock.borrow_mut() = Some(lock);
                    set(&this.stats, "wakeLock", "held");
                    this.publish(|m| m.wake_lock = Some(true));
                }
                Ok(lock) => {
                    let _ = call(&lock, "release", &[]);
                }
                Err(e) => {
                    // Refused while hidden is normal; it is asked for again
                    // on return.
                    let name = get(&e, "name").as_string().unwrap_or_default();
                    set(&this.stats, "wakeLock", name.as_str());
                    if !document_hidden() {
                        this.publish(|m| m.wake_lock = Some(false));
                    }
                }
            }
        });
    }

    /// FR-WEBST-7: battery from the Battery Status API, where it exists.
    fn spawn_battery(self: &Rc<Self>) {
        let Some(nav) = web_sys::window().map(|w| JsValue::from(w.navigator())) else {
            return;
        };
        let Some(p) = call(&nav, "getBattery", &[]).and_then(|p| p.dyn_into::<Promise>().ok())
        else {
            set(&self.stats, "battery", "unsupported");
            return;
        };
        let this = self.clone();
        let mut stop = self.stop.subscribe();
        spawn_local(async move {
            let Ok(manager) = JsFuture::from(p).await else {
                return;
            };
            *this.battery.borrow_mut() = Some(manager.clone());
            loop {
                let level = get(&manager, "level").as_f64();
                let charging = get(&manager, "charging").as_bool();
                if let Some(l) = level {
                    let percent = (l * 100.0).round().clamp(0.0, 100.0) as u8;
                    this.engine.set_battery(Some(percent), charging);
                    set(&this.stats, "battery", percent);
                }
                tokio::select! {
                    () = gloo_timers::future::sleep(BATTERY_POLL) => {}
                    _ = stop.wait_for(|s| *s) => break,
                }
            }
        });
    }
}

/// A playing, muted, inline `<video>` for `stream` (not in the document)
/// and the canvas frames are drawn into.
fn video_and_canvas(
    stream: &MediaStream,
) -> Option<(
    HtmlVideoElement,
    HtmlCanvasElement,
    CanvasRenderingContext2d,
)> {
    let doc = web_sys::window()?.document()?;
    let video: HtmlVideoElement = doc.create_element("video").ok()?.dyn_into().ok()?;
    video.set_muted(true);
    let _ = video.set_attribute("playsinline", "");
    video.set_src_object(Some(stream));
    let canvas: HtmlCanvasElement = doc.create_element("canvas").ok()?.dyn_into().ok()?;
    let opts = Object::new();
    set(&opts, "willReadFrequently", true);
    let ctx2d: CanvasRenderingContext2d = canvas
        .get_context_with_context_options("2d", &opts)
        .ok()??
        .dyn_into()
        .ok()?;
    Some((video, canvas, ctx2d))
}

/// Asks the browser to keep this origin's storage (security.md §7.1).
fn persist_storage() {
    let Some(nav) = web_sys::window().map(|w| JsValue::from(w.navigator())) else {
        return;
    };
    let storage = get(&nav, "storage");
    if storage.is_undefined() {
        return;
    }
    if let Some(p) = call(&storage, "persist", &[]).and_then(|p| p.dyn_into::<Promise>().ok()) {
        spawn_local(async move {
            let granted = JsFuture::from(p).await.ok().and_then(|v| v.as_bool());
            log(&format!(
                "Nestwatch station: persistent storage {}",
                if granted == Some(true) {
                    "granted"
                } else {
                    "not granted"
                }
            ));
        });
    }
}
