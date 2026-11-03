// SPDX-License-Identifier: AGPL-3.0-or-later
//! The station side (product.md §3.1, §3.3, §3.6, §3.7).

use std::net::{IpAddr, Ipv6Addr, SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use lull_core::station::{
    BindScope, CameraAvailability as CoreAvailability, ConnectedListener, ListenConfig, PairingEnd,
    PairingInfo, StationConfig, StationEngine, StationEvent, StationState, TrustedListener,
};
use lull_core::{Clock, Endpoint, SystemClock, Transport};
use lull_media::capture::{GainSetting, GainStep};
use lull_proto::endpoint::{InterfaceAddr, order_endpoints};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

use crate::error::LullError;
use crate::listener::{CameraFacing, ListenMode};
use crate::media::{self, CameraImage};
use crate::runtime::{HEARTBEAT, STATE_INTERVAL, on_rt, rt};
use crate::storage::{ForeignStorage, Storage};

/// Default first TCP port and fallbacks (protocol.md §3.2).
const DEFAULT_PORT: u16 = 47_100;
const PORT_ATTEMPTS: u16 = 10;
/// Pairings per window (FR-PAIR-3).
const MAX_PAIRINGS: u32 = 8;

/// One network interface address of this device, as the platform lists
/// it (Android `LinkProperties`, iOS `getifaddrs`).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct InterfaceAddress {
    /// Interface name (`wlan0`, `tun0`, `utun3`); VPN interfaces are
    /// recognised by name and address (protocol.md §3.5).
    pub name: String,
    /// IP address literal.
    pub ip: String,
}

/// How to start a station.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct StationOptions {
    /// Display name used until the user renames the station.
    pub name: String,
    /// First TCP port; `0` uses 47100 with its fallbacks.
    pub port: u16,
    /// Advertise with the built-in mDNS responder. Apps that advertise
    /// through the platform (Android NSD) pass `false` and publish
    /// [`StationSnapshot::mdns_txt`] themselves.
    pub mdns: bool,
    /// Interface addresses to build invitation endpoints from. Empty: the
    /// engine lists the interfaces itself (desktop).
    pub interfaces: Vec<InterfaceAddress>,
    /// Extra endpoints (`host:port`) appended to invitations, e.g. a VPN
    /// DNS name.
    pub extra_endpoints: Vec<String>,
    /// Listen on loopback only (tests).
    pub loopback: bool,
}

/// Microphone gain (FR-AU-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum Gain {
    /// Automatic gain control (default).
    Auto,
    /// 0 dB.
    Db0,
    /// +6 dB.
    Db6,
    /// +12 dB.
    Db12,
    /// +18 dB.
    Db18,
}

impl Gain {
    fn from_code(db: u8) -> Self {
        match db {
            0 => Self::Db0,
            6 => Self::Db6,
            12 => Self::Db12,
            18 => Self::Db18,
            _ => Self::Auto,
        }
    }

    fn setting(self) -> GainSetting {
        match self {
            Self::Auto => GainSetting::Auto,
            Self::Db0 => GainSetting::Manual(GainStep::Db0),
            Self::Db6 => GainSetting::Manual(GainStep::Db6),
            Self::Db12 => GainSetting::Manual(GainStep::Db12),
            Self::Db18 => GainSetting::Manual(GainStep::Db18),
        }
    }
}

/// Whether the station's camera can be used now (protocol.md §7.7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum CameraAvailability {
    /// It can be started.
    Available,
    /// No camera, or the platform forbids it now.
    Unavailable,
    /// The OS permission was denied.
    Denied,
}

/// Persistent station settings (FR-ST-7, FR-ST-8, FR-TB-3, FR-AU-5).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct StationSettings {
    /// Display name.
    pub name: String,
    /// Camera allowed at all.
    pub camera_enabled: bool,
    /// Talk-back allowed at all.
    pub talk_enabled: bool,
    /// Talk-back volume, linear 0..1.
    pub talkback_volume: f32,
    /// Microphone gain.
    pub gain: Gain,
}

/// The open pairing window (FR-PAIR-1..3).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct PairingSnapshot {
    /// `lull://invite?…` for the QR code and the copyable link.
    pub invite_uri: String,
    /// Browser address with the invitation, if the station serves one.
    pub web_url: Option<String>,
    /// Pairing code `XXXXX-XXXXX`.
    pub code: String,
    /// Time left when the snapshot was taken.
    pub remaining_ms: u64,
    /// Pairings so far.
    pub pairings: u32,
    /// Pairings allowed per window.
    pub max_pairings: u32,
    /// Failed code attempts so far.
    pub code_failures: u32,
}

/// A connected listener (FR-ST-3).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ConnectedListenerInfo {
    /// Engine-local session number.
    pub session: u64,
    /// Device ID.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Its listen mode.
    pub mode: ListenMode,
    /// Its threshold in dBFS (meaningful when sound-activated).
    pub threshold_dbfs: f32,
    /// Audio currently flows to it.
    pub open: bool,
    /// It holds talk-back.
    pub talking: bool,
    /// It has the camera view open.
    pub watching: bool,
    /// Connected over WebSocket (browser).
    pub web: bool,
    /// Remote address.
    pub peer: Option<String>,
    /// Last round-trip time.
    pub rtt_ms: Option<u64>,
}

/// One TXT entry of the station's DNS-SD record (protocol.md §3.4).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TxtEntry {
    /// Key.
    pub key: String,
    /// Value.
    pub value: String,
}

/// Snapshot of the station (architecture.md §5).
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct StationSnapshot {
    /// Display name.
    pub name: String,
    /// Device ID.
    pub device_id: String,
    /// Serving.
    pub running: bool,
    /// Bound TCP port.
    pub tcp_port: u16,
    /// Endpoints in invitations, `host:port`.
    pub endpoints: Vec<String>,
    /// Built-in mDNS advertising runs.
    pub mdns: bool,
    /// TXT data to advertise when the platform does the advertising
    /// (service type `_lull._tcp`, instance name = `name`, port =
    /// `tcp_port`).
    pub mdns_txt: Vec<TxtEntry>,
    /// The open pairing window.
    pub pairing: Option<PairingSnapshot>,
    /// Connected listeners.
    pub listeners: Vec<ConnectedListenerInfo>,
    /// A camera feed is being sent (FR-CAM-2).
    pub camera_on: bool,
    /// Name of the listener talking (FR-TB-3).
    pub talker: Option<String>,
    /// Last level in dBFS (FR-ST-1).
    pub level_dbfs: f32,
    /// Lowest threshold of connected sound-activated listeners: the ring
    /// tick (UI-2).
    pub strictest_threshold_dbfs: Option<f32>,
    /// Sound activation is open for at least one sound-activated listener
    /// (UI-2).
    pub sound_open: bool,
    /// Battery percent reported to listeners.
    pub battery: Option<u8>,
    /// Charging state reported to listeners.
    pub charging: Option<bool>,
    /// Settings.
    pub settings: StationSettings,
    /// What the app last reported about the camera.
    pub camera_availability: CameraAvailability,
}

/// A listener the station trusts (FR-ST-7).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TrustedListenerInfo {
    /// Device ID.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Paired at (Unix ms).
    pub paired_unix_ms: u64,
    /// Last connected (Unix ms).
    pub last_seen_unix_ms: u64,
}

/// What the camera should deliver (media.md §5).
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct CameraRequest {
    /// Largest requested width.
    pub max_w: u32,
    /// Largest requested height.
    pub max_h: u32,
    /// Highest requested frame rate.
    pub max_fps: u32,
    /// Camera to use.
    pub facing: CameraFacing,
    /// Night image requested (the engine applies the filter).
    pub night: bool,
}

/// Why pairing mode ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PairingEndReason {
    /// The user closed it.
    Closed,
    /// Its 10 minutes ran out.
    Expired,
    /// 8 listeners paired.
    Full,
    /// 5 failed code attempts.
    CodeFailures,
    /// The station stopped.
    Shutdown,
}

/// Something that happened on the station.
#[derive(Debug, Clone, PartialEq, uniffi::Enum)]
pub enum StationEventInfo {
    /// Pairing mode opened.
    PairingOpened,
    /// Pairing mode ended.
    PairingClosed {
        /// Why.
        reason: PairingEndReason,
    },
    /// A listener paired; show its verification number (FR-PAIR-5).
    Paired {
        /// Device ID.
        listener_id: String,
        /// Name.
        name: String,
        /// Verification number, `042 917`.
        verification: String,
    },
    /// A listener connected.
    ListenerConnected {
        /// Session.
        session: u64,
        /// Device ID.
        listener_id: String,
        /// Name.
        name: String,
    },
    /// A listener's session ended.
    ListenerDisconnected {
        /// Session.
        session: u64,
        /// Device ID.
        listener_id: String,
        /// Diagnostics.
        reason: String,
    },
    /// A listener revoked by the user.
    ListenerRevoked {
        /// Device ID.
        listener_id: String,
    },
    /// A listener forgot this station.
    ListenerUnpaired {
        /// Device ID.
        listener_id: String,
    },
    /// Start or reconfigure the camera, then feed
    /// [`Station::push_camera_frame`] (FR-CAM-1).
    CameraWanted {
        /// What to deliver.
        request: CameraRequest,
    },
    /// Stop the camera now (FR-CAM-1).
    CameraNotWanted,
    /// A listener started talking; play [`Station::pull_talkback`].
    TalkStarted {
        /// Its name.
        name: String,
    },
    /// Talk-back ended.
    TalkEnded,
}

/// Talk-back audio pulled from the station.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct TalkbackChunk {
    /// 16 kHz mono 16-bit little-endian PCM at the talk-back volume.
    pub pcm: Vec<u8>,
    /// A listener is talking.
    pub talking: bool,
}

/// Receives station state and events. Called on engine threads.
#[uniffi::export(foreign)]
pub trait StationObserver: Send + Sync {
    /// A new snapshot (at most 10 per second, and every second while a
    /// pairing window is open).
    fn on_state(&self, state: StationSnapshot);
    /// An event.
    fn on_event(&self, event: StationEventInfo);
}

/// A running station. Stop it with [`Station::stop`].
#[derive(uniffi::Object)]
pub struct Station {
    engine: StationEngine,
    clock: Arc<dyn Clock>,
    pumps: Mutex<Vec<JoinHandle<()>>>,
    stopped: AtomicBool,
}

impl std::fmt::Debug for Station {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Station")
            .field("engine", &self.engine)
            .finish_non_exhaustive()
    }
}

fn port_free(port: u16) -> bool {
    TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port))).is_ok()
        && TcpListener::bind(SocketAddr::from((Ipv6Addr::UNSPECIFIED, port))).is_ok()
}

fn first_free_port(first: u16) -> Option<u16> {
    (0..PORT_ATTEMPTS)
        .filter_map(|i| first.checked_add(i))
        .find(|p| port_free(*p))
}

fn config(o: &StationOptions) -> Result<StationConfig, LullError> {
    let first = if o.port == 0 { DEFAULT_PORT } else { o.port };
    let mut cfg = StationConfig::new(o.name.clone());
    cfg.mdns = o.mdns && !o.loopback;
    let extra: Vec<Endpoint> = o
        .extra_endpoints
        .iter()
        .map(|e| {
            Endpoint::parse(e).map_err(|_| LullError::Invalid {
                reason: format!("endpoint {e:?}"),
            })
        })
        .collect::<Result<_, _>>()?;
    if o.loopback {
        cfg.tcp = ListenConfig {
            port: 0,
            attempts: 1,
            scope: BindScope::Loopback,
        };
        if !extra.is_empty() {
            return Err(LullError::Invalid {
                reason: "extra endpoints need a fixed port".into(),
            });
        }
        return Ok(cfg);
    }
    if o.interfaces.is_empty() && extra.is_empty() {
        cfg.tcp = ListenConfig::new(first);
        return Ok(cfg);
    }
    // Endpoints are built before the engine binds, so the port is chosen
    // here (protocol.md §3.2 fallbacks) and bound without fallback.
    let port = first_free_port(first).ok_or_else(|| LullError::Network {
        reason: format!("no free port in {first}..{}", first + PORT_ATTEMPTS - 1),
    })?;
    cfg.tcp = ListenConfig {
        port,
        attempts: 1,
        scope: BindScope::All,
    };
    let addrs: Vec<InterfaceAddr> = o
        .interfaces
        .iter()
        .filter_map(|i| {
            Some(InterfaceAddr::new(
                i.name.clone(),
                i.ip.parse::<IpAddr>().ok()?,
            ))
        })
        .collect();
    let mut endpoints = order_endpoints(&[], &addrs, port);
    for e in extra {
        if !endpoints.contains(&e) {
            endpoints.push(e);
        }
    }
    cfg.endpoints = Some(endpoints);
    Ok(cfg)
}

/// Starts a station: loads or creates the device key, trust list and
/// settings from `storage`, binds the TCP port and begins serving.
#[uniffi::export]
pub async fn start_station(
    options: StationOptions,
    storage: Arc<dyn Storage>,
    observer: Arc<dyn StationObserver>,
) -> Result<Arc<Station>, LullError> {
    let cfg = config(&options)?;
    on_rt(async move {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let engine =
            StationEngine::start(cfg, Arc::new(ForeignStorage(storage)), clock.clone()).await?;
        let station = Arc::new(Station {
            pumps: Mutex::new(spawn_pumps(&engine, clock.clone(), observer)),
            engine,
            clock,
            stopped: AtomicBool::new(false),
        });
        Ok(station)
    })
    .await
}

fn spawn_pumps(
    engine: &StationEngine,
    clock: Arc<dyn Clock>,
    observer: Arc<dyn StationObserver>,
) -> Vec<JoinHandle<()>> {
    let state = {
        let mut rx = engine.state();
        let observer = observer.clone();
        rt().spawn(async move {
            loop {
                let s = rx.borrow_and_update().clone();
                let pairing = s.pairing.is_some();
                observer.on_state(snapshot(&s, clock.now_ms()));
                tokio::time::sleep(STATE_INTERVAL).await;
                let changed = if pairing {
                    // Heartbeat for the countdown.
                    tokio::time::timeout(HEARTBEAT, rx.changed())
                        .await
                        .unwrap_or(Ok(()))
                } else {
                    rx.changed().await
                };
                if changed.is_err() {
                    break;
                }
            }
        })
    };
    let events = {
        let mut rx = engine.events();
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
    vec![state, events]
}

#[uniffi::export]
impl Station {
    /// The station's device ID.
    pub fn device_id(&self) -> String {
        self.engine.device_id().to_owned()
    }

    /// The current snapshot.
    pub fn snapshot(&self) -> StationSnapshot {
        snapshot(&self.engine.snapshot(), self.clock.now_ms())
    }

    /// Stops serving: closes pairing, tells every listener and withdraws
    /// advertising. Idempotent.
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

    /// Opens pairing mode (FR-PAIR-1..3), or returns the open window.
    pub async fn open_pairing(&self) -> Result<PairingSnapshot, LullError> {
        let engine = self.engine.clone();
        let info = on_rt(async move { engine.open_pairing().await }).await?;
        Ok(pairing(&info, self.clock.now_ms()))
    }

    /// Closes pairing mode.
    pub fn close_pairing(&self) {
        self.engine.close_pairing();
    }

    /// Trusted listeners (FR-ST-7).
    pub fn trusted_listeners(&self) -> Vec<TrustedListenerInfo> {
        self.engine
            .trusted_listeners()
            .iter()
            .map(trusted)
            .collect()
    }

    /// Revokes a listener (FR-PAIR-6); returns whether it was trusted.
    pub fn revoke(&self, listener_id: String) -> bool {
        self.engine.revoke(&listener_id)
    }

    /// Renames the station (FR-ST-7).
    pub fn rename(&self, name: String) -> Result<(), LullError> {
        Ok(self.engine.rename(&name)?)
    }

    /// Reports battery level and charging state (FR-ST-6).
    pub fn set_battery(&self, percent: Option<u8>, charging: Option<bool>) {
        self.engine
            .set_battery(percent.map(|p| p.min(100)), charging);
    }

    /// Allows or forbids the camera entirely (FR-ST-8).
    pub fn set_camera_enabled(&self, enabled: bool) {
        self.engine.set_camera_enabled(enabled);
    }

    /// Allows or forbids talk-back entirely (FR-ST-8).
    pub fn set_talk_enabled(&self, enabled: bool) {
        self.engine.set_talk_enabled(enabled);
    }

    /// Talk-back volume, linear 0..1 (FR-TB-3).
    pub fn set_talkback_volume(&self, volume: f32) {
        self.engine.set_talkback_volume(volume);
    }

    /// Microphone gain (FR-AU-5).
    pub fn set_gain(&self, gain: Gain) {
        self.engine.set_gain(gain.setting());
    }

    /// Reports whether the camera can be used (permission, platform).
    pub fn set_camera_availability(&self, availability: CameraAvailability) {
        self.engine.set_camera_availability(match availability {
            CameraAvailability::Available => CoreAvailability::Available,
            CameraAvailability::Unavailable => CoreAvailability::Unavailable,
            CameraAvailability::Denied => CoreAvailability::Denied,
        });
    }

    /// Feeds interleaved 16-bit little-endian microphone PCM at the
    /// device rate (FR-ST-1). Call from the capture thread.
    pub fn push_capture_i16(&self, pcm: Vec<u8>, rate: u32, channels: u16) {
        self.engine
            .push_capture(&media::i16_from_le(&pcm), rate, channels);
    }

    /// Feeds interleaved 32-bit float little-endian PCM in `[-1, 1]`.
    pub fn push_capture_f32(&self, pcm: Vec<u8>, rate: u32, channels: u16) {
        self.engine
            .push_capture_f32(&media::f32_from_le(&pcm), rate, channels);
    }

    /// Pulls `frames` samples of 16 kHz mono talk-back audio (silence when
    /// nobody talks). Call at the speaker's pace.
    pub fn pull_talkback(&self, frames: u32) -> TalkbackChunk {
        let mut out = vec![0i16; frames.min(48_000) as usize];
        let talking = self.engine.pull_talkback(&mut out);
        TalkbackChunk {
            pcm: media::i16_to_le(&out),
            talking,
        }
    }

    /// Offers one camera image (only while the camera is wanted). Returns
    /// `false` if the image was malformed.
    pub fn push_camera_frame(&self, image: CameraImage) -> bool {
        match media::to_frame(image) {
            Some(f) => {
                self.engine.push_camera_frame(&f);
                true
            }
            None => false,
        }
    }
}

impl Drop for Station {
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

fn pairing(p: &PairingInfo, now_ms: u64) -> PairingSnapshot {
    PairingSnapshot {
        invite_uri: p.invite_uri.clone(),
        web_url: p.web_url.clone(),
        code: p.code.clone(),
        remaining_ms: p.closes_at_ms.saturating_sub(now_ms),
        pairings: p.pairings,
        max_pairings: MAX_PAIRINGS,
        code_failures: p.code_failures,
    }
}

fn trusted(t: &TrustedListener) -> TrustedListenerInfo {
    TrustedListenerInfo {
        id: t.id.clone(),
        name: t.name.clone(),
        paired_unix_ms: t.paired_unix_ms,
        last_seen_unix_ms: t.last_seen_unix_ms,
    }
}

fn listener(l: &ConnectedListener) -> ConnectedListenerInfo {
    ConnectedListenerInfo {
        session: l.session,
        id: l.id.clone(),
        name: l.name.clone(),
        mode: l.mode.into(),
        threshold_dbfs: l.threshold_dbfs,
        open: l.open,
        talking: l.talking,
        watching: l.watching,
        web: l.transport == Transport::WebSocket,
        peer: l.peer.map(|p| p.to_string()),
        rtt_ms: l.rtt_ms,
    }
}

pub(crate) fn snapshot(s: &StationState, now_ms: u64) -> StationSnapshot {
    let activated = s
        .listeners
        .iter()
        .filter(|l| l.mode == lull_core::ListenMode::Activated);
    let strictest = activated
        .clone()
        .map(|l| l.threshold_dbfs)
        .filter(|t| !t.is_nan())
        .reduce(f32::min);
    let sound_open = activated.clone().any(|l| l.open);
    let txt = lull_proto::DeviceId::parse(&s.device_id)
        .map(|id| {
            lull_proto::StationTxt {
                id,
                pair: s.pairing.is_some(),
                http: s.http_port,
                https: s.https_port,
            }
            .to_pairs()
            .into_iter()
            .map(|(key, value)| TxtEntry { key, value })
            .collect()
        })
        .unwrap_or_default();
    StationSnapshot {
        name: s.name.clone(),
        device_id: s.device_id.clone(),
        running: s.running,
        tcp_port: s.tcp_port,
        endpoints: s.endpoints.iter().map(ToString::to_string).collect(),
        mdns: s.mdns,
        mdns_txt: txt,
        pairing: s.pairing.as_ref().map(|p| pairing(p, now_ms)),
        listeners: s.listeners.iter().map(listener).collect(),
        camera_on: s.camera_on,
        talker: s.talker.clone(),
        level_dbfs: s.level_dbfs,
        strictest_threshold_dbfs: strictest,
        sound_open,
        battery: s.battery,
        charging: s.charging,
        settings: StationSettings {
            name: s.settings.name.clone(),
            camera_enabled: s.settings.camera_enabled,
            talk_enabled: s.settings.talk_enabled,
            talkback_volume: s.settings.talkback_volume,
            gain: Gain::from_code(s.settings.gain_db),
        },
        camera_availability: match s.camera_availability {
            CoreAvailability::Available => CameraAvailability::Available,
            CoreAvailability::Unavailable => CameraAvailability::Unavailable,
            CoreAvailability::Denied => CameraAvailability::Denied,
        },
    }
}

fn event(e: StationEvent) -> StationEventInfo {
    match e {
        StationEvent::PairingOpened => StationEventInfo::PairingOpened,
        StationEvent::PairingClosed { reason } => StationEventInfo::PairingClosed {
            reason: match reason {
                PairingEnd::Closed => PairingEndReason::Closed,
                PairingEnd::Expired => PairingEndReason::Expired,
                PairingEnd::Full => PairingEndReason::Full,
                PairingEnd::CodeFailures => PairingEndReason::CodeFailures,
                PairingEnd::Shutdown => PairingEndReason::Shutdown,
            },
        },
        StationEvent::Paired {
            listener_id,
            name,
            verification,
        } => StationEventInfo::Paired {
            listener_id,
            name,
            verification,
        },
        StationEvent::ListenerConnected {
            session,
            listener_id,
            name,
        } => StationEventInfo::ListenerConnected {
            session,
            listener_id,
            name,
        },
        StationEvent::ListenerDisconnected {
            session,
            listener_id,
            reason,
        } => StationEventInfo::ListenerDisconnected {
            session,
            listener_id,
            reason,
        },
        StationEvent::SessionReplaced {
            listener_id,
            old_session,
        } => StationEventInfo::ListenerDisconnected {
            session: old_session,
            listener_id,
            reason: "replaced".into(),
        },
        StationEvent::ListenerRevoked { listener_id } => {
            StationEventInfo::ListenerRevoked { listener_id }
        }
        StationEvent::ListenerUnpaired { listener_id } => {
            StationEventInfo::ListenerUnpaired { listener_id }
        }
        StationEvent::CameraWanted(r) => StationEventInfo::CameraWanted {
            request: CameraRequest {
                max_w: r.max_w,
                max_h: r.max_h,
                max_fps: r.max_fps,
                facing: r.facing.into(),
                night: r.night,
            },
        },
        StationEvent::CameraNotWanted => StationEventInfo::CameraNotWanted,
        StationEvent::TalkStarted { name } => StationEventInfo::TalkStarted { name },
        StationEvent::TalkEnded => StationEventInfo::TalkEnded,
    }
}
