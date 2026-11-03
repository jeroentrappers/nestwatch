// SPDX-License-Identifier: AGPL-3.0-or-later
//! The listener side (product.md §3.2–§3.8).

use std::net::IpAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nestwatch_core::listener::{
    Alert, AlertToggles, ConnStatus, DiscoveredView, KnownStation, ListenSettings, ListenerConfig,
    ListenerEngine, ListenerEvent, ListenerState, NetPath, Route, StationView, TalkView,
    VideoFrameEvent, VideoRequest, VideoView,
};
use nestwatch_core::{SystemClock, VideoStateKind};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use crate::error::NestwatchError;
use crate::media;
use crate::runtime::{STATE_INTERVAL, on_rt, rt};
use crate::station::TxtEntry;
use crate::storage::{ForeignStorage, Storage};

/// Listen mode for one station (FR-AU-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum ListenMode {
    /// Audio always.
    Continuous,
    /// Audio while the room is louder than the threshold.
    Activated,
}

impl From<nestwatch_core::ListenMode> for ListenMode {
    fn from(m: nestwatch_core::ListenMode) -> Self {
        match m {
            nestwatch_core::ListenMode::Continuous => Self::Continuous,
            nestwatch_core::ListenMode::Activated => Self::Activated,
        }
    }
}

impl From<ListenMode> for nestwatch_core::ListenMode {
    fn from(m: ListenMode) -> Self {
        match m {
            ListenMode::Continuous => Self::Continuous,
            ListenMode::Activated => Self::Activated,
        }
    }
}

/// Station camera (FR-CAM-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CameraFacing {
    /// Front camera.
    Front,
    /// Back camera.
    Back,
}

impl From<nestwatch_core::CameraFacing> for CameraFacing {
    fn from(f: nestwatch_core::CameraFacing) -> Self {
        match f {
            nestwatch_core::CameraFacing::Front => Self::Front,
            nestwatch_core::CameraFacing::Back => Self::Back,
        }
    }
}

impl From<CameraFacing> for nestwatch_core::CameraFacing {
    fn from(f: CameraFacing) -> Self {
        match f {
            CameraFacing::Front => Self::Front,
            CameraFacing::Back => Self::Back,
        }
    }
}

/// How to start a listener.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ListenerOptions {
    /// Display name sent to stations until changed with
    /// [`Listener::rename`].
    pub name: String,
    /// Browse with the built-in mDNS browser. Apps that browse through the
    /// platform (Android NSD) pass `false` and report results with
    /// [`Listener::add_discovered`].
    pub discovery: bool,
}

/// Sound activation settings (FR-AU-2, FR-AU-4).
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct ListenSettingsInfo {
    /// Mode.
    pub mode: ListenMode,
    /// Threshold in dBFS.
    pub threshold_dbfs: f32,
    /// Hold time in ms.
    pub hold_ms: u32,
    /// Pre-roll in ms.
    pub preroll_ms: u32,
}

impl From<ListenSettings> for ListenSettingsInfo {
    fn from(l: ListenSettings) -> Self {
        Self {
            mode: l.mode.into(),
            threshold_dbfs: l.threshold_dbfs,
            hold_ms: l.hold_ms,
            preroll_ms: l.preroll_ms,
        }
    }
}

impl From<ListenSettingsInfo> for ListenSettings {
    fn from(l: ListenSettingsInfo) -> Self {
        Self {
            mode: l.mode.into(),
            threshold_dbfs: l.threshold_dbfs,
            hold_ms: l.hold_ms,
            preroll_ms: l.preroll_ms,
        }
    }
}

/// Which alerts a station raises (product.md §3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Record)]
pub struct AlertSettings {
    /// Sound started (FR-AL-1).
    pub sound: bool,
    /// Unreachable / reachable again (FR-AL-2).
    pub unreachable: bool,
    /// Low battery (FR-AL-3).
    pub battery: bool,
}

/// Per-station settings on this listener.
#[derive(Debug, Clone, Copy, PartialEq, uniffi::Record)]
pub struct StationPrefs {
    /// Sound activation.
    pub listen: ListenSettingsInfo,
    /// Volume, linear 0..1.
    pub volume: f32,
    /// Muted (alerts keep working).
    pub muted: bool,
    /// Alerts.
    pub alerts: AlertSettings,
    /// Followed.
    pub follow: bool,
}

impl From<nestwatch_core::listener::StationSettings> for StationPrefs {
    fn from(s: nestwatch_core::listener::StationSettings) -> Self {
        Self {
            listen: s.listen.into(),
            volume: s.volume,
            muted: s.muted,
            alerts: AlertSettings {
                sound: s.alerts.sound,
                unreachable: s.alerts.unreachable,
                battery: s.alerts.battery,
            },
            follow: s.follow,
        }
    }
}

/// Connection state of a station (FR-LS-3, FR-LS-5).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum Connection {
    /// Not followed.
    Idle,
    /// First attempt of this follow.
    Connecting {
        /// Attempt, from 1.
        attempt: u32,
        /// Routes tried so far in this attempt.
        tried: Vec<String>,
    },
    /// Live session.
    Live,
    /// Retrying.
    Reconnecting {
        /// Attempt, from 1.
        attempt: u32,
        /// Routes tried so far in this attempt.
        tried: Vec<String>,
    },
    /// The station revoked us; pair again.
    NeedsPairing,
}

/// Network path of a live session (FR-LS-3, rendezvous.md FR-RV-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum NetworkPath {
    /// Local network ("home network").
    Lan,
    /// User-operated VPN.
    Vpn,
    /// Hole-punched path across the internet ("direct (internet)").
    Direct,
    /// Through the relay server ("relayed").
    Relayed,
}

/// Talk-back state with one station (FR-TB-1).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum TalkState {
    /// Not talking.
    Idle,
    /// Requested.
    Requested,
    /// We talk.
    Granted,
    /// Someone else talks.
    Busy {
        /// Their name.
        by: Option<String>,
    },
    /// Talk-back off on the station.
    Disabled,
}

/// Camera view state with one station (FR-CAM-1).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum VideoState {
    /// No view requested.
    Off,
    /// Requested, no answer yet.
    Requested,
    /// Frames are coming.
    Streaming {
        /// Width.
        w: u32,
        /// Height.
        h: u32,
        /// Frame rate.
        fps: u32,
    },
    /// The station stopped the video.
    Stopped,
    /// No camera at the station right now.
    Unavailable,
    /// The station's camera permission is denied.
    Denied,
    /// Camera turned off on the station (FR-ST-8).
    Disabled,
}

/// One station tile (FR-LS-3).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct StationTile {
    /// Device ID.
    pub id: String,
    /// Name.
    pub name: String,
    /// Connection state.
    pub connection: Connection,
    /// Path of the live session.
    pub path: Option<NetworkPath>,
    /// Route of the live session.
    pub route: Option<String>,
    /// Last reported room level in dBFS.
    pub level_dbfs: Option<f32>,
    /// Audio flows to us now.
    pub open: bool,
    /// Battery percent.
    pub battery: Option<u8>,
    /// Charging.
    pub charging: Option<bool>,
    /// Listeners connected to the station.
    pub station_listeners: Option<u32>,
    /// The station's camera is sending to someone.
    pub camera_on: bool,
    /// Who talks to the station.
    pub talker: Option<String>,
    /// Our talk-back state.
    pub talk: TalkState,
    /// Our camera view state.
    pub video: VideoState,
    /// Last round-trip time.
    pub rtt_ms: Option<u64>,
    /// The station offers talk-back (caps).
    pub can_talk: bool,
    /// The station offers video (caps).
    pub can_video: bool,
    /// Settings.
    pub prefs: StationPrefs,
    /// Audio frames received in this follow.
    pub audio_frames: u64,
}

/// A station on the local network (protocol.md §3.4).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct NearbyStation {
    /// Device ID.
    pub id: String,
    /// Name.
    pub name: String,
    /// Pairing mode open.
    pub pairing: bool,
    /// Addresses, `host:port`.
    pub endpoints: Vec<String>,
    /// Already paired.
    pub known: bool,
}

/// Snapshot of the listener (architecture.md §5).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ListenerSnapshot {
    /// Our name.
    pub name: String,
    /// Our device ID.
    pub device_id: String,
    /// Known stations.
    pub stations: Vec<StationTile>,
    /// The solo station (FR-LS-2).
    pub solo: Option<String>,
    /// Stations on the local network.
    pub nearby: Vec<NearbyStation>,
}

/// A paired station as stored.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct KnownStationInfo {
    /// Device ID.
    pub id: String,
    /// Name.
    pub name: String,
    /// Known routes by last success.
    pub routes: Vec<String>,
    /// Settings.
    pub prefs: StationPrefs,
    /// Revoked by the station.
    pub needs_pairing: bool,
    /// Paired at (Unix ms).
    pub paired_unix_ms: u64,
}

/// Pairing result (FR-PAIR-5).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PairResult {
    /// Station device ID.
    pub station_id: String,
    /// Station name.
    pub name: String,
    /// Verification number to compare with the station's.
    pub verification: String,
}

/// An alert (product.md §3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum AlertKind {
    /// Sound started (FR-AL-1).
    SoundStarted,
    /// Unreachable for over 20 s (FR-AL-2).
    Unreachable,
    /// Reachable again (FR-AL-2).
    Reachable,
    /// Battery low (FR-AL-3).
    BatteryLow {
        /// Percent.
        percent: u8,
    },
}

/// Something that happened on the listener.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum ListenerEventInfo {
    /// Pairing finished.
    Paired {
        /// Station.
        station_id: String,
        /// Name.
        name: String,
        /// Verification number.
        verification: String,
    },
    /// A session became live.
    Connected {
        /// Station.
        station_id: String,
    },
    /// A live session ended.
    Disconnected {
        /// Station.
        station_id: String,
        /// Diagnostics.
        reason: String,
    },
    /// The station revoked us.
    NeedsPairing {
        /// Station.
        station_id: String,
    },
    /// Raise an alert, regardless of mute (FR-AL-4).
    Alert {
        /// Station.
        station_id: String,
        /// Which.
        alert: AlertKind,
    },
    /// Talk-back state changed.
    Talk {
        /// Station.
        station_id: String,
        /// State.
        state: TalkState,
    },
    /// Start the microphone and feed [`Listener::push_talk_capture`].
    MicrophoneWanted {
        /// Station.
        station_id: String,
    },
    /// Stop the microphone.
    MicrophoneNotWanted,
    /// A station was forgotten.
    Forgotten {
        /// Station.
        station_id: String,
    },
}

/// A complete JPEG video frame from a station.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct VideoFrame {
    /// Station.
    pub station_id: String,
    /// Frame counter.
    pub frame_id: u32,
    /// Width.
    pub width: u16,
    /// Height.
    pub height: u16,
    /// JPEG bytes.
    pub jpeg: Vec<u8>,
}

/// A station found by the platform's DNS-SD service (Android NSD).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct DiscoveredService {
    /// Unique service name for updates and removal (e.g. instance name
    /// plus type).
    pub fullname: String,
    /// Instance name (display name).
    pub name: String,
    /// Host name, if known.
    pub host: String,
    /// Resolved IP addresses.
    pub addrs: Vec<String>,
    /// TCP port.
    pub port: u16,
    /// TXT entries.
    pub txt: Vec<TxtEntry>,
}

/// Receives listener state, events and video. Called on engine threads.
#[uniffi::export(foreign)]
pub trait ListenerObserver: Send + Sync {
    /// A new snapshot (at most 10 per second).
    fn on_state(&self, state: ListenerSnapshot);
    /// An event.
    fn on_event(&self, event: ListenerEventInfo);
    /// A video frame of a station whose camera view is open.
    fn on_video_frame(&self, frame: VideoFrame);
}

/// A running listener. Stop it with [`Listener::stop`].
#[derive(uniffi::Object)]
pub struct Listener {
    engine: ListenerEngine,
    pumps: Mutex<Vec<JoinHandle<()>>>,
    stopped: AtomicBool,
}

impl std::fmt::Debug for Listener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Listener")
            .field("engine", &self.engine)
            .finish_non_exhaustive()
    }
}

fn routes(list: &[String]) -> Result<Vec<Route>, NestwatchError> {
    list.iter()
        .map(|r| {
            Route::parse(r).ok_or_else(|| NestwatchError::Invalid {
                reason: format!("address {r:?}"),
            })
        })
        .collect()
}

#[uniffi::export]
impl Listener {
    /// Loads or creates the device key and known stations from `storage`
    /// and starts following the stations marked as followed.
    #[uniffi::constructor]
    pub fn start(
        options: ListenerOptions,
        storage: Arc<dyn Storage>,
        observer: Arc<dyn ListenerObserver>,
    ) -> Result<Arc<Self>, NestwatchError> {
        let _enter = rt().enter();
        let mut cfg = ListenerConfig::new(options.name);
        cfg.discovery = options.discovery;
        let engine = ListenerEngine::start(
            cfg,
            Arc::new(ForeignStorage(storage)),
            Arc::new(SystemClock::new()),
        )?;
        let pumps = spawn_pumps(&engine, observer);
        Ok(Arc::new(Self {
            engine,
            pumps: Mutex::new(pumps),
            stopped: AtomicBool::new(false),
        }))
    }

    /// Our device ID.
    pub fn device_id(&self) -> String {
        self.engine.device_id().to_owned()
    }

    /// The current snapshot.
    pub fn snapshot(&self) -> ListenerSnapshot {
        snapshot(&self.engine.snapshot())
    }

    /// Paired stations.
    pub fn known_stations(&self) -> Vec<KnownStationInfo> {
        self.engine.known_stations().iter().map(known).collect()
    }

    /// Renames this listener.
    pub fn rename(&self, name: String) -> Result<(), NestwatchError> {
        Ok(self.engine.rename(&name)?)
    }

    /// Pairs from an invitation (QR code, link or station web address),
    /// trying `routes` (`host:port` or `ws(s)://…`) before the
    /// invitation's endpoints (FR-PAIR-1).
    pub async fn pair_invitation(
        &self,
        input: String,
        routes: Vec<String>,
    ) -> Result<PairResult, NestwatchError> {
        let routes = self::routes(&routes)?;
        let engine = self.engine.clone();
        let r = on_rt(async move { engine.pair_invitation_via(&input, routes).await }).await?;
        Ok(PairResult {
            station_id: r.station_id,
            name: r.name,
            verification: r.verification,
        })
    }

    /// Pairs with a 10-character code (FR-PAIR-2). `station_id` comes
    /// from a nearby station (or is typed); with no `routes`, its
    /// discovered addresses are used.
    pub async fn pair_code(
        &self,
        code: String,
        station_id: String,
        routes: Vec<String>,
    ) -> Result<PairResult, NestwatchError> {
        let routes = self::routes(&routes)?;
        let engine = self.engine.clone();
        let r = on_rt(async move { engine.pair_code(&code, &station_id, routes).await }).await?;
        Ok(PairResult {
            station_id: r.station_id,
            name: r.name,
            verification: r.verification,
        })
    }

    /// Follows a known station (FR-LS-1).
    pub fn follow(&self, station_id: String) -> Result<(), NestwatchError> {
        let _enter = rt().enter();
        Ok(self.engine.follow(&station_id)?)
    }

    /// Stops following a station.
    pub fn unfollow(&self, station_id: String) -> Result<(), NestwatchError> {
        Ok(self.engine.unfollow(&station_id)?)
    }

    /// Forgets a station, telling it if reachable (FR-PAIR-6).
    pub fn forget(&self, station_id: String) -> Result<(), NestwatchError> {
        Ok(self.engine.forget(&station_id)?)
    }

    /// Adds a manually entered address (FR-NET-2).
    pub fn add_route(&self, station_id: String, route: String) -> Result<(), NestwatchError> {
        let route = Route::parse(&route).ok_or_else(|| NestwatchError::Invalid {
            reason: "address".into(),
        })?;
        Ok(self.engine.add_route(&station_id, route)?)
    }

    /// Sets mode, threshold, hold time and pre-roll (FR-AU-2).
    pub fn set_listen(
        &self,
        station_id: String,
        listen: ListenSettingsInfo,
    ) -> Result<(), NestwatchError> {
        Ok(self.engine.set_listen(&station_id, listen.into())?)
    }

    /// Volume 0..1 (FR-AU-6).
    pub fn set_volume(&self, station_id: String, volume: f32) -> Result<(), NestwatchError> {
        Ok(self.engine.set_volume(&station_id, volume)?)
    }

    /// Mute; alerts keep working (FR-AU-6).
    pub fn set_muted(&self, station_id: String, muted: bool) -> Result<(), NestwatchError> {
        Ok(self.engine.set_muted(&station_id, muted)?)
    }

    /// Alert toggles.
    pub fn set_alerts(
        &self,
        station_id: String,
        alerts: AlertSettings,
    ) -> Result<(), NestwatchError> {
        Ok(self.engine.set_alerts(
            &station_id,
            AlertToggles {
                sound: alerts.sound,
                unreachable: alerts.unreachable,
                battery: alerts.battery,
            },
        )?)
    }

    /// Hears only one station, or all with `None` (FR-LS-2).
    pub fn set_solo(&self, station_id: Option<String>) -> Result<(), NestwatchError> {
        Ok(self.engine.set_solo(station_id.as_deref())?)
    }

    /// Push-to-talk (FR-TB-1). While granted, feed
    /// [`push_talk_capture`](Self::push_talk_capture).
    pub fn talk(&self, station_id: String, on: bool) -> Result<(), NestwatchError> {
        Ok(self.engine.talk(&station_id, on)?)
    }

    /// Feeds interleaved 16-bit little-endian microphone PCM for talk-back.
    pub fn push_talk_capture(&self, pcm: Vec<u8>, rate: u32, channels: u16) {
        self.engine
            .push_talk_capture(&media::i16_from_le(&pcm), rate, channels);
    }

    /// Opens the camera view (FR-CAM-1); frames arrive through
    /// [`ListenerObserver::on_video_frame`].
    pub fn request_video(
        &self,
        station_id: String,
        max_w: u32,
        max_h: u32,
        max_fps: u32,
    ) -> Result<(), NestwatchError> {
        Ok(self.engine.request_video(
            &station_id,
            VideoRequest {
                max_w,
                max_h,
                max_fps,
            },
        )?)
    }

    /// Closes the camera view.
    pub fn stop_video(&self, station_id: String) -> Result<(), NestwatchError> {
        Ok(self.engine.stop_video(&station_id)?)
    }

    /// Chooses the station's camera and night image (FR-CAM-4).
    pub fn set_video_options(
        &self,
        station_id: String,
        facing: CameraFacing,
        night: bool,
    ) -> Result<(), NestwatchError> {
        Ok(self
            .engine
            .set_video_options(&station_id, facing.into(), night)?)
    }

    /// Pulls `frames` samples of the 16 kHz mono mix of all audible
    /// stations as 16-bit little-endian PCM (FR-LS-2). Call at the output
    /// device's pace.
    pub fn pull_playback(&self, frames: u32) -> Vec<u8> {
        let mut out = vec![0i16; frames.min(48_000) as usize];
        self.engine.pull_playback(&mut out);
        media::i16_to_le(&out)
    }

    /// Tells the engine the device's network changed (Android
    /// `ConnectivityManager` callback): reconnects retry at once and
    /// relayed sessions try a direct path again (rendezvous.md §5.2).
    pub fn network_changed(&self) {
        self.engine.network_changed();
    }

    /// Reports a station found by the platform's DNS-SD browser.
    pub fn add_discovered(&self, service: DiscoveredService) -> Result<(), NestwatchError> {
        let txt = nestwatch_proto::StationTxt::from_pairs(
            service
                .txt
                .iter()
                .map(|e| (e.key.as_str(), e.value.as_str())),
        )
        .map_err(|e| NestwatchError::Invalid {
            reason: e.to_string(),
        })?;
        let mut addrs: Vec<IpAddr> = service
            .addrs
            .iter()
            .filter_map(|a| a.parse().ok())
            .filter(
                |a: &IpAddr| !matches!(a, IpAddr::V6(v) if (v.segments()[0] & 0xffc0) == 0xfe80),
            )
            .collect();
        addrs.sort_by_key(|a| (a.is_ipv6(), *a));
        addrs.dedup();
        if addrs.is_empty() || service.port == 0 {
            return Err(NestwatchError::Invalid {
                reason: "no usable address".into(),
            });
        }
        self.engine
            .add_discovered(nestwatch_net::mdns::DiscoveredStation {
                fullname: service.fullname,
                name: service.name,
                host: service.host,
                addrs,
                port: service.port,
                txt,
            });
        Ok(())
    }

    /// Removes a station reported with
    /// [`add_discovered`](Self::add_discovered).
    pub fn remove_discovered(&self, fullname: String) {
        self.engine.remove_discovered(&fullname);
    }

    /// Closes all sessions and stops reconnecting. Idempotent.
    pub async fn stop(&self) {
        if self.stopped.swap(true, Ordering::SeqCst) {
            return;
        }
        let engine = self.engine.clone();
        on_rt(async move { engine.shutdown().await }).await;
        for p in self.pumps.lock().expect("poisoned").drain(..) {
            p.abort();
        }
    }
}

impl Drop for Listener {
    fn drop(&mut self) {
        for p in self.pumps.lock().expect("poisoned").drain(..) {
            p.abort();
        }
        if !self.stopped.swap(true, Ordering::SeqCst) {
            let engine = self.engine.clone();
            rt().spawn(async move { engine.shutdown().await });
        }
    }
}

fn spawn_pumps(
    engine: &ListenerEngine,
    observer: Arc<dyn ListenerObserver>,
) -> Vec<JoinHandle<()>> {
    let state = {
        let mut rx = engine.state();
        let observer = observer.clone();
        rt().spawn(async move {
            loop {
                let s = rx.borrow_and_update().clone();
                observer.on_state(snapshot(&s));
                tokio::time::sleep(STATE_INTERVAL).await;
                if rx.changed().await.is_err() {
                    break;
                }
            }
        })
    };
    let events = {
        let mut rx = engine.events();
        let observer = observer.clone();
        rt().spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(e) => observer.on_event(event(e)),
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                }
            }
        })
    };
    let video = {
        let mut rx = engine.video_frames();
        rt().spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(f) => observer.on_video_frame(frame(f)),
                    // Late frames are dropped; the next one replaces them.
                    Err(RecvError::Lagged(_)) => {}
                    Err(RecvError::Closed) => break,
                }
            }
        })
    };
    vec![state, events, video]
}

fn talk(t: &TalkView) -> TalkState {
    match t {
        TalkView::Idle => TalkState::Idle,
        TalkView::Requested => TalkState::Requested,
        TalkView::Granted => TalkState::Granted,
        TalkView::Busy { by } => TalkState::Busy { by: by.clone() },
        TalkView::Disabled => TalkState::Disabled,
    }
}

fn video(v: &VideoView) -> VideoState {
    match v {
        VideoView::Off => VideoState::Off,
        VideoView::Requested => VideoState::Requested,
        VideoView::Streaming { w, h, fps } => VideoState::Streaming {
            w: *w,
            h: *h,
            fps: *fps,
        },
        VideoView::State(k) => match k {
            VideoStateKind::Streaming => VideoState::Requested,
            VideoStateKind::Stopped => VideoState::Stopped,
            VideoStateKind::Unavailable => VideoState::Unavailable,
            VideoStateKind::Denied => VideoState::Denied,
            VideoStateKind::Disabled => VideoState::Disabled,
        },
    }
}

fn connection(c: &ConnStatus) -> Connection {
    match c {
        ConnStatus::Idle => Connection::Idle,
        ConnStatus::Connecting { attempt, tried } => Connection::Connecting {
            attempt: *attempt,
            tried: tried.clone(),
        },
        ConnStatus::Live => Connection::Live,
        ConnStatus::Reconnecting { attempt, tried } => Connection::Reconnecting {
            attempt: *attempt,
            tried: tried.clone(),
        },
        ConnStatus::NeedsPairing => Connection::NeedsPairing,
    }
}

fn tile(v: &StationView) -> StationTile {
    use nestwatch_proto::control::caps;
    StationTile {
        id: v.id.clone(),
        name: v.name.clone(),
        connection: connection(&v.status),
        path: v.path.map(|p| match p {
            NetPath::HomeNetwork => NetworkPath::Lan,
            NetPath::Vpn => NetworkPath::Vpn,
            NetPath::Direct => NetworkPath::Direct,
            NetPath::Relayed => NetworkPath::Relayed,
        }),
        route: v.route.clone(),
        level_dbfs: v.level_dbfs,
        open: v.open,
        battery: v.battery,
        charging: v.charging,
        station_listeners: v.station_listeners,
        camera_on: v.camera_on,
        talker: v.talker.clone(),
        talk: talk(&v.talk),
        video: video(&v.video),
        rtt_ms: v.rtt_ms,
        can_talk: v.caps.iter().any(|c| c == caps::TALK),
        can_video: v.caps.iter().any(|c| c == caps::VIDEO),
        prefs: v.settings.into(),
        audio_frames: v.audio_frames,
    }
}

fn nearby(d: &DiscoveredView) -> NearbyStation {
    NearbyStation {
        id: d.id.clone(),
        name: d.name.clone(),
        pairing: d.pairing,
        endpoints: d.endpoints.iter().map(ToString::to_string).collect(),
        known: d.known,
    }
}

fn known(k: &KnownStation) -> KnownStationInfo {
    KnownStationInfo {
        id: k.id.clone(),
        name: k.name.clone(),
        routes: k
            .routes_by_success()
            .iter()
            .map(ToString::to_string)
            .collect(),
        prefs: k.settings.into(),
        needs_pairing: k.needs_pairing,
        paired_unix_ms: k.paired_unix_ms,
    }
}

pub(crate) fn snapshot(s: &ListenerState) -> ListenerSnapshot {
    ListenerSnapshot {
        name: s.name.clone(),
        device_id: s.device_id.clone(),
        stations: s.stations.iter().map(tile).collect(),
        solo: s.solo.clone(),
        nearby: s.discovered.iter().map(nearby).collect(),
    }
}

fn event(e: ListenerEvent) -> ListenerEventInfo {
    match e {
        ListenerEvent::Paired {
            station_id,
            name,
            verification,
        } => ListenerEventInfo::Paired {
            station_id,
            name,
            verification,
        },
        ListenerEvent::Connected { station_id } => ListenerEventInfo::Connected { station_id },
        ListenerEvent::Disconnected { station_id, reason } => {
            ListenerEventInfo::Disconnected { station_id, reason }
        }
        ListenerEvent::NeedsPairing { station_id } => {
            ListenerEventInfo::NeedsPairing { station_id }
        }
        ListenerEvent::Alert { station_id, alert } => ListenerEventInfo::Alert {
            station_id,
            alert: match alert {
                Alert::SoundStarted => AlertKind::SoundStarted,
                Alert::Unreachable => AlertKind::Unreachable,
                Alert::Reachable => AlertKind::Reachable,
                Alert::BatteryLow { percent } => AlertKind::BatteryLow { percent },
            },
        },
        ListenerEvent::Talk { station_id, state } => ListenerEventInfo::Talk {
            station_id,
            state: talk(&state),
        },
        ListenerEvent::MicrophoneWanted { station_id } => {
            ListenerEventInfo::MicrophoneWanted { station_id }
        }
        ListenerEvent::MicrophoneNotWanted => ListenerEventInfo::MicrophoneNotWanted,
        ListenerEvent::Forgotten { station_id } => ListenerEventInfo::Forgotten { station_id },
    }
}

fn frame(f: VideoFrameEvent) -> VideoFrame {
    VideoFrame {
        station_id: f.station_id,
        frame_id: f.frame_id,
        width: f.width,
        height: f.height,
        jpeg: f.jpeg.as_ref().clone(),
    }
}
