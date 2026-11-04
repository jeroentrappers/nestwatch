// SPDX-License-Identifier: AGPL-3.0-or-later
//! [`EngineBackend`]: the [`Backend`] over the nestwatch-core engines.
//!
//! The same code serves the native engines (desktop, tokio) and the
//! browser engines (wasm): the listener, and the station (relay-only in the
//! browser, product.md §3.10). Snapshots from the engines' `watch` channels are mapped to view
//! models ([`listener::map_listener`], [`station::map_station`]) and written
//! to the [`Ui`] signals by tasks on the UI executor; intents are forwarded
//! as plain engine calls. Everything the engines do not cover (preferences,
//! theme, audio unlock, timers, starting a station with its devices) comes
//! from the app through the [`Host`] port.

pub mod listener;
pub mod station;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use base64::Engine as _;
use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use nestwatch_core::Clock;
use nestwatch_core::listener::{
    AlertToggles, ListenSettings, ListenerEngine, ListenerError, ListenerEvent, Route, VideoRequest,
};
use nestwatch_core::{CameraFacing, ListenMode};

use crate::backend::{Backend, LocalFuture, Ui, race};
use crate::model::{
    AlertKind, Capabilities, Frame, Mode, NoticeKind, PairOutcome, Prefs, Theme, UiError,
};
use crate::model::{Gain, StationRun};

/// The app side of an [`EngineBackend`]: platform services the engines do
/// not provide.
pub trait Host {
    /// What the platform can do.
    fn capabilities(&self) -> Capabilities;
    /// Called once with the UI signals (e.g. to flag locked audio).
    fn attach(&self, _ui: Ui) {}
    /// Loads UI preferences.
    fn load_prefs(&self) -> Prefs;
    /// Stores UI preferences.
    fn save_prefs(&self, prefs: &Prefs);
    /// Applies the theme to the root element.
    fn apply_theme(&self, theme: Theme);
    /// UI timer.
    fn sleep(&self, ms: u64) -> LocalFuture<()>;
    /// Clipboard.
    fn copy_text(&self, _text: &str) -> bool {
        false
    }
    /// Audio unlock after a gesture (browser).
    fn unlock_audio(&self) {}
    /// Notification permission (browser).
    fn request_notifications(&self) {}
    /// A route tried for code pairing when the user gives none (the
    /// browser app's own station WebSocket).
    fn default_route(&self) -> Option<String> {
        None
    }
    /// Starts the station engine with its devices (microphone, speaker,
    /// camera, battery, sleep inhibition); the guard keeps them running.
    fn start_station(&self) -> LocalFuture<Result<station::RunningStation, String>>;
}

/// The [`Backend`] over nestwatch-core engines.
pub struct EngineBackend {
    host: Rc<dyn Host>,
    clock: Arc<dyn Clock>,
    listener: ListenerEngine,
    station: Rc<RefCell<Option<station::RunningStation>>>,
    station_gen: Rc<Cell<u64>>,
    ui: Rc<Cell<Option<Ui>>>,
    _not_send: std::marker::PhantomData<RefCell<()>>,
}

impl std::fmt::Debug for EngineBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineBackend")
            .field("listener", &self.listener)
            .finish_non_exhaustive()
    }
}

impl EngineBackend {
    /// A backend over a started listener engine. `clock` must be the clock
    /// the engines run on (pairing deadlines are on that clock).
    pub fn new(host: Rc<dyn Host>, clock: Arc<dyn Clock>, listener: ListenerEngine) -> Self {
        Self {
            host,
            clock,
            listener,
            station: Rc::new(RefCell::new(None)),
            station_gen: Rc::new(Cell::new(0)),
            ui: Rc::new(Cell::new(None)),
            _not_send: std::marker::PhantomData,
        }
    }

    /// The listener engine.
    pub fn listener(&self) -> &ListenerEngine {
        &self.listener
    }

    /// The running station engine, if any.
    pub fn station(&self) -> Option<nestwatch_core::station::StationEngine> {
        self.station.borrow().as_ref().map(|s| s.engine.clone())
    }

    fn settings_of(&self, id: &str) -> Result<nestwatch_core::listener::StationSettings, UiError> {
        self.listener
            .known_stations()
            .into_iter()
            .find(|k| k.id == id)
            .map(|k| k.settings)
            .ok_or(UiError::Other("unknown station".into()))
    }
}

/// Maps engine errors (pairing context decides what "invalid" means).
pub fn map_error(e: ListenerError, invalid: UiError) -> UiError {
    match e {
        ListenerError::Invalid(m) if m.contains("station id") => UiError::InvalidId,
        ListenerError::Invalid(_) => invalid,
        ListenerError::Unreachable(_) => UiError::Unreachable,
        ListenerError::Refused(_) => UiError::Refused,
        ListenerError::Rejected(_) => UiError::Rejected,
        ListenerError::TooManyStations => UiError::TooMany,
        ListenerError::NotConnected => UiError::NotConnected,
        other => UiError::Other(other.to_string()),
    }
}

fn plain(e: ListenerError) -> UiError {
    map_error(e, UiError::Other("invalid input".into()))
}

/// A `data:` URI for a JPEG frame (CSP allows `img-src data:`).
pub fn jpeg_data_uri(jpeg: &[u8]) -> String {
    format!(
        "data:image/jpeg;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(jpeg)
    )
}

fn alert_kind(a: nestwatch_core::listener::Alert) -> AlertKind {
    use nestwatch_core::listener::Alert;
    match a {
        Alert::SoundStarted => AlertKind::Sound,
        Alert::Unreachable => AlertKind::Unreachable,
        Alert::Reachable => AlertKind::Reachable,
        Alert::BatteryLow { percent } => AlertKind::BatteryLow(percent),
    }
}

fn spawn_listener_pumps(
    engine: ListenerEngine,
    clock: Arc<dyn Clock>,
    host: Rc<dyn Host>,
    mut ui: Ui,
    talk_possible: bool,
) {
    // State snapshots, plus a 1 s tick for "unreachable for 0:42".
    {
        let engine = engine.clone();
        let host = host.clone();
        spawn_forever(async move {
            let mut rx = engine.state();
            let mut tracker = listener::Tracker::default();
            loop {
                let state = rx.borrow_and_update().clone();
                let now = clock.now_ms();
                let model = listener::map_listener(&state, &mut tracker, now, talk_possible);
                {
                    let mut h = ui.history.write();
                    h.retain(|id, _| model.tile(id).is_some());
                    for t in &model.tiles {
                        if t.conn == crate::model::Conn::Live
                            && let Some(l) = t.level_dbfs
                        {
                            h.entry(t.id.clone()).or_default().push(now, l);
                        }
                    }
                }
                if *ui.listener.peek() != model {
                    ui.listener.set(model);
                }
                if let Some(Err(_)) = race(rx.changed(), host.sleep(1000)).await {
                    break;
                }
            }
        });
    }
    // Events: alerts and forgotten stations.
    {
        let engine = engine.clone();
        spawn_forever(async move {
            let mut ev = engine.events();
            loop {
                match ev.recv().await {
                    Ok(ListenerEvent::Alert { station_id, alert }) => {
                        let station = ui
                            .listener
                            .peek()
                            .tile(&station_id)
                            .map(|t| t.name.clone())
                            .unwrap_or_default();
                        ui.notify(NoticeKind::Alert {
                            station_id,
                            station,
                            kind: alert_kind(alert),
                        });
                    }
                    Ok(ListenerEvent::Forgotten { station_id }) => {
                        ui.frames.write().remove(&station_id);
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
        });
    }
    // Video frames.
    spawn_forever(async move {
        let mut rx = engine.video_frames();
        loop {
            match rx.recv().await {
                Ok(f) => {
                    let frame = Frame {
                        frame_id: f.frame_id,
                        src: jpeg_data_uri(&f.jpeg),
                        width: f.width,
                        height: f.height,
                    };
                    ui.frames.write().insert(f.station_id, frame);
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
    });
}

impl Backend for EngineBackend {
    fn capabilities(&self) -> Capabilities {
        self.host.capabilities()
    }

    fn attach(&self, ui: Ui) {
        self.ui.set(Some(ui));
        self.host.attach(ui);
        let talk = self.host.capabilities().talk;
        spawn_listener_pumps(
            self.listener.clone(),
            self.clock.clone(),
            self.host.clone(),
            ui,
            talk,
        );
    }

    fn load_prefs(&self) -> Prefs {
        self.host.load_prefs()
    }

    fn save_prefs(&self, prefs: &Prefs) {
        self.host.save_prefs(prefs);
    }

    fn apply_theme(&self, theme: Theme) {
        self.host.apply_theme(theme);
    }

    fn sleep(&self, ms: u64) -> LocalFuture<()> {
        self.host.sleep(ms)
    }

    fn copy_text(&self, text: &str) -> bool {
        self.host.copy_text(text)
    }

    fn unlock_audio(&self) {
        self.host.unlock_audio();
    }

    fn request_notifications(&self) {
        self.host.request_notifications();
    }

    fn unix_ms(&self) -> u64 {
        self.clock.unix_ms()
    }

    fn pair_invitation(&self, input: String) -> LocalFuture<Result<PairOutcome, UiError>> {
        let l = self.listener.clone();
        Box::pin(async move {
            let r = l
                .pair_invitation(input.trim())
                .await
                .map_err(|e| map_error(e, UiError::InvalidInvite))?;
            Ok(PairOutcome {
                station_id: r.station_id,
                name: r.name,
                verification: r.verification,
            })
        })
    }

    fn pair_code(
        &self,
        code: String,
        station_id: String,
        address: Option<String>,
    ) -> LocalFuture<Result<PairOutcome, UiError>> {
        let l = self.listener.clone();
        let default = self.host.default_route();
        Box::pin(async move {
            let mut routes = Vec::new();
            if let Some(a) = address.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
                routes.push(Route::parse(a).ok_or(UiError::InvalidAddress)?);
            }
            if let Some(r) = default.as_deref().and_then(Route::parse) {
                routes.push(r);
            }
            let r = l
                .pair_code(&code, station_id.trim(), routes)
                .await
                .map_err(|e| map_error(e, UiError::InvalidCode))?;
            Ok(PairOutcome {
                station_id: r.station_id,
                name: r.name,
                verification: r.verification,
            })
        })
    }

    fn forget(&self, station: &str) -> Result<(), UiError> {
        self.listener.forget(station).map_err(plain)
    }

    fn set_follow(&self, station: &str, follow: bool) -> Result<(), UiError> {
        if follow {
            self.listener.follow(station)
        } else {
            self.listener.unfollow(station)
        }
        .map_err(plain)
    }

    fn add_address(&self, station: &str, address: &str) -> Result<(), UiError> {
        let r = Route::parse(address).ok_or(UiError::InvalidAddress)?;
        self.listener.add_route(station, r).map_err(plain)
    }

    fn set_listen(
        &self,
        station: &str,
        mode: Mode,
        threshold_dbfs: f32,
        hold_ms: u32,
    ) -> Result<(), UiError> {
        let current = self.settings_of(station)?.listen;
        let listen = ListenSettings {
            mode: match mode {
                Mode::Continuous => ListenMode::Continuous,
                Mode::Activated => ListenMode::Activated,
            },
            threshold_dbfs,
            hold_ms,
            preroll_ms: current.preroll_ms,
        };
        self.listener.set_listen(station, listen).map_err(plain)
    }

    fn set_volume(&self, station: &str, volume: f32) -> Result<(), UiError> {
        self.listener.set_volume(station, volume).map_err(plain)
    }

    fn set_muted(&self, station: &str, muted: bool) -> Result<(), UiError> {
        self.listener.set_muted(station, muted).map_err(plain)
    }

    fn set_alerts(
        &self,
        station: &str,
        sound: bool,
        unreachable: bool,
        battery: bool,
    ) -> Result<(), UiError> {
        self.listener
            .set_alerts(
                station,
                AlertToggles {
                    sound,
                    unreachable,
                    battery,
                },
            )
            .map_err(plain)
    }

    fn set_solo(&self, station: Option<&str>) -> Result<(), UiError> {
        self.listener.set_solo(station).map_err(plain)
    }

    fn talk(&self, station: &str, on: bool) -> Result<(), UiError> {
        self.listener.talk(station, on).map_err(plain)
    }

    fn set_camera(&self, station: &str, on: bool) -> Result<(), UiError> {
        if on {
            self.listener
                .request_video(station, VideoRequest::default())
                .map_err(plain)
        } else {
            if let Some(mut ui) = self.ui.get() {
                ui.frames.write().remove(station);
            }
            self.listener.stop_video(station).map_err(plain)
        }
    }

    fn set_video_options(&self, station: &str, front: bool, night: bool) -> Result<(), UiError> {
        let facing = if front {
            CameraFacing::Front
        } else {
            CameraFacing::Back
        };
        self.listener
            .set_video_options(station, facing, night)
            .map_err(plain)
    }

    fn start_station(&self) -> LocalFuture<Result<(), UiError>> {
        let host = self.host.clone();
        let slot = self.station.clone();
        let gen_ = self.station_gen.clone();
        let clock = self.clock.clone();
        let ui = self.ui.get();
        Box::pin(async move {
            let Some(mut ui) = ui else {
                return Err(UiError::Other("not attached".into()));
            };
            if slot.borrow().is_some() {
                return Ok(());
            }
            ui.station.set(StationRun::Starting);
            match host.start_station().await {
                Ok(running) => {
                    let engine = running.engine.clone();
                    *slot.borrow_mut() = Some(running);
                    gen_.set(gen_.get() + 1);
                    station::spawn_station_pumps(engine, clock, host, ui, gen_.clone());
                    Ok(())
                }
                Err(e) => {
                    ui.station.set(StationRun::Failed(e.clone()));
                    Err(UiError::Other(e))
                }
            }
        })
    }

    fn stop_station(&self) -> LocalFuture<()> {
        let slot = self.station.clone();
        let gen_ = self.station_gen.clone();
        let ui = self.ui.get();
        Box::pin(async move {
            gen_.set(gen_.get() + 1);
            let running = slot.borrow_mut().take();
            if let Some(r) = running {
                r.engine.shutdown().await;
                drop(r);
            }
            if let Some(mut ui) = ui {
                ui.station.set(StationRun::Stopped);
                ui.just_paired.write().clear();
            }
        })
    }

    fn open_pairing(&self) -> LocalFuture<Result<(), UiError>> {
        let engine = self.station();
        let ui = self.ui.get();
        Box::pin(async move {
            let engine = engine.ok_or(UiError::Other("station not running".into()))?;
            if engine.snapshot().pairing.is_none()
                && let Some(mut ui) = ui
            {
                ui.just_paired.write().clear();
            }
            engine
                .open_pairing()
                .await
                .map(|_| ())
                .map_err(|e| UiError::Other(e.to_string()))
        })
    }

    fn close_pairing(&self) {
        if let Some(s) = self.station() {
            s.close_pairing();
        }
    }

    fn rename_station(&self, name: &str) -> Result<(), UiError> {
        let s = self
            .station()
            .ok_or(UiError::Other("station not running".into()))?;
        s.rename(name).map_err(|e| UiError::Other(e.to_string()))
    }

    fn revoke(&self, listener: &str) {
        if let Some(s) = self.station() {
            s.revoke(listener);
        }
    }

    fn set_camera_enabled(&self, on: bool) {
        if let Some(s) = self.station() {
            s.set_camera_enabled(on);
        }
    }

    fn set_talk_enabled(&self, on: bool) {
        if let Some(s) = self.station() {
            s.set_talk_enabled(on);
        }
    }

    fn set_gain(&self, gain: Gain) {
        if let Some(s) = self.station() {
            s.set_gain(station::gain_setting(gain));
        }
    }

    fn set_talkback_volume(&self, volume: f32) {
        if let Some(s) = self.station() {
            s.set_talkback_volume(volume);
        }
    }

    fn set_relay_enabled(&self, on: bool) -> Result<(), UiError> {
        let s = self
            .station()
            .ok_or(UiError::Other("station not running".into()))?;
        s.set_relay_enabled(on)
            .map_err(|e| UiError::Other(e.to_string()))
    }

    fn set_relay_server(&self, origin: &str) -> Result<(), UiError> {
        let s = self
            .station()
            .ok_or(UiError::Other("station not running".into()))?;
        s.set_relay_server(origin)
            .map_err(|_| UiError::Other(crate::strings::RELAY_SERVER_INVALID.into()))
    }

    fn reset_relay_identity(&self) -> Result<(), UiError> {
        let s = self
            .station()
            .ok_or(UiError::Other("station not running".into()))?;
        s.reset_relay_identity()
            .map_err(|e| UiError::Other(e.to_string()))
    }
}
