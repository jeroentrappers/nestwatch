// SPDX-License-Identifier: AGPL-3.0-or-later
//! The station engine (product.md §3.1, §3.3, §3.4, §3.6, §3.7;
//! protocol.md; media.md).
//!
//! A [`StationEngine`] serves listeners over TCP and WebSocket, advertises
//! itself over mDNS, runs the pairing window, and moves media:
//!
//! - audio: [`push_capture`](StationEngine::push_capture) → capture chain →
//!   level → encoder → shared ring → per-listener sound activation gate →
//!   session (media.md §1, §3);
//! - talk-back: stream 1 from the one granted listener →
//!   [`pull_talkback`](StationEngine::pull_talkback) (FR-TB-1, FR-TB-3);
//! - video: [`StationEvent::CameraWanted`] while any listener watches;
//!   frames pushed with [`push_camera_frame`](StationEngine::push_camera_frame)
//!   are scaled, filtered and JPEG-encoded per listener (media.md §5).

mod media;
mod pairing;
mod session;
mod trust;

use std::collections::BTreeMap;
use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use minicbor::{Decode, Encode};
use nestwatch_crypto::{DeviceKey, InviteToken, PairingCode};
use nestwatch_media::capture::{GainSetting, GainStep};
use nestwatch_net::StationListener;
use nestwatch_net::mdns::{Advertisement, Advertiser};
use nestwatch_net::web::{StationCert, WebServer};
use nestwatch_proto::control::{CloseReason, Control, Hello, Role, StationStatus, caps};
use nestwatch_proto::{Endpoint, Host, Invitation, StationTxt};
use thiserror::Error;
use tokio::sync::{broadcast, watch};

use crate::clock::Clock;
use crate::persist;
use crate::storage::{Storage, StorageError};
use crate::types::{ListenMode, Transport, clamp_name};

pub use media::{CameraAvailability, CameraFrame, CameraRequest, PixelFormat};
pub use nestwatch_net::web::{AssetSource, WebConfig};
pub use nestwatch_net::{BindScope, ListenConfig};
pub use pairing::PairingEnd;
pub use trust::{KEY_TRUST, TrustedListener};

use media::{Audio, Camera, Talk};
use pairing::{PairingWindow, RateLimiter, WindowLimits};
use session::{Cmd, Sess};
use trust::TrustStore;

/// Storage key of the station settings.
pub const KEY_SETTINGS: &str = "station-settings";
/// Storage key of the HTTPS certificate (protocol.md §3.3).
pub const KEY_CERT: &str = "station-cert";

/// Connections still in preface or handshake at most; more are dropped
/// without an answer.
const MAX_PENDING_SESSIONS: usize = 16;

/// The browser listener app and its server ports (protocol.md §3.3).
#[derive(Debug, Clone)]
pub struct WebOptions {
    /// Ports, CSP and origin checks.
    pub config: WebConfig,
    /// The app's files, provided by the caller.
    pub assets: AssetSource,
}

/// Station configuration. Tunables default to the spec values; tests
/// lower them.
#[derive(Debug, Clone)]
pub struct StationConfig {
    /// Display name used until the user renames the station.
    pub name: String,
    /// TCP listener ports (protocol.md §3.2).
    pub tcp: ListenConfig,
    /// HTTP(S) and WebSocket serving; `None` disables it.
    pub web: Option<WebOptions>,
    /// Advertise over mDNS (protocol.md §3.4).
    pub mdns: bool,
    /// Endpoints to put in invitations instead of the local interface
    /// addresses (protocol.md §3.5).
    pub endpoints: Option<Vec<Endpoint>>,
    /// Time allowed to ask Tailscale for the VPN DNS name.
    pub tailscale_timeout: Duration,
    /// Concurrent listener sessions; more get `busy` (protocol.md §9: 8).
    pub max_listeners: usize,
    /// Pairing window length (FR-PAIR-3: 10 minutes).
    pub pairing_window: Duration,
    /// Pairings per window (FR-PAIR-3: 8).
    pub max_pairings: u32,
    /// Failed code attempts that close the window (protocol.md §5.3: 5).
    pub max_code_failures: u32,
    /// `level` period (protocol.md §7.5: 250 ms).
    pub level_interval: Duration,
    /// Longest time between `status` messages (protocol.md §7.6: 30 s).
    pub status_interval: Duration,
}

impl StationConfig {
    /// Spec defaults on all interfaces.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            tcp: ListenConfig::default(),
            web: None,
            mdns: true,
            endpoints: None,
            tailscale_timeout: nestwatch_net::local::TAILSCALE_TIMEOUT,
            max_listeners: 8,
            pairing_window: Duration::from_secs(600),
            max_pairings: 8,
            max_code_failures: 5,
            level_interval: Duration::from_millis(250),
            status_interval: Duration::from_secs(30),
        }
    }

    /// Loopback only, OS-chosen ports, no mDNS, no web server: for tests
    /// and local tools. Invitations carry `127.0.0.1:<port>`.
    pub fn loopback(name: impl Into<String>) -> Self {
        Self {
            tcp: ListenConfig {
                port: 0,
                attempts: 1,
                scope: nestwatch_net::BindScope::Loopback,
            },
            mdns: false,
            ..Self::new(name)
        }
    }
}

/// Persistent station settings (FR-ST-7, FR-ST-8, FR-TB-3, FR-AU-5).
#[derive(Debug, Clone, PartialEq, Encode, Decode)]
pub struct StationSettings {
    /// Display name.
    #[n(0)]
    pub name: String,
    /// Camera allowed at all (FR-ST-8).
    #[n(1)]
    pub camera_enabled: bool,
    /// Talk-back allowed at all (FR-ST-8).
    #[n(2)]
    pub talk_enabled: bool,
    /// Talk-back playback volume, linear 0..1 (FR-TB-3).
    #[n(3)]
    pub talkback_volume: f32,
    /// Microphone gain: 255 = automatic, else manual dB (0, 6, 12, 18).
    #[n(4)]
    pub gain_db: u8,
}

impl StationSettings {
    fn defaults(name: &str) -> Self {
        Self {
            name: clamp_name(name),
            camera_enabled: true,
            talk_enabled: true,
            talkback_volume: 1.0,
            gain_db: GAIN_AUTO,
        }
    }

    /// The gain as a [`GainSetting`] (AGC on by default, FR-AU-5).
    pub fn gain(&self) -> GainSetting {
        match self.gain_db {
            0 => GainSetting::Manual(GainStep::Db0),
            6 => GainSetting::Manual(GainStep::Db6),
            12 => GainSetting::Manual(GainStep::Db12),
            18 => GainSetting::Manual(GainStep::Db18),
            _ => GainSetting::Auto,
        }
    }
}

const GAIN_AUTO: u8 = 255;

fn gain_code(g: GainSetting) -> u8 {
    match g {
        GainSetting::Auto => GAIN_AUTO,
        GainSetting::Manual(s) => s.db() as u8,
    }
}

#[derive(Encode, Decode)]
struct CertRecord {
    #[n(0)]
    cert_pem: String,
    #[n(1)]
    key_pem: String,
}

/// The open pairing window as shown to the user (FR-PAIR-1, FR-PAIR-2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingInfo {
    /// `nestwatch://invite?...` for the QR code and the copyable link.
    pub invite_uri: String,
    /// The browser address with the invitation in its fragment.
    pub web_url: Option<String>,
    /// The pairing code, `XXXXX-XXXXX`.
    pub code: String,
    /// Monotonic time ([`Clock::now_ms`]) at which the window closes.
    pub closes_at_ms: u64,
    /// Pairings so far in this window.
    pub pairings: u32,
    /// Failed code attempts so far.
    pub code_failures: u32,
}

/// A connected listener (FR-ST-3).
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectedListener {
    /// Engine-local session number.
    pub session: u64,
    /// Device ID.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Its listen mode.
    pub mode: ListenMode,
    /// Its sound activation threshold in dBFS (meaningful when `mode` is
    /// activated; shown as the ring tick, design README §4).
    pub threshold_dbfs: f32,
    /// Whether audio currently flows to it (`level.open`).
    pub open: bool,
    /// Whether it holds talk-back.
    pub talking: bool,
    /// Whether it has the camera view open.
    pub watching: bool,
    /// TCP or WebSocket.
    pub transport: Transport,
    /// Remote address, if known.
    pub peer: Option<SocketAddr>,
    /// Last measured round-trip time.
    pub rtt_ms: Option<u64>,
}

/// Snapshot of the station for its UI (architecture.md §5).
#[derive(Debug, Clone, PartialEq)]
pub struct StationState {
    /// Display name.
    pub name: String,
    /// Device ID (protocol.md §2.2).
    pub device_id: String,
    /// Whether the engine is serving.
    pub running: bool,
    /// Bound TCP port.
    pub tcp_port: u16,
    /// Bound HTTP port of the browser app.
    pub http_port: Option<u16>,
    /// Bound HTTPS port of the browser app.
    pub https_port: Option<u16>,
    /// SHA-256 fingerprint of the HTTPS certificate (FR-WEB-2).
    pub https_fingerprint: Option<String>,
    /// Endpoints put in invitations (protocol.md §3.5).
    pub endpoints: Vec<Endpoint>,
    /// Whether mDNS advertising runs.
    pub mdns: bool,
    /// The open pairing window.
    pub pairing: Option<PairingInfo>,
    /// Connected listeners (FR-ST-3).
    pub listeners: Vec<ConnectedListener>,
    /// Whether any camera feed is wanted and being sent (FR-ST-3, FR-CAM-2).
    pub camera_on: bool,
    /// Name of the listener talking (FR-ST-3, FR-TB-3).
    pub talker: Option<String>,
    /// Last `level` value in dBFS (FR-ST-1).
    pub level_dbfs: f32,
    /// Battery percent, if known (FR-ST-6).
    pub battery: Option<u8>,
    /// Charging state, if known.
    pub charging: Option<bool>,
    /// Persistent settings.
    pub settings: StationSettings,
    /// What the app last reported about the camera.
    pub camera_availability: CameraAvailability,
}

/// Something that happened on the station.
#[derive(Debug, Clone, PartialEq)]
pub enum StationEvent {
    /// Pairing mode opened.
    PairingOpened,
    /// Pairing mode ended.
    PairingClosed {
        /// Why.
        reason: PairingEnd,
    },
    /// A listener paired; show `verification` next to its name (FR-PAIR-5).
    Paired {
        /// Device ID.
        listener_id: String,
        /// Display name.
        name: String,
        /// Verification number, `042 917` (protocol.md §5.5).
        verification: String,
    },
    /// A listener finished setup.
    ListenerConnected {
        /// Session number.
        session: u64,
        /// Device ID.
        listener_id: String,
        /// Display name.
        name: String,
    },
    /// A connected listener's session ended.
    ListenerDisconnected {
        /// Session number.
        session: u64,
        /// Device ID.
        listener_id: String,
        /// Why, for diagnostics.
        reason: String,
    },
    /// A listener connected again; its older session got `replaced` (§9).
    SessionReplaced {
        /// Device ID.
        listener_id: String,
        /// The closed session.
        old_session: u64,
    },
    /// The user revoked a listener (FR-PAIR-6).
    ListenerRevoked {
        /// Device ID.
        listener_id: String,
    },
    /// A listener forgot this station and sent `unpair` (§7.10).
    ListenerUnpaired {
        /// Device ID.
        listener_id: String,
    },
    /// Start (or reconfigure) the camera (FR-CAM-1, media.md §5).
    CameraWanted(CameraRequest),
    /// Stop the camera: nobody watches any more (FR-CAM-1).
    CameraNotWanted,
    /// A listener started talking; play [`StationEngine::pull_talkback`].
    TalkStarted {
        /// Its name.
        name: String,
    },
    /// Talk-back ended.
    TalkEnded,
}

/// Errors starting or controlling the station.
#[derive(Debug, Error)]
pub enum StationError {
    /// Storage failed.
    #[error(transparent)]
    Storage(#[from] StorageError),
    /// Binding or serving failed.
    #[error(transparent)]
    Net(#[from] nestwatch_net::NetError),
    /// The random number generator failed.
    #[error("random number generator failed")]
    Rng,
    /// The engine was shut down.
    #[error("station stopped")]
    Stopped,
    /// Invalid input.
    #[error("invalid input: {0}")]
    Invalid(String),
}

/// Mutable state behind the engine's lock.
pub(crate) struct Inner {
    pub running: bool,
    pub settings: StationSettings,
    pub trust: TrustStore,
    pub pairing: Option<PairingWindow>,
    pub pairing_info: Option<PairingInfo>,
    pub limiter: RateLimiter,
    pub sessions: BTreeMap<u64, Sess>,
    pub audio: Audio,
    pub talk: Talk,
    pub camera: Camera,
    pub battery: Option<u8>,
    pub charging: Option<bool>,
    pub last_status: Option<StationStatus>,
    pub last_status_ms: u64,
    pub advertiser: Option<Advertiser>,
    pub tcp_port: u16,
    pub http_port: Option<u16>,
    pub https_port: Option<u16>,
    pub fingerprint: Option<String>,
    pub endpoints: Vec<Endpoint>,
}

/// State shared by the engine handle and its tasks.
pub(crate) struct Shared {
    pub cfg: StationConfig,
    pub storage: Arc<dyn Storage>,
    pub clock: Arc<dyn Clock>,
    pub key: DeviceKey,
    pub device_id: String,
    pub inner: Mutex<Inner>,
    pub state_tx: watch::Sender<StationState>,
    pub events: broadcast::Sender<StationEvent>,
    pub next_session: AtomicU64,
    pub shutdown: watch::Sender<bool>,
}

impl Shared {
    pub fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().expect("station state poisoned")
    }

    pub fn emit(&self, e: StationEvent) {
        let _ = self.events.send(e);
    }

    pub fn now(&self) -> u64 {
        self.clock.now_ms()
    }

    /// The station's `hello` for a new session (protocol.md §7.2).
    pub fn hello(&self, g: &Inner) -> Hello {
        let mut c = vec![caps::AUDIO.to_owned(), caps::VOX.to_owned()];
        if g.settings.talk_enabled {
            c.push(caps::TALK.to_owned());
        }
        let camera = g.settings.camera_enabled;
        if camera {
            c.push(caps::VIDEO.to_owned());
        }
        if g.battery.is_some() {
            c.push(caps::BATTERY.to_owned());
        }
        let video_now = camera && g.camera.availability == CameraAvailability::Available;
        Hello {
            name: g.settings.name.clone(),
            role: Role::Station,
            app: crate::APP_NAME.to_owned(),
            caps: c,
            audio: crate::types::audio_codecs(),
            video: if video_now {
                vec![nestwatch_proto::record::VIDEO_CODEC_JPEG]
            } else {
                Vec::new()
            },
        }
    }

    /// The current `status` (protocol.md §7.6).
    pub fn status(&self, g: &Inner) -> StationStatus {
        StationStatus {
            name: g.settings.name.clone(),
            battery: g.battery,
            charging: g.charging,
            listeners: g.sessions.values().filter(|s| s.connected).count() as u32,
            camera: g.camera.wanted.is_some(),
            talker: g
                .talk
                .talker
                .and_then(|id| g.sessions.get(&id))
                .and_then(|s| s.name.clone()),
        }
    }

    /// Sends `status` to every listener if it changed, or always with
    /// `force` (protocol.md §7.6).
    pub fn send_status(&self, g: &mut Inner, force: bool) {
        let status = self.status(g);
        if !force && g.last_status.as_ref() == Some(&status) {
            return;
        }
        for s in g.sessions.values().filter(|s| s.connected) {
            s.send(Cmd::Control(Control::Status(status.clone())));
        }
        g.last_status = Some(status);
        g.last_status_ms = self.now();
    }

    /// Publishes a new [`StationState`].
    pub fn publish(&self, g: &Inner) {
        let talker = g.talk.talker;
        let listeners = g
            .sessions
            .iter()
            .filter(|(_, s)| s.connected)
            .map(|(&id, s)| ConnectedListener {
                session: id,
                id: s
                    .key
                    .map(|k| k.device_id().as_str().to_owned())
                    .unwrap_or_default(),
                name: s.name.clone().unwrap_or_default(),
                mode: s.mode,
                threshold_dbfs: s.threshold_dbfs,
                open: s.gate.as_ref().is_some_and(|g| g.is_flowing()),
                talking: talker == Some(id),
                watching: s.video.is_some(),
                transport: s.transport,
                peer: s.peer,
                rtt_ms: s.rtt_ms,
            })
            .collect();
        let state = StationState {
            name: g.settings.name.clone(),
            device_id: self.device_id.clone(),
            running: g.running,
            tcp_port: g.tcp_port,
            http_port: g.http_port,
            https_port: g.https_port,
            https_fingerprint: g.fingerprint.clone(),
            endpoints: g.endpoints.clone(),
            mdns: g.advertiser.is_some(),
            pairing: g.pairing_info.clone(),
            listeners,
            camera_on: g.camera.wanted.is_some(),
            talker: talker
                .and_then(|id| g.sessions.get(&id))
                .and_then(|s| s.name.clone()),
            level_dbfs: g.audio.last_level,
            battery: g.battery,
            charging: g.charging,
            settings: g.settings.clone(),
            camera_availability: g.camera.availability,
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

    /// The TXT data for mDNS (protocol.md §3.4).
    fn txt(&self, g: &Inner) -> StationTxt {
        StationTxt {
            id: nestwatch_proto::DeviceId::parse(&self.device_id).expect("canonical device id"),
            pair: g.pairing.is_some(),
            http: g.http_port,
            https: g.https_port,
        }
    }

    /// Re-announces mDNS after a TXT or name change.
    pub fn update_mdns(&self, g: &mut Inner, renamed: bool) {
        if !self.cfg.mdns || !g.running {
            return;
        }
        let txt = self.txt(g);
        if renamed || g.advertiser.is_none() {
            g.advertiser = None;
            g.advertiser = Advertiser::start(Advertisement {
                name: g.settings.name.clone(),
                port: g.tcp_port,
                txt,
            })
            .ok();
        } else if let Some(a) = g.advertiser.as_mut() {
            let _ = a.set_txt(txt);
        }
    }

    /// Closes the pairing window, if open.
    pub fn close_window(&self, g: &mut Inner, reason: PairingEnd) {
        if g.pairing.take().is_some() {
            g.pairing_info = None;
            self.update_mdns(g, false);
            self.emit(StationEvent::PairingClosed { reason });
            self.publish(g);
        }
    }

    /// Ends the window if one of its limits was reached.
    pub fn check_window(&self, g: &mut Inner) {
        let now = self.now();
        if let Some(reason) = g.pairing.as_ref().and_then(|w| w.ended(now)) {
            self.close_window(g, reason);
        } else if let (Some(w), Some(info)) = (&g.pairing, &mut g.pairing_info) {
            info.pairings = w.pairings;
            info.code_failures = w.code_failures;
        }
    }

    pub fn save_trust(&self, g: &Inner) {
        let _ = g.trust.save(&*self.storage);
    }

    pub fn save_settings(&self, g: &Inner) {
        let _ = persist::save(&*self.storage, KEY_SETTINGS, &g.settings);
    }

    /// Periodic work: `level`, `status`, pairing expiry (protocol.md §7.5,
    /// §7.6, FR-PAIR-3).
    fn tick(&self) {
        let mut g = self.lock();
        if !g.running {
            return;
        }
        self.check_window(&mut g);
        let dbfs = g.audio.take_level();
        for s in g.sessions.values().filter(|s| s.connected) {
            let open = s.gate.as_ref().is_some_and(|g| g.is_flowing());
            s.send(Cmd::Control(Control::Level { dbfs, open }));
        }
        let due = self.now().saturating_sub(g.last_status_ms)
            >= self.cfg.status_interval.as_millis() as u64;
        self.send_status(&mut g, due);
        self.publish(&g);
    }
}

/// The station engine. Cheap to clone; all clones control the same
/// station. Must be started inside a tokio runtime.
#[derive(Clone)]
pub struct StationEngine {
    shared: Arc<Shared>,
}

impl fmt::Debug for StationEngine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StationEngine")
            .field("device_id", &self.shared.device_id)
            .finish_non_exhaustive()
    }
}

/// Reads the trusted listener list from `storage` without starting an engine.
pub fn stored_trusted_listeners(
    storage: &dyn Storage,
) -> Result<Vec<TrustedListener>, StorageError> {
    Ok(TrustStore::load(storage)?.list().to_vec())
}

fn cert_names(endpoints: &[Endpoint]) -> Vec<Host> {
    let mut names = vec![Host::Dns("localhost".into())];
    names.push(Host::Ipv4(std::net::Ipv4Addr::LOCALHOST));
    for e in endpoints {
        if !names.contains(&e.host) {
            names.push(e.host.clone());
        }
    }
    names
}

fn load_or_create_cert(
    storage: &dyn Storage,
    endpoints: &[Endpoint],
) -> Result<StationCert, StationError> {
    if let Some(rec) = persist::load::<CertRecord>(storage, KEY_CERT)?
        && let Ok(c) = StationCert::from_pem(&rec.cert_pem, &rec.key_pem)
    {
        return Ok(c);
    }
    let cert = StationCert::generate(&cert_names(endpoints))?;
    persist::save(
        storage,
        KEY_CERT,
        &CertRecord {
            cert_pem: cert.cert_pem().to_owned(),
            key_pem: cert.key_pem().to_owned(),
        },
    )?;
    Ok(cert)
}

impl StationEngine {
    /// Loads (or creates) the device key, trust list and settings, binds
    /// the TCP and web ports, starts mDNS and begins serving.
    pub async fn start(
        cfg: StationConfig,
        storage: Arc<dyn Storage>,
        clock: Arc<dyn Clock>,
    ) -> Result<Self, StationError> {
        let key = persist::load_or_create_device_key(&*storage)?;
        let device_id = key.device_id().as_str().to_owned();
        let trust = TrustStore::load(&*storage)?;
        let settings = persist::load::<StationSettings>(&*storage, KEY_SETTINGS)?
            .unwrap_or_else(|| StationSettings::defaults(&cfg.name));

        let tcp = StationListener::bind(cfg.tcp)?;
        let tcp_port = tcp.port();
        let endpoints = match &cfg.endpoints {
            Some(e) => e.clone(),
            None if cfg.tcp.scope == nestwatch_net::BindScope::Loopback => {
                vec![Endpoint::new(
                    std::net::IpAddr::from(std::net::Ipv4Addr::LOCALHOST),
                    tcp_port,
                )]
            }
            None => nestwatch_net::local::station_endpoints(tcp_port, cfg.tailscale_timeout).await,
        };
        let mut web = None;
        let mut fingerprint = None;
        if let Some(w) = &cfg.web {
            let cert = if w.config.https.is_some() {
                Some(load_or_create_cert(&*storage, &endpoints)?)
            } else {
                None
            };
            fingerprint = cert.as_ref().map(StationCert::fingerprint);
            web = Some(WebServer::start(w.config.clone(), w.assets.clone(), cert.as_ref()).await?);
        }
        let http_port = web.as_ref().and_then(WebServer::http_port);
        let https_port = web.as_ref().and_then(WebServer::https_port);

        let gain = settings.gain();
        let inner = Inner {
            running: true,
            settings,
            trust,
            pairing: None,
            pairing_info: None,
            limiter: RateLimiter::default(),
            sessions: BTreeMap::new(),
            audio: Audio::new(gain),
            talk: Talk::default(),
            camera: Camera::default(),
            battery: None,
            charging: None,
            last_status: None,
            last_status_ms: clock.now_ms(),
            advertiser: None,
            tcp_port,
            http_port,
            https_port,
            fingerprint,
            endpoints,
        };
        let (state_tx, _) = watch::channel(StationState {
            name: String::new(),
            device_id: device_id.clone(),
            running: true,
            tcp_port,
            http_port,
            https_port,
            https_fingerprint: None,
            endpoints: Vec::new(),
            mdns: false,
            pairing: None,
            listeners: Vec::new(),
            camera_on: false,
            talker: None,
            level_dbfs: nestwatch_media::level::LEVEL_FLOOR_DBFS,
            battery: None,
            charging: None,
            settings: StationSettings::defaults(""),
            camera_availability: CameraAvailability::Available,
        });
        let (events, _) = broadcast::channel(256);
        let (shutdown, _) = watch::channel(false);
        let shared = Arc::new(Shared {
            cfg,
            storage,
            clock,
            key,
            device_id,
            inner: Mutex::new(inner),
            state_tx,
            events,
            next_session: AtomicU64::new(1),
            shutdown,
        });
        {
            let mut g = shared.lock();
            shared.update_mdns(&mut g, true);
            shared.publish(&g);
        }
        spawn_accept_tcp(shared.clone(), tcp);
        if let Some(web) = web {
            spawn_accept_web(shared.clone(), web);
        }
        spawn_ticker(shared.clone());
        Ok(Self { shared })
    }

    /// Subscribes to state snapshots.
    pub fn state(&self) -> watch::Receiver<StationState> {
        self.shared.state_tx.subscribe()
    }

    /// The current snapshot.
    pub fn snapshot(&self) -> StationState {
        self.shared.state_tx.borrow().clone()
    }

    /// Subscribes to events.
    pub fn events(&self) -> broadcast::Receiver<StationEvent> {
        self.shared.events.subscribe()
    }

    /// The station's device ID (protocol.md §2.2).
    pub fn device_id(&self) -> &str {
        &self.shared.device_id
    }

    /// The station's static public key.
    pub fn public_key(&self) -> nestwatch_crypto::PublicKey {
        *self.shared.key.public_key()
    }

    // ----- pairing ---------------------------------------------------------

    /// Opens pairing mode (FR-PAIR-1..3): a fresh invite token and code,
    /// with the code's Argon2id PSK derived once, now. Returns the open
    /// window's info if one is already open.
    pub async fn open_pairing(&self) -> Result<PairingInfo, StationError> {
        {
            let mut g = self.shared.lock();
            if !g.running {
                return Err(StationError::Stopped);
            }
            self.shared.check_window(&mut g);
            if let Some(info) = &g.pairing_info {
                return Ok(info.clone());
            }
        }
        let token = InviteToken::generate().map_err(|_| StationError::Rng)?;
        let code = PairingCode::generate().map_err(|_| StationError::Rng)?;
        let id = self.shared.key.device_id();
        let c = code.clone();
        let psk = tokio::task::spawn_blocking(move || c.psk(&id))
            .await
            .map_err(|e| StationError::Invalid(e.to_string()))?;
        let shared = &self.shared;
        let mut g = shared.lock();
        if !g.running {
            return Err(StationError::Stopped);
        }
        let now = shared.now();
        let limits = WindowLimits {
            duration_ms: shared.cfg.pairing_window.as_millis() as u64,
            max_pairings: shared.cfg.max_pairings,
            max_code_failures: shared.cfg.max_code_failures,
        };
        let invitation = Invitation {
            station_key: *shared.key.public_key().as_bytes(),
            token: *token.as_bytes(),
            name: g.settings.name.clone(),
            endpoints: g
                .endpoints
                .iter()
                .take(nestwatch_proto::invite::MAX_INVITE_ENDPOINTS)
                .cloned()
                .collect(),
        };
        let invite_uri = invitation
            .to_uri()
            .map_err(|e| StationError::Invalid(e.to_string()))?;
        let web_url = match (g.http_port, g.endpoints.first()) {
            (Some(port), Some(e)) => invitation
                .to_web_url(&format!("http://{}:{port}", e.host))
                .ok(),
            _ => None,
        };
        let info = PairingInfo {
            invite_uri,
            web_url,
            code: code.display().to_string(),
            closes_at_ms: now + limits.duration_ms,
            pairings: 0,
            code_failures: 0,
        };
        g.pairing = Some(PairingWindow::new(token, psk, now, limits));
        g.pairing_info = Some(info.clone());
        shared.update_mdns(&mut g, false);
        shared.emit(StationEvent::PairingOpened);
        shared.publish(&g);
        Ok(info)
    }

    /// Closes pairing mode (FR-PAIR-3).
    pub fn close_pairing(&self) {
        let mut g = self.shared.lock();
        self.shared.close_window(&mut g, PairingEnd::Closed);
    }

    // ----- trust -----------------------------------------------------------

    /// Trusted listeners (FR-ST-7).
    pub fn trusted_listeners(&self) -> Vec<TrustedListener> {
        self.shared.lock().trust.list().to_vec()
    }

    /// Revokes a listener by device ID (FR-PAIR-6): its sessions get
    /// `close revoked` (protocol.md §7.11) and it is rejected from now on.
    /// Returns whether it was trusted.
    pub fn revoke(&self, listener_id: &str) -> bool {
        let shared = &self.shared;
        let mut g = shared.lock();
        let Some(key) = g.trust.find_id(listener_id) else {
            return false;
        };
        let removed = g.trust.remove(&key);
        shared.save_trust(&g);
        for s in g.sessions.values().filter(|s| s.key == Some(key)) {
            s.send(Cmd::Close(CloseReason::Revoked, None));
        }
        if let Some(l) = removed {
            shared.emit(StationEvent::ListenerRevoked { listener_id: l.id });
        }
        shared.publish(&g);
        true
    }

    // ----- settings --------------------------------------------------------

    /// Renames the station (FR-ST-7); listeners get a new `status`.
    pub fn rename(&self, name: &str) -> Result<(), StationError> {
        let name = clamp_name(name);
        if name.is_empty() {
            return Err(StationError::Invalid("empty name".into()));
        }
        let shared = &self.shared;
        let mut g = shared.lock();
        g.settings.name = name;
        shared.save_settings(&g);
        shared.update_mdns(&mut g, true);
        shared.send_status(&mut g, false);
        shared.publish(&g);
        Ok(())
    }

    /// Reports battery level and charging state (FR-ST-6).
    pub fn set_battery(&self, percent: Option<u8>, charging: Option<bool>) {
        let shared = &self.shared;
        let mut g = shared.lock();
        g.battery = percent.map(|p| p.min(100));
        g.charging = charging;
        shared.send_status(&mut g, false);
        shared.publish(&g);
    }

    /// Turns the camera feature on or off entirely (FR-ST-8).
    pub fn set_camera_enabled(&self, enabled: bool) {
        let shared = &self.shared;
        let mut g = shared.lock();
        g.settings.camera_enabled = enabled;
        shared.save_settings(&g);
        if !enabled {
            shared.stop_all_video(&mut g, nestwatch_proto::control::VideoStateKind::Disabled);
        }
        shared.publish(&g);
    }

    /// Turns talk-back on or off entirely (FR-ST-8).
    pub fn set_talk_enabled(&self, enabled: bool) {
        let shared = &self.shared;
        let mut g = shared.lock();
        g.settings.talk_enabled = enabled;
        shared.save_settings(&g);
        if !enabled && let Some(id) = g.talk.talker {
            shared.release_talk(&mut g, id, nestwatch_proto::control::TalkStatus::Disabled);
        }
        shared.publish(&g);
    }

    /// Sets the talk-back playback volume, linear 0..1 (FR-TB-3).
    pub fn set_talkback_volume(&self, volume: f32) {
        let shared = &self.shared;
        let mut g = shared.lock();
        let v = if volume.is_nan() {
            0.0
        } else {
            volume.clamp(0.0, 1.0)
        };
        g.settings.talkback_volume = v;
        shared.save_settings(&g);
        shared.publish(&g);
    }

    /// Sets the microphone gain (FR-AU-5).
    pub fn set_gain(&self, gain: GainSetting) {
        let shared = &self.shared;
        let mut g = shared.lock();
        g.settings.gain_db = gain_code(gain);
        g.audio.set_gain(gain);
        shared.save_settings(&g);
        shared.publish(&g);
    }

    /// Reports whether the camera can be used (iOS background, permission).
    pub fn set_camera_availability(&self, availability: CameraAvailability) {
        let shared = &self.shared;
        let mut g = shared.lock();
        g.camera.availability = availability;
        match availability {
            CameraAvailability::Available => {}
            CameraAvailability::Unavailable => shared.stop_all_video(
                &mut g,
                nestwatch_proto::control::VideoStateKind::Unavailable,
            ),
            CameraAvailability::Denied => {
                shared.stop_all_video(&mut g, nestwatch_proto::control::VideoStateKind::Denied)
            }
        }
        shared.publish(&g);
    }

    // ----- media -----------------------------------------------------------

    /// Feeds interleaved 16-bit microphone samples at the device rate
    /// (FR-ST-1, media.md §1). Call from the capture callback.
    pub fn push_capture(&self, samples: &[i16], rate: u32, channels: u16) {
        self.shared
            .push_capture(media::Samples::I16(samples), rate, channels);
    }

    /// Feeds interleaved float microphone samples in `[-1, 1]`.
    pub fn push_capture_f32(&self, samples: &[f32], rate: u32, channels: u16) {
        self.shared
            .push_capture(media::Samples::F32(samples), rate, channels);
    }

    /// Fills `out` with 16 kHz mono talk-back audio at the talk-back volume
    /// (FR-TB-3); silence when nobody talks. Returns whether a listener is
    /// talking. Call at the speaker's pace.
    pub fn pull_talkback(&self, out: &mut [i16]) -> bool {
        self.shared.pull_talkback(out)
    }

    /// Offers one camera frame (FR-CAM-1..3). It is sent, scaled and
    /// encoded per listener, only to listeners watching and only at their
    /// adapted frame rate.
    pub fn push_camera_frame(&self, frame: &CameraFrame) {
        self.shared.push_camera_frame(frame);
    }

    // ----- lifecycle -------------------------------------------------------

    /// Stops serving: closes pairing, sends `close shutdown` to every
    /// listener and withdraws the mDNS advertisement.
    pub async fn shutdown(&self) {
        let shared = &self.shared;
        {
            let mut g = shared.lock();
            if !g.running {
                return;
            }
            shared.close_window(&mut g, PairingEnd::Shutdown);
            g.running = false;
            for s in g.sessions.values() {
                s.send(Cmd::Close(CloseReason::Shutdown, None));
            }
            g.advertiser = None;
            if g.camera.wanted.take().is_some() {
                shared.emit(StationEvent::CameraNotWanted);
            }
            shared.publish(&g);
        }
        let _ = shared.shutdown.send(true);
        for _ in 0..100 {
            if shared.lock().sessions.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

fn spawn_accept_tcp(shared: Arc<Shared>, mut listener: StationListener) {
    let mut stop = shared.shutdown.subscribe();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                r = listener.accept() => match r {
                    Ok(link) => { tokio::spawn(session::run(shared.clone(), link)); }
                    Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
                },
                () = stopped(&mut stop) => break,
            }
        }
    });
}

fn spawn_accept_web(shared: Arc<Shared>, mut web: WebServer) {
    let mut stop = shared.shutdown.subscribe();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                l = web.accept() => match l {
                    Some(link) => { tokio::spawn(session::run(shared.clone(), link)); }
                    None => break,
                },
                () = stopped(&mut stop) => break,
            }
        }
        drop(web);
    });
}

fn spawn_ticker(shared: Arc<Shared>) {
    let mut stop = shared.shutdown.subscribe();
    let period = shared.cfg.level_interval.max(Duration::from_millis(10));
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(period);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = tick.tick() => shared.tick(),
                () = stopped(&mut stop) => break,
            }
        }
    });
}

/// Next engine-local session number.
pub(crate) fn next_session(shared: &Shared) -> u64 {
    shared.next_session.fetch_add(1, Ordering::Relaxed)
}

/// Resolves once the shutdown flag is set (or its sender is gone).
pub(crate) async fn stopped(rx: &mut watch::Receiver<bool>) {
    while !*rx.borrow_and_update() {
        if rx.changed().await.is_err() {
            break;
        }
    }
}
