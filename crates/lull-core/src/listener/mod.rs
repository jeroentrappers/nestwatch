// SPDX-License-Identifier: AGPL-3.0-or-later
//! The listener engine (product.md §3.2, §3.3, §3.4, §3.5, §3.8).
//!
//! A [`ListenerEngine`] pairs with stations, follows up to 8 of them with
//! an independent reconnect loop each, mixes their audio into one output
//! ([`pull_playback`](ListenerEngine::pull_playback)), sends talk-back,
//! delivers video frames and raises alerts.
//!
//! It runs on tokio (`native`) or in the browser (`web`, WebSocket only).

mod alerts;
mod follow;
mod store;

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use lull_crypto::DeviceKey;
use lull_media::capture::{CaptureChain, GainSetting, GainStep};
use lull_media::codec::AudioEncoder;
use lull_media::jitter::JitterBuffer;
use lull_media::mixer::{Mixer, StationHandle};
use lull_media::video::RgbaImage;
use lull_media::{CodecId, FRAME_TICKS, PcmFrame};
use lull_proto::control::{CameraFacing, CloseReason, Control, Hello, Role, VideoStart, caps};
use lull_proto::record::{AudioFlags, AudioHeader, AudioStream, VIDEO_CODEC_JPEG};
use thiserror::Error;
use tokio::sync::{Notify, broadcast, mpsc, watch};

use crate::clock::Clock;
use crate::persist;
use crate::rt;
use crate::storage::{Storage, StorageError};
use crate::types::{TalkStatus, VideoStateKind, clamp_name};

pub use alerts::Alert;
pub use store::{
    AlertToggles, KEY_LISTENER, KEY_STATIONS, KnownEndpoint, KnownStation, ListenSettings, Route,
    StationSettings,
};

use alerts::{AlertLimits, AlertState};
use store::ListenerRecord;

/// Listener configuration. Tunables default to the spec values.
#[derive(Debug, Clone)]
pub struct ListenerConfig {
    /// Display name sent in `hello` until changed.
    pub name: String,
    /// Browse mDNS for stations (native only, protocol.md §3.4).
    pub discovery: bool,
    /// Time allowed to open one route.
    pub dial_timeout: Duration,
    /// First reconnect delay.
    pub backoff_min: Duration,
    /// Longest reconnect delay.
    pub backoff_max: Duration,
    /// Unreachable alert delay (FR-AL-2: 20 s).
    pub unreachable_alert_after: Duration,
    /// Minimum time between sound alerts per station (FR-AL-1: 30 s).
    pub sound_alert_interval: Duration,
    /// Battery alert limit in percent (FR-AL-3: 15).
    pub battery_alert_percent: u8,
    /// Also deliver video frames decoded to RGBA.
    pub decode_video: bool,
    /// Stations followed at once at most (FR-LS-1: 8).
    pub max_followed: usize,
    /// WebSocket URL tried for every station (the browser app's own
    /// station, `ws(s)://<page host>/lull/1`).
    pub default_websocket: Option<String>,
}

impl ListenerConfig {
    /// Spec defaults.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            discovery: cfg!(feature = "native"),
            dial_timeout: Duration::from_secs(3),
            backoff_min: Duration::from_millis(500),
            backoff_max: Duration::from_secs(10),
            unreachable_alert_after: Duration::from_secs(20),
            sound_alert_interval: Duration::from_secs(30),
            battery_alert_percent: 15,
            decode_video: false,
            max_followed: 8,
            default_websocket: None,
        }
    }
}

/// Connection state of one station (FR-LS-3, FR-LS-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnStatus {
    /// Not followed.
    Idle,
    /// First connection attempt of this follow.
    Connecting {
        /// Attempt number, from 1.
        attempt: u32,
        /// Routes tried in this attempt so far.
        tried: Vec<String>,
    },
    /// A session is set up.
    Live,
    /// Trying again after a failure or a lost session.
    Reconnecting {
        /// Attempt number, from 1.
        attempt: u32,
        /// Routes tried in this attempt so far.
        tried: Vec<String>,
    },
    /// The station revoked this listener; pair again (protocol.md §7.11).
    NeedsPairing,
}

/// Network path of a live session (FR-LS-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NetPath {
    /// Local network.
    Lan,
    /// User-operated VPN (protocol.md §3.5 ranges, interfaces or names).
    Vpn,
}

/// Talk-back state with one station (FR-TB-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TalkView {
    /// Not talking.
    Idle,
    /// `talk-request` sent.
    Requested,
    /// This listener talks.
    Granted,
    /// Someone else talks.
    Busy {
        /// Their name.
        by: Option<String>,
    },
    /// Talk-back is off on the station (FR-ST-8).
    Disabled,
}

/// Camera view state with one station (FR-CAM-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoView {
    /// No view requested.
    Off,
    /// `video-start` sent, no answer yet.
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
    /// The station reported a non-streaming state.
    State(VideoStateKind),
}

/// A camera request (media.md §5: default 640×480 at 10 fps).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoRequest {
    /// Largest width.
    pub max_w: u32,
    /// Largest height.
    pub max_h: u32,
    /// Highest frame rate.
    pub max_fps: u32,
}

impl Default for VideoRequest {
    fn default() -> Self {
        Self {
            max_w: 640,
            max_h: 480,
            max_fps: 10,
        }
    }
}

/// One station as shown on its tile (FR-LS-3).
#[derive(Debug, Clone, PartialEq)]
pub struct StationView {
    /// Device ID.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Connection state.
    pub status: ConnStatus,
    /// Path of the live session.
    pub path: Option<NetPath>,
    /// Route of the live session.
    pub route: Option<String>,
    /// Last reported room level (`level.dbfs`).
    pub level_dbfs: Option<f32>,
    /// Whether audio flows to us (`level.open` / `activity`).
    pub open: bool,
    /// Battery percent.
    pub battery: Option<u8>,
    /// Charging.
    pub charging: Option<bool>,
    /// Listeners connected to the station.
    pub station_listeners: Option<u32>,
    /// The station's camera is sending to someone.
    pub camera_on: bool,
    /// Name of whoever talks to the station.
    pub talker: Option<String>,
    /// Our talk-back state.
    pub talk: TalkView,
    /// Our camera view state.
    pub video: VideoView,
    /// Last round-trip time.
    pub rtt_ms: Option<u64>,
    /// Station capabilities from its `hello`.
    pub caps: Vec<String>,
    /// Settings.
    pub settings: StationSettings,
    /// AUDIO frames received in this follow (diagnostics).
    pub audio_frames: u64,
    /// Of those, frames flagged pre-roll.
    pub preroll_frames: u64,
}

/// A station found by mDNS (protocol.md §3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredView {
    /// Device ID from TXT `id`.
    pub id: String,
    /// Instance name.
    pub name: String,
    /// Pairing mode open (TXT `pair`).
    pub pairing: bool,
    /// Its addresses with the TCP port.
    pub endpoints: Vec<lull_proto::Endpoint>,
    /// Whether it is a known station.
    pub known: bool,
}

/// Snapshot of the listener for its UI (architecture.md §5).
#[derive(Debug, Clone, PartialEq)]
pub struct ListenerState {
    /// Our display name.
    pub name: String,
    /// Our device ID.
    pub device_id: String,
    /// Known stations, followed or not.
    pub stations: Vec<StationView>,
    /// The solo station, if any (FR-LS-2).
    pub solo: Option<String>,
    /// Stations on the local network.
    pub discovered: Vec<DiscoveredView>,
}

/// A complete video frame from a station (protocol.md §6.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrameEvent {
    /// Station device ID.
    pub station_id: String,
    /// Frame counter.
    pub frame_id: u32,
    /// Width.
    pub width: u16,
    /// Height.
    pub height: u16,
    /// The JPEG.
    pub jpeg: Arc<Vec<u8>>,
    /// Decoded pixels, with [`ListenerConfig::decode_video`].
    pub rgba: Option<Arc<RgbaImage>>,
}

/// Something that happened on the listener.
#[derive(Debug, Clone, PartialEq)]
pub enum ListenerEvent {
    /// Pairing finished; show `verification` for comparison (FR-PAIR-5).
    Paired {
        /// Station device ID.
        station_id: String,
        /// Its name.
        name: String,
        /// Verification number, `042 917`.
        verification: String,
    },
    /// A session became live.
    Connected {
        /// Station device ID.
        station_id: String,
    },
    /// A live session ended.
    Disconnected {
        /// Station device ID.
        station_id: String,
        /// Why, for diagnostics.
        reason: String,
    },
    /// The station revoked us.
    NeedsPairing {
        /// Station device ID.
        station_id: String,
    },
    /// An alert (product.md §3.5). Raised regardless of mute (FR-AL-4).
    Alert {
        /// Station device ID.
        station_id: String,
        /// Which alert.
        alert: Alert,
    },
    /// Talk-back state changed.
    Talk {
        /// Station device ID.
        station_id: String,
        /// New state.
        state: TalkView,
    },
    /// Start capturing the microphone for talk-back; feed
    /// [`ListenerEngine::push_talk_capture`].
    MicrophoneWanted {
        /// Station device ID.
        station_id: String,
    },
    /// Stop capturing the microphone.
    MicrophoneNotWanted,
    /// A station was forgotten.
    Forgotten {
        /// Station device ID.
        station_id: String,
    },
}

/// Pairing results (FR-PAIR-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairResult {
    /// Station device ID.
    pub station_id: String,
    /// Station name.
    pub name: String,
    /// Verification number to compare with the station's.
    pub verification: String,
}

/// Errors of listener operations.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ListenerError {
    /// Storage failed.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// Input could not be parsed (invitation, code, ID, route).
    #[error("invalid input: {0}")]
    Invalid(String),
    /// No route reached the station.
    #[error("station unreachable: {0}")]
    Unreachable(String),
    /// The station refused: pairing closed, busy or rate limited.
    #[error("station refused: {0}")]
    Refused(String),
    /// The handshake failed: wrong code, expired invitation or a different
    /// station (protocol.md §5).
    #[error("pairing rejected: {0}")]
    Rejected(String),
    /// No such station.
    #[error("unknown station")]
    UnknownStation,
    /// Already following the maximum number of stations (FR-LS-1).
    #[error("already following the maximum number of stations")]
    TooManyStations,
    /// The station is not connected.
    #[error("station not connected")]
    NotConnected,
    /// The engine stopped.
    #[error("listener stopped")]
    Stopped,
}

/// Commands from the engine to a session task.
#[derive(Debug)]
pub(crate) enum Cmd {
    Control(Control),
    TalkAudio(AudioHeader, Vec<u8>),
    Close(CloseReason),
}

/// Runtime state of one known station.
pub(crate) struct Rt {
    pub known: KnownStation,
    pub handle: StationHandle,
    pub following: bool,
    pub generation: u64,
    pub wake: Arc<Notify>,
    pub status: ConnStatus,
    pub path: Option<NetPath>,
    pub route: Option<String>,
    pub cmd: Option<(u64, mpsc::UnboundedSender<Cmd>)>,
    pub jitter: Option<JitterBuffer>,
    pub codec: Option<CodecId>,
    pub level: Option<f32>,
    pub open: bool,
    pub battery: Option<u8>,
    pub charging: Option<bool>,
    pub station_listeners: Option<u32>,
    pub camera_on: bool,
    pub talker: Option<String>,
    pub talk: TalkView,
    pub video: VideoView,
    pub video_req: Option<VideoRequest>,
    pub video_options: Option<(CameraFacing, bool)>,
    pub rtt_ms: Option<u64>,
    pub caps: Vec<String>,
    pub audio_frames: u64,
    pub preroll_frames: u64,
    pub alerts: AlertState,
}

impl Rt {
    fn new(known: KnownStation, handle: StationHandle) -> Self {
        Self {
            known,
            handle,
            following: false,
            generation: 0,
            wake: Arc::new(Notify::new()),
            status: ConnStatus::Idle,
            path: None,
            route: None,
            cmd: None,
            jitter: None,
            codec: None,
            level: None,
            open: false,
            battery: None,
            charging: None,
            station_listeners: None,
            camera_on: false,
            talker: None,
            talk: TalkView::Idle,
            video: VideoView::Off,
            video_req: None,
            video_options: None,
            rtt_ms: None,
            caps: Vec::new(),
            audio_frames: 0,
            preroll_frames: 0,
            alerts: AlertState::default(),
        }
    }

    pub fn send(&self, cmd: Cmd) -> bool {
        self.cmd
            .as_ref()
            .is_some_and(|(_, tx)| tx.send(cmd).is_ok())
    }

    fn view(&self) -> StationView {
        StationView {
            id: self.known.id.clone(),
            name: self.known.name.clone(),
            status: self.status.clone(),
            path: self.path,
            route: self.route.clone(),
            level_dbfs: self.level,
            open: self.open,
            battery: self.battery,
            charging: self.charging,
            station_listeners: self.station_listeners,
            camera_on: self.camera_on,
            talker: self.talker.clone(),
            talk: self.talk.clone(),
            video: self.video.clone(),
            rtt_ms: self.rtt_ms,
            caps: self.caps.clone(),
            settings: self.known.settings,
            audio_frames: self.audio_frames,
            preroll_frames: self.preroll_frames,
        }
    }
}

/// Talk-back capture for the station we hold talk-back with (media.md §4.3).
pub(crate) struct TalkCapture {
    pub station: String,
    chain: Option<(u32, u16, CaptureChain)>,
    encoder: Option<Box<dyn AudioEncoder>>,
    seq: u32,
}

/// Mutable state behind the engine's lock.
pub(crate) struct Inner {
    pub running: bool,
    pub name: String,
    pub stations: BTreeMap<String, Rt>,
    pub mixer: Mixer,
    pub solo: Option<String>,
    pending: VecDeque<i16>,
    pub talk: Option<TalkCapture>,
    #[cfg(feature = "native")]
    pub discovered: BTreeMap<String, lull_net::mdns::DiscoveredStation>,
    dirty: bool,
    last_publish: u64,
}

/// State shared by the engine handle and its tasks.
pub(crate) struct Shared {
    pub cfg: ListenerConfig,
    pub storage: Arc<dyn Storage>,
    pub clock: Arc<dyn Clock>,
    pub key: DeviceKey,
    pub device_id: String,
    pub inner: Mutex<Inner>,
    pub state_tx: watch::Sender<ListenerState>,
    pub events: broadcast::Sender<ListenerEvent>,
    pub video: broadcast::Sender<VideoFrameEvent>,
    pub next_session: AtomicU64,
}

/// Minimum spacing of snapshots caused by `level` updates (architecture.md
/// §5: levels at ≤ 10 Hz).
const LEVEL_PUBLISH_MS: u64 = 100;

impl Shared {
    pub fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("listener state poisoned")
    }

    pub fn now(&self) -> u64 {
        self.clock.now_ms()
    }

    pub fn emit(&self, e: ListenerEvent) {
        let _ = self.events.send(e);
    }

    pub fn limits(&self) -> AlertLimits {
        AlertLimits {
            sound_interval_ms: self.cfg.sound_alert_interval.as_millis() as u64,
            unreachable_after_ms: self.cfg.unreachable_alert_after.as_millis() as u64,
            battery_percent: self.cfg.battery_alert_percent,
        }
    }

    pub fn alert(&self, g: &Inner, id: &str, alert: Alert) {
        let Some(rt) = g.stations.get(id) else { return };
        let t = rt.known.settings.alerts;
        let on = match alert {
            Alert::SoundStarted => t.sound,
            Alert::Unreachable | Alert::Reachable => t.unreachable,
            Alert::BatteryLow { .. } => t.battery,
        };
        if on {
            self.emit(ListenerEvent::Alert {
                station_id: id.to_owned(),
                alert,
            });
        }
    }

    /// The listener's `hello` (protocol.md §7.2).
    pub fn hello(&self, g: &Inner) -> Hello {
        Hello {
            name: g.name.clone(),
            role: Role::Listener,
            app: crate::APP_NAME.to_owned(),
            caps: [caps::AUDIO, caps::VOX, caps::TALK, caps::VIDEO]
                .map(str::to_owned)
                .to_vec(),
            audio: crate::types::audio_codecs(),
            video: vec![VIDEO_CODEC_JPEG],
        }
    }

    pub fn save(&self, g: &Inner) {
        let _ = store::save_stations(&*self.storage, g.stations.values().map(|r| &r.known));
    }

    /// Publishes now.
    pub fn publish(&self, g: &mut Inner) {
        g.dirty = false;
        g.last_publish = self.now();
        let state = ListenerState {
            name: g.name.clone(),
            device_id: self.device_id.clone(),
            stations: g.stations.values().map(Rt::view).collect(),
            solo: g.solo.clone(),
            discovered: self.discovered_views(g),
        };
        self.state_tx.send_if_modified(|s| {
            if *s == state {
                false
            } else {
                *s = state;
                true
            }
        });
    }

    /// Publishes unless one went out less than 100 ms ago; the ticker
    /// catches up (levels at ≤ 10 Hz).
    pub fn touch(&self, g: &mut Inner) {
        if self.now().saturating_sub(g.last_publish) >= LEVEL_PUBLISH_MS {
            self.publish(g);
        } else {
            g.dirty = true;
        }
    }

    #[cfg(feature = "native")]
    fn discovered_views(&self, g: &Inner) -> Vec<DiscoveredView> {
        g.discovered
            .values()
            .map(|d| DiscoveredView {
                id: d.txt.id.to_string(),
                name: d.name.clone(),
                pairing: d.txt.pair,
                endpoints: d
                    .addrs
                    .iter()
                    .map(|ip| lull_proto::Endpoint::new(*ip, d.port))
                    .collect(),
                known: g.stations.contains_key(d.txt.id.as_str()),
            })
            .collect()
    }

    #[cfg(not(feature = "native"))]
    fn discovered_views(&self, _g: &Inner) -> Vec<DiscoveredView> {
        Vec::new()
    }

    /// Applies mixer settings of one station (FR-AU-6, FR-LS-2, FR-TB-2).
    pub fn apply_mixer(&self, g: &mut Inner, id: &str) {
        let Some(rt) = g.stations.get(id) else { return };
        let (h, s) = (rt.handle, rt.known.settings);
        let talking = rt.talk == TalkView::Granted;
        g.mixer.set_volume(h, s.volume);
        g.mixer.set_muted(h, s.muted);
        g.mixer.set_talking(h, talking);
    }

    /// Adds or updates a known station.
    pub fn upsert_station(&self, g: &mut Inner, known: KnownStation) {
        if let Some(rt) = g.stations.get_mut(&known.id) {
            let mut known = known;
            for e in &rt.known.endpoints {
                known.note_endpoint(&e.route, e.last_success_unix_ms);
            }
            known.settings = rt.known.settings;
            rt.known = known;
        } else {
            let id = known.id.clone();
            let handle = g.mixer.add_station();
            g.stations.insert(id.clone(), Rt::new(known, handle));
            self.apply_mixer(g, &id);
        }
    }

    /// Ends talk-back capture for `id` if it was ours.
    pub fn stop_talk_capture(&self, g: &mut Inner, id: &str) {
        if g.talk.as_ref().is_some_and(|t| t.station == id) {
            g.talk = None;
            self.emit(ListenerEvent::MicrophoneNotWanted);
        }
    }

    /// Starts talk-back capture for `id`.
    pub fn start_talk_capture(&self, g: &mut Inner, id: &str) {
        if g.talk.as_ref().is_some_and(|t| t.station == id) {
            return;
        }
        if g.talk.is_some() {
            self.emit(ListenerEvent::MicrophoneNotWanted);
        }
        g.talk = Some(TalkCapture {
            station: id.to_owned(),
            chain: None,
            encoder: None,
            seq: 0,
        });
        self.emit(ListenerEvent::MicrophoneWanted {
            station_id: id.to_owned(),
        });
    }

    pub fn next_session(&self) -> u64 {
        self.next_session.fetch_add(1, Ordering::Relaxed)
    }

    /// Periodic work: catch-up publishing and unreachable alerts (FR-AL-2).
    fn tick(&self) {
        let now = self.now();
        let limits = self.limits();
        let mut g = self.lock();
        if !g.running {
            return;
        }
        let mut due = Vec::new();
        for (id, rt) in g.stations.iter_mut() {
            if rt.following
                && rt.status != ConnStatus::Live
                && rt.alerts.check_unreachable(now, &limits)
            {
                due.push(id.clone());
            }
        }
        for id in due {
            self.alert(&g, &id, Alert::Unreachable);
        }
        if g.dirty {
            self.publish(&mut g);
        }
    }

    /// Marks `id` followed and returns the new reconnect loop generation.
    pub fn start_follow(&self, g: &mut Inner, id: &str) -> Result<u64, ListenerError> {
        let followed = g
            .stations
            .iter()
            .filter(|(k, r)| r.following && k.as_str() != id)
            .count();
        let max = self.cfg.max_followed;
        let now = self.now();
        let rt = g
            .stations
            .get_mut(id)
            .ok_or(ListenerError::UnknownStation)?;
        if !rt.following && followed >= max {
            return Err(ListenerError::TooManyStations);
        }
        rt.following = true;
        rt.generation += 1;
        rt.known.settings.follow = true;
        rt.known.needs_pairing = false;
        rt.audio_frames = 0;
        rt.preroll_frames = 0;
        if rt.status != ConnStatus::Live {
            rt.alerts.on_down(now);
            rt.status = ConnStatus::Connecting {
                attempt: 1,
                tried: Vec::new(),
            };
        }
        rt.wake.notify_waiters();
        Ok(rt.generation)
    }
}

/// The listener engine. Cheap to clone; all clones control the same
/// listener. Create it inside a tokio runtime (native) or the browser.
#[derive(Clone)]
pub struct ListenerEngine {
    shared: Arc<Shared>,
}

impl fmt::Debug for ListenerEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ListenerEngine")
            .field("device_id", &self.shared.device_id)
            .finish_non_exhaustive()
    }
}

fn parse_station_id(id: &str) -> Result<String, ListenerError> {
    lull_proto::DeviceId::parse(id)
        .map(|d| d.as_str().to_owned())
        .map_err(|_| ListenerError::Invalid("station id".into()))
}

impl ListenerEngine {
    /// Loads (or creates) the device key and known stations and starts
    /// following the stations marked as followed.
    // In the browser everything is single-threaded; `Arc` is still needed
    // to share state with spawned local tasks.
    #[cfg_attr(not(feature = "native"), allow(clippy::arc_with_non_send_sync))]
    pub fn start(
        cfg: ListenerConfig,
        storage: Arc<dyn Storage>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, ListenerError> {
        let key = persist::load_or_create_device_key(&*storage)?;
        let device_id = key.device_id().as_str().to_owned();
        let name = persist::load::<ListenerRecord>(&*storage, KEY_LISTENER)?
            .map(|r| r.name)
            .unwrap_or_else(|| clamp_name(&cfg.name));
        let known = store::load_stations(&*storage)?;
        let (state_tx, _) = watch::channel(ListenerState {
            name: name.clone(),
            device_id: device_id.clone(),
            stations: Vec::new(),
            solo: None,
            discovered: Vec::new(),
        });
        let (events, _) = broadcast::channel(256);
        let (video, _) = broadcast::channel(16);
        let shared = Arc::new(Shared {
            cfg,
            storage,
            clock,
            key,
            device_id,
            inner: Mutex::new(Inner {
                running: true,
                name,
                stations: BTreeMap::new(),
                mixer: Mixer::new(),
                solo: None,
                pending: VecDeque::new(),
                talk: None,
                #[cfg(feature = "native")]
                discovered: BTreeMap::new(),
                dirty: false,
                last_publish: 0,
            }),
            state_tx,
            events,
            video,
            next_session: AtomicU64::new(1),
        });
        let mut to_follow = Vec::new();
        {
            let mut g = shared.lock();
            for k in known {
                let follow = k.settings.follow && !k.needs_pairing;
                let needs_pairing = k.needs_pairing;
                let id = k.id.clone();
                shared.upsert_station(&mut g, k);
                if needs_pairing && let Some(rt) = g.stations.get_mut(&id) {
                    rt.status = ConnStatus::NeedsPairing;
                }
                if follow && let Ok(gen_) = shared.start_follow(&mut g, &id) {
                    to_follow.push((id, gen_));
                }
            }
            shared.publish(&mut g);
        }
        for (id, gen_) in to_follow {
            rt::spawn(follow::follow_loop(shared.clone(), id, gen_));
        }
        spawn_ticker(shared.clone());
        #[cfg(feature = "native")]
        if shared.cfg.discovery {
            follow::spawn_discovery(shared.clone());
        }
        Ok(Self { shared })
    }

    /// Subscribes to state snapshots.
    pub fn state(&self) -> watch::Receiver<ListenerState> {
        self.shared.state_tx.subscribe()
    }

    /// The current snapshot.
    pub fn snapshot(&self) -> ListenerState {
        self.shared.state_tx.borrow().clone()
    }

    /// Subscribes to events.
    pub fn events(&self) -> broadcast::Receiver<ListenerEvent> {
        self.shared.events.subscribe()
    }

    /// Subscribes to video frames of all stations.
    pub fn video_frames(&self) -> broadcast::Receiver<VideoFrameEvent> {
        self.shared.video.subscribe()
    }

    /// Our device ID.
    pub fn device_id(&self) -> &str {
        &self.shared.device_id
    }

    /// Known stations.
    pub fn known_stations(&self) -> Vec<KnownStation> {
        self.shared
            .lock()
            .stations
            .values()
            .map(|r| r.known.clone())
            .collect()
    }

    /// Renames this listener (sent in `hello` from the next session on).
    pub fn rename(&self, name: &str) -> Result<(), ListenerError> {
        let name = clamp_name(name);
        let mut g = self.shared.lock();
        g.name = name.clone();
        persist::save(
            &*self.shared.storage,
            KEY_LISTENER,
            &ListenerRecord { name },
        )?;
        self.shared.publish(&mut g);
        Ok(())
    }

    // ----- pairing ---------------------------------------------------------

    /// Pairs from an invitation, either `lull://invite?…` or the station's
    /// web address with the invitation in its fragment (FR-PAIR-1,
    /// protocol.md §3.5). Routes: the web address's WebSocket (if given),
    /// then the invitation's endpoints.
    pub async fn pair_invitation(&self, input: &str) -> Result<PairResult, ListenerError> {
        self.pair_invitation_via(input, Vec::new()).await
    }

    /// Like [`pair_invitation`](Self::pair_invitation), trying `routes` first.
    pub async fn pair_invitation_via(
        &self,
        input: &str,
        routes: Vec<Route>,
    ) -> Result<PairResult, ListenerError> {
        let inv = lull_proto::Invitation::parse_any(input)
            .map_err(|e| ListenerError::Invalid(e.to_string()))?;
        let mut all = routes;
        if !input.trim().to_ascii_lowercase().starts_with("lull://")
            && let Some(r) = Route::websocket_for_origin(input.trim())
        {
            all.push(r);
        }
        all.extend(inv.endpoints.iter().cloned().map(Route::Tcp));
        let station = lull_crypto::PublicKey::from_bytes(inv.station_key);
        let token = lull_crypto::InviteToken::from_bytes(&inv.token);
        let mode = lull_session::ListenerMode::Invite { station, token };
        follow::pair(&self.shared, mode, all).await
    }

    /// Pairs with a pairing code (FR-PAIR-2). `station_id` comes from the
    /// station's mDNS record (or is entered); `routes` reach it. With no
    /// routes, discovered addresses of that station are used.
    pub async fn pair_code(
        &self,
        code: &str,
        station_id: &str,
        routes: Vec<Route>,
    ) -> Result<PairResult, ListenerError> {
        let code = lull_crypto::PairingCode::parse(code)
            .map_err(|e| ListenerError::Invalid(e.to_string()))?;
        let id = lull_crypto::DeviceId::parse(station_id)
            .map_err(|_| ListenerError::Invalid("station id".into()))?;
        let mut routes = routes;
        if routes.is_empty() {
            routes = follow::discovered_routes(&self.shared, id.as_str());
        }
        if routes.is_empty() {
            return Err(ListenerError::Unreachable(
                "no address for the station".into(),
            ));
        }
        let mode = follow::code_mode(code, id).await;
        follow::pair(&self.shared, mode, routes).await
    }

    // ----- following -------------------------------------------------------

    /// Follows a known station (FR-LS-1).
    pub fn follow(&self, station_id: &str) -> Result<(), ListenerError> {
        let id = parse_station_id(station_id)?;
        let shared = &self.shared;
        let mut g = shared.lock();
        if g.stations.get(&id).is_some_and(|r| r.following) {
            return Ok(());
        }
        let gen_ = shared.start_follow(&mut g, &id)?;
        shared.save(&g);
        shared.publish(&mut g);
        drop(g);
        rt::spawn(follow::follow_loop(shared.clone(), id, gen_));
        Ok(())
    }

    /// Stops following a station; its session is closed.
    pub fn unfollow(&self, station_id: &str) -> Result<(), ListenerError> {
        let id = parse_station_id(station_id)?;
        let shared = &self.shared;
        let mut g = shared.lock();
        let rt = g
            .stations
            .get_mut(&id)
            .ok_or(ListenerError::UnknownStation)?;
        rt.following = false;
        rt.generation += 1;
        rt.known.settings.follow = false;
        rt.status = ConnStatus::Idle;
        rt.send(Cmd::Close(CloseReason::Shutdown));
        rt.wake.notify_waiters();
        shared.stop_talk_capture(&mut g, &id);
        shared.save(&g);
        shared.publish(&mut g);
        Ok(())
    }

    /// Forgets a station (FR-PAIR-6): sends `unpair` if it is connected,
    /// then removes it.
    pub fn forget(&self, station_id: &str) -> Result<(), ListenerError> {
        let id = parse_station_id(station_id)?;
        let shared = &self.shared;
        let mut g = shared.lock();
        let rt = g
            .stations
            .remove(&id)
            .ok_or(ListenerError::UnknownStation)?;
        rt.send(Cmd::Control(Control::Unpair));
        rt.wake.notify_waiters();
        g.mixer.remove_station(rt.handle);
        if g.solo.as_deref() == Some(id.as_str()) {
            g.solo = None;
            g.mixer.set_solo(None);
        }
        shared.stop_talk_capture(&mut g, &id);
        shared.save(&g);
        shared.emit(ListenerEvent::Forgotten { station_id: id });
        shared.publish(&mut g);
        Ok(())
    }

    /// Adds a manually entered address for a station (FR-NET-2).
    pub fn add_route(&self, station_id: &str, route: Route) -> Result<(), ListenerError> {
        let id = parse_station_id(station_id)?;
        let mut g = self.shared.lock();
        let rt = g
            .stations
            .get_mut(&id)
            .ok_or(ListenerError::UnknownStation)?;
        rt.known.note_endpoint(&route, None);
        rt.wake.notify_waiters();
        self.shared.save(&g);
        self.shared.publish(&mut g);
        Ok(())
    }

    // ----- per-station settings -------------------------------------------

    fn update_settings(
        &self,
        station_id: &str,
        f: impl FnOnce(&mut StationSettings),
    ) -> Result<(), ListenerError> {
        let id = parse_station_id(station_id)?;
        let shared = &self.shared;
        let mut g = shared.lock();
        let rt = g
            .stations
            .get_mut(&id)
            .ok_or(ListenerError::UnknownStation)?;
        let before = rt.known.settings;
        f(&mut rt.known.settings);
        if rt.known.settings.listen != before.listen {
            // protocol.md §7.4: sent whenever the user changes it.
            rt.send(Cmd::Control(Control::Listen(
                rt.known.settings.listen.to_wire(),
            )));
        }
        shared.apply_mixer(&mut g, &id);
        shared.save(&g);
        shared.publish(&mut g);
        Ok(())
    }

    /// Sets the listen mode, threshold, hold and pre-roll (FR-AU-2).
    pub fn set_listen(
        &self,
        station_id: &str,
        listen: ListenSettings,
    ) -> Result<(), ListenerError> {
        self.update_settings(station_id, |s| s.listen = listen)
    }

    /// Sets the playback volume, linear 0..1 (FR-AU-6).
    pub fn set_volume(&self, station_id: &str, volume: f32) -> Result<(), ListenerError> {
        let v = if volume.is_nan() {
            0.0
        } else {
            volume.clamp(0.0, 1.0)
        };
        self.update_settings(station_id, |s| s.volume = v)
    }

    /// Mutes or unmutes a station; alerts keep working (FR-AU-6, FR-AL-4).
    pub fn set_muted(&self, station_id: &str, muted: bool) -> Result<(), ListenerError> {
        self.update_settings(station_id, |s| s.muted = muted)
    }

    /// Sets which alerts a station raises.
    pub fn set_alerts(&self, station_id: &str, alerts: AlertToggles) -> Result<(), ListenerError> {
        self.update_settings(station_id, |s| s.alerts = alerts)
    }

    /// Hears only `station_id`, or every station with `None` (FR-LS-2).
    pub fn set_solo(&self, station_id: Option<&str>) -> Result<(), ListenerError> {
        let shared = &self.shared;
        let mut g = shared.lock();
        match station_id {
            None => {
                g.solo = None;
                g.mixer.set_solo(None);
            }
            Some(s) => {
                let id = parse_station_id(s)?;
                let h = g
                    .stations
                    .get(&id)
                    .ok_or(ListenerError::UnknownStation)?
                    .handle;
                g.solo = Some(id);
                g.mixer.set_solo(Some(h));
            }
        }
        shared.publish(&mut g);
        Ok(())
    }

    // ----- talk-back -------------------------------------------------------

    /// Push-to-talk (FR-TB-1): `true` sends `talk-request`, `false`
    /// `talk-release`. While granted, feed
    /// [`push_talk_capture`](Self::push_talk_capture).
    pub fn talk(&self, station_id: &str, on: bool) -> Result<(), ListenerError> {
        let id = parse_station_id(station_id)?;
        let shared = &self.shared;
        let mut g = shared.lock();
        let rt = g
            .stations
            .get_mut(&id)
            .ok_or(ListenerError::UnknownStation)?;
        if rt.status != ConnStatus::Live {
            return Err(ListenerError::NotConnected);
        }
        if on {
            rt.send(Cmd::Control(Control::TalkRequest));
            if rt.talk != TalkView::Granted {
                rt.talk = TalkView::Requested;
            }
        } else {
            rt.send(Cmd::Control(Control::TalkRelease));
            rt.talk = TalkView::Idle;
            shared.stop_talk_capture(&mut g, &id);
            shared.apply_mixer(&mut g, &id);
        }
        shared.publish(&mut g);
        Ok(())
    }

    /// Feeds interleaved 16-bit microphone samples for talk-back (media.md
    /// §4.3); ignored unless talk-back is granted.
    pub fn push_talk_capture(&self, samples: &[i16], rate: u32, channels: u16) {
        let mut g = self.shared.lock();
        let Inner { talk, stations, .. } = &mut *g;
        let Some(tc) = talk.as_mut() else { return };
        let Some(rt) = stations.get(&tc.station) else {
            return;
        };
        if rt.talk != TalkView::Granted || rate == 0 || channels == 0 {
            return;
        }
        let Some(codec) = rt.codec else { return };
        if !matches!(&tc.chain, Some((r, c, _)) if *r == rate && *c == channels) {
            // media.md §4.3: same capture path, gain fixed at 0 dB.
            match CaptureChain::new(rate, channels, GainSetting::Manual(GainStep::Db0)) {
                Ok(c) => tc.chain = Some((rate, channels, c)),
                Err(_) => return,
            }
        }
        if tc.encoder.as_ref().is_none_or(|e| e.codec() != codec) {
            match lull_media::codec::new_encoder(codec) {
                Ok(e) => tc.encoder = Some(e),
                Err(_) => return,
            }
        }
        let (_, _, chain) = tc.chain.as_mut().expect("set above");
        let encoder = tc.encoder.as_mut().expect("set above");
        chain.push_i16(samples);
        while let Some(f) = chain.pop_frame() {
            let Ok(payload) = encoder.encode(&f.pcm) else {
                continue;
            };
            let header = AudioHeader {
                stream: AudioStream::TalkBack,
                codec: codec.0,
                flags: AudioFlags::new(false, tc.seq == 0),
                seq: tc.seq,
                timestamp: tc.seq.wrapping_mul(FRAME_TICKS),
            };
            tc.seq = tc.seq.wrapping_add(1);
            rt.send(Cmd::TalkAudio(header, payload));
        }
    }

    // ----- video -----------------------------------------------------------

    /// Opens the camera view of a station (FR-CAM-1).
    pub fn request_video(&self, station_id: &str, req: VideoRequest) -> Result<(), ListenerError> {
        let id = parse_station_id(station_id)?;
        let mut g = self.shared.lock();
        let rt = g
            .stations
            .get_mut(&id)
            .ok_or(ListenerError::UnknownStation)?;
        rt.video_req = Some(req);
        rt.video = VideoView::Requested;
        rt.send(Cmd::Control(video_start(req)));
        self.shared.publish(&mut g);
        Ok(())
    }

    /// Closes the camera view (FR-CAM-1).
    pub fn stop_video(&self, station_id: &str) -> Result<(), ListenerError> {
        let id = parse_station_id(station_id)?;
        let mut g = self.shared.lock();
        let rt = g
            .stations
            .get_mut(&id)
            .ok_or(ListenerError::UnknownStation)?;
        rt.video_req = None;
        rt.video = VideoView::Off;
        rt.send(Cmd::Control(Control::VideoStop));
        self.shared.publish(&mut g);
        Ok(())
    }

    /// Chooses the station camera and night image (FR-CAM-4).
    pub fn set_video_options(
        &self,
        station_id: &str,
        camera: CameraFacing,
        night: bool,
    ) -> Result<(), ListenerError> {
        let id = parse_station_id(station_id)?;
        let mut g = self.shared.lock();
        let rt = g
            .stations
            .get_mut(&id)
            .ok_or(ListenerError::UnknownStation)?;
        rt.video_options = Some((camera, night));
        rt.send(Cmd::Control(Control::VideoOptions { camera, night }));
        Ok(())
    }

    // ----- playback --------------------------------------------------------

    /// Fills `out` with the 16 kHz mono mix of all audible stations
    /// (FR-LS-2, media.md §4). Call at the output device's pace; every
    /// 20 ms frame is pulled from each station's jitter buffer at the
    /// current time.
    pub fn pull_playback(&self, out: &mut [i16]) {
        let now = self.shared.now();
        let mut g = self.shared.lock();
        let Inner {
            stations,
            mixer,
            pending,
            ..
        } = &mut *g;
        let mut i = 0;
        while i < out.len() {
            if pending.is_empty() {
                let frames: Vec<(StationHandle, PcmFrame)> = stations
                    .values_mut()
                    .filter_map(|rt| Some((rt.handle, rt.jitter.as_mut()?.pull(now).pcm())))
                    .collect();
                let inputs: Vec<(StationHandle, &PcmFrame)> =
                    frames.iter().map(|(h, f)| (*h, f)).collect();
                pending.extend(mixer.mix(&inputs));
            }
            while i < out.len() {
                match pending.pop_front() {
                    Some(s) => {
                        out[i] = s;
                        i += 1;
                    }
                    None => break,
                }
            }
        }
    }

    // ----- lifecycle -------------------------------------------------------

    /// Closes all sessions and stops reconnecting.
    pub async fn shutdown(&self) {
        {
            let mut g = self.shared.lock();
            if !g.running {
                return;
            }
            g.running = false;
            for rt in g.stations.values_mut() {
                rt.following = false;
                rt.generation += 1;
                rt.send(Cmd::Close(CloseReason::Shutdown));
                rt.wake.notify_waiters();
            }
        }
        for _ in 0..50 {
            if self
                .shared
                .lock()
                .stations
                .values()
                .all(|r| r.cmd.is_none())
            {
                break;
            }
            rt::sleep(Duration::from_millis(20)).await;
        }
    }
}

pub(crate) fn video_start(req: VideoRequest) -> Control {
    Control::VideoStart(VideoStart {
        max_w: req.max_w,
        max_h: req.max_h,
        max_fps: req.max_fps,
        codecs: vec![VIDEO_CODEC_JPEG],
    })
}

/// Maps `talk-state` (protocol.md §7.8).
pub(crate) fn talk_view(state: TalkStatus, by: Option<String>) -> TalkView {
    match state {
        TalkStatus::Granted => TalkView::Granted,
        TalkStatus::Busy => TalkView::Busy { by },
        TalkStatus::Idle => TalkView::Idle,
        TalkStatus::Disabled => TalkView::Disabled,
    }
}

fn spawn_ticker(shared: Arc<Shared>) {
    rt::spawn(async move {
        loop {
            rt::sleep(Duration::from_millis(LEVEL_PUBLISH_MS)).await;
            if !shared.lock().running {
                break;
            }
            shared.tick();
        }
    });
}
