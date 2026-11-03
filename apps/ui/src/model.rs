// SPDX-License-Identifier: AGPL-3.0-or-later
//! View models: plain data the components render. Backends map engine
//! snapshots (`nestwatch_core::listener::ListenerState`,
//! `nestwatch_core::station::StationState`) into these, so the components do not
//! depend on the engines and run against a demo backend too.

use std::collections::VecDeque;

/// The role this installation plays (design README §5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// Watches the child.
    Station,
    /// Follows stations.
    Listener,
}

/// Colour theme choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Theme {
    /// Follow the system (`prefers-color-scheme`).
    #[default]
    System,
    /// Night tokens.
    Night,
    /// Day tokens.
    Day,
}

impl Theme {
    /// Value of `data-theme` on the root element, `None` for the system.
    pub fn attr(self) -> Option<&'static str> {
        match self {
            Self::System => None,
            Self::Night => Some("night"),
            Self::Day => Some("day"),
        }
    }
}

/// Which platform the UI runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Platform {
    /// Dioxus desktop: station and listener, discovery, tray.
    Desktop,
    /// Browser listener.
    Web,
}

/// What a backend can do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    /// Platform.
    pub platform: Platform,
    /// Can run a station.
    pub station: bool,
    /// Finds stations over mDNS.
    pub discovery: bool,
    /// Talk-back possible here (a browser needs a secure context).
    pub talk: bool,
}

/// Connection state of a followed station (FR-LS-3, FR-LS-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conn {
    /// Not followed.
    Paused,
    /// First attempt.
    Connecting {
        /// Attempt number.
        attempt: u32,
        /// Routes tried so far in this attempt.
        tried: usize,
    },
    /// Set up.
    Live,
    /// Trying again.
    Reconnecting {
        /// Attempt number.
        attempt: u32,
        /// Routes tried so far in this attempt.
        tried: usize,
    },
    /// The station revoked us.
    NeedsPairing,
}

/// Network path (FR-LS-3, rendezvous.md FR-RV-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetPath {
    /// Local network.
    Lan,
    /// User VPN.
    Vpn,
    /// Hole-punched path across the internet.
    Direct,
    /// Through the relay server.
    Relayed,
}

/// Talk-back state of one tile (FR-TB-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Talk {
    /// Not talking.
    Idle,
    /// Requested, waiting for the grant.
    Requested,
    /// We talk.
    Granted,
    /// Someone else talks.
    Busy(Option<String>),
    /// Turned off on the station (FR-ST-8).
    Disabled,
}

/// Why a camera view is not streaming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoProblem {
    /// Turned off on the station (FR-ST-8).
    Disabled,
    /// No camera.
    Unavailable,
    /// OS permission denied.
    Denied,
    /// Stopped by the station.
    Stopped,
}

/// Camera view state (FR-CAM-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Video {
    /// Not requested.
    Off,
    /// Requested, waiting.
    Starting,
    /// Frames are coming.
    Streaming {
        /// Width.
        w: u32,
        /// Height.
        h: u32,
        /// Frame rate.
        fps: u32,
    },
    /// Not streaming.
    Problem(VideoProblem),
}

impl Video {
    /// Whether a view is open (requested or streaming).
    pub fn is_open(&self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// Listening mode (FR-AU-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Always on.
    Continuous,
    /// Sound activation.
    Activated,
}

/// Per-station listener settings.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TileSettings {
    /// Mode.
    pub mode: Mode,
    /// Threshold dBFS.
    pub threshold_dbfs: f32,
    /// Hold time.
    pub hold_ms: u32,
    /// Pre-roll (kept as is).
    pub preroll_ms: u32,
    /// Linear volume 0..1.
    pub volume: f32,
    /// Muted.
    pub muted: bool,
    /// Sound alerts.
    pub alert_sound: bool,
    /// Unreachable alerts.
    pub alert_unreachable: bool,
    /// Battery alerts.
    pub alert_battery: bool,
}

impl Default for TileSettings {
    fn default() -> Self {
        Self {
            mode: Mode::Activated,
            threshold_dbfs: -50.0,
            hold_ms: 4_000,
            preroll_ms: 500,
            volume: 1.0,
            muted: false,
            alert_sound: true,
            alert_unreachable: true,
            alert_battery: true,
        }
    }
}

/// One station tile (FR-LS-3).
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    /// Station device ID.
    pub id: String,
    /// Name.
    pub name: String,
    /// Connection.
    pub conn: Conn,
    /// Path of the live session.
    pub path: Option<NetPath>,
    /// Route of the live session.
    pub route: Option<String>,
    /// Last reported room level.
    pub level_dbfs: Option<f32>,
    /// Audio flows to us.
    pub open: bool,
    /// Battery percent.
    pub battery: Option<u8>,
    /// Charging.
    pub charging: Option<bool>,
    /// Listeners on the station.
    pub station_listeners: Option<u32>,
    /// Station camera sends to anyone.
    pub camera_on: bool,
    /// Who talks to the station.
    pub talker: Option<String>,
    /// Our talk state.
    pub talk: Talk,
    /// Our camera view.
    pub video: Video,
    /// Station offers talk-back.
    pub can_talk: bool,
    /// Station offers video.
    pub can_video: bool,
    /// Settings.
    pub settings: TileSettings,
    /// Time since the last live session ended (ms), while not live.
    pub down_for_ms: Option<u64>,
    /// Known routes.
    pub routes: usize,
    /// The station is in the sound alert halo (sound flowing / loud).
    pub alerting: bool,
}

/// A station found on the local network (protocol.md §3.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nearby {
    /// Device ID.
    pub id: String,
    /// Name.
    pub name: String,
    /// Pairing open.
    pub pairing: bool,
    /// Already paired.
    pub known: bool,
    /// Addresses (`host:port`).
    pub endpoints: Vec<String>,
}

/// The listener as shown on its home screen.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ListenerModel {
    /// Our name.
    pub name: String,
    /// Our device ID.
    pub device_id: String,
    /// Tiles in a stable order.
    pub tiles: Vec<Tile>,
    /// Solo station.
    pub solo: Option<String>,
    /// mDNS results.
    pub nearby: Vec<Nearby>,
}

impl ListenerModel {
    /// The tile with `id`.
    pub fn tile(&self, id: &str) -> Option<&Tile> {
        self.tiles.iter().find(|t| t.id == id)
    }
}

/// Microphone gain (FR-AU-5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Gain {
    /// Automatic gain control.
    Auto,
    /// Manual dB step: 0, 6, 12 or 18.
    Db(u8),
}

/// Station settings.
#[derive(Debug, Clone, PartialEq)]
pub struct StationSettingsModel {
    /// Camera allowed.
    pub camera_enabled: bool,
    /// Talk-back allowed.
    pub talk_enabled: bool,
    /// Talk-back volume 0..1.
    pub talkback_volume: f32,
    /// Gain.
    pub gain: Gain,
    /// "Reachable through a relay server" (rendezvous.md FR-RV-1).
    pub relay_enabled: bool,
    /// The relay server origin (FR-RV-6).
    pub relay_server: String,
}

/// Relay registration as shown on the station (rendezvous.md FR-RV-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelayView {
    /// The setting is off.
    Off,
    /// Connecting to the server.
    Connecting,
    /// Registered.
    Online,
    /// The last attempt failed.
    Error {
        /// What went wrong.
        message: String,
        /// Only a new identity or another server helps.
        permanent: bool,
    },
}

/// A connected listener row (FR-ST-3).
#[derive(Debug, Clone, PartialEq)]
pub struct ListenerRow {
    /// Session number (stable key).
    pub session: u64,
    /// Device ID.
    pub id: String,
    /// Name.
    pub name: String,
    /// Mode.
    pub mode: Mode,
    /// Threshold dBFS.
    pub threshold_dbfs: f32,
    /// Audio flows to it.
    pub open: bool,
    /// It talks.
    pub talking: bool,
    /// It watches the camera.
    pub watching: bool,
    /// Browser listener.
    pub web: bool,
    /// Connected through the relay server.
    pub relayed: bool,
}

/// A paired listener (FR-ST-7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedRow {
    /// Device ID.
    pub id: String,
    /// Name.
    pub name: String,
    /// Paired at (Unix ms).
    pub paired_unix_ms: u64,
    /// Last seen (Unix ms).
    pub last_seen_unix_ms: u64,
}

/// The open pairing window (FR-PAIR-1..3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairingModel {
    /// `nestwatch://invite?...`.
    pub invite_uri: String,
    /// Browser address with the invitation fragment.
    pub web_url: Option<String>,
    /// The browser listener on the relay server with the invitation
    /// fragment (rendezvous.md FR-RV-5).
    pub relay_web_url: Option<String>,
    /// Code `XXXXX-XXXXX`.
    pub code: String,
    /// Time left.
    pub remaining_ms: u64,
    /// Pairings so far.
    pub pairings: u32,
    /// Pairings allowed per window.
    pub max_pairings: u32,
}

/// Camera availability on the station.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraHere {
    /// Usable.
    Available,
    /// None.
    Unavailable,
    /// Permission denied.
    Denied,
}

/// The station as shown on its screen.
#[derive(Debug, Clone, PartialEq)]
pub struct StationModel {
    /// Name.
    pub name: String,
    /// Device ID (canonical).
    pub device_id: String,
    /// Room level.
    pub level_dbfs: f32,
    /// Connected listeners.
    pub listeners: Vec<ListenerRow>,
    /// Camera sending.
    pub camera_on: bool,
    /// Who talks.
    pub talker: Option<String>,
    /// Pairing window.
    pub pairing: Option<PairingModel>,
    /// Paired listeners.
    pub trusted: Vec<TrustedRow>,
    /// Settings.
    pub settings: StationSettingsModel,
    /// HTTPS fingerprint.
    pub fingerprint: Option<String>,
    /// `http://host:port/` of the browser app.
    pub http_url: Option<String>,
    /// `https://host:port/` of the browser app.
    pub https_url: Option<String>,
    /// Battery.
    pub battery: Option<u8>,
    /// Charging.
    pub charging: Option<bool>,
    /// Camera availability.
    pub camera_here: CameraHere,
    /// Relay registration.
    pub relay: RelayView,
}

/// Station run state as the UI sees it.
#[derive(Debug, Clone, PartialEq)]
pub enum StationRun {
    /// Not started.
    Stopped,
    /// Starting.
    Starting,
    /// Running.
    Running(Box<StationModel>),
    /// Failed to start.
    Failed(String),
}

/// A newest camera frame of one station.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Frame counter.
    pub frame_id: u32,
    /// `data:image/jpeg;base64,...`.
    pub src: String,
    /// Width.
    pub width: u16,
    /// Height.
    pub height: u16,
}

/// Alert kinds (product.md §3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlertKind {
    /// Sound started.
    Sound,
    /// Unreachable.
    Unreachable,
    /// Reachable again.
    Reachable,
    /// Battery low.
    BatteryLow(u8),
}

/// Something to tell the user in the notice area.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoticeKind {
    /// Listener alert.
    Alert {
        /// Station ID.
        station_id: String,
        /// Station name at the time.
        station: String,
        /// Kind.
        kind: AlertKind,
    },
    /// A listener paired with this station (station side, FR-PAIR-5).
    ListenerPaired {
        /// Its name.
        name: String,
        /// Verification number.
        verification: String,
    },
    /// Plain message.
    Info(String),
}

/// A notice with an id for dismissal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// Unique id.
    pub id: u64,
    /// What.
    pub kind: NoticeKind,
}

/// Result of a successful pairing on the listener (FR-PAIR-5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PairOutcome {
    /// Station ID.
    pub station_id: String,
    /// Station name.
    pub name: String,
    /// Verification number `042 917`.
    pub verification: String,
}

/// UI-level errors, mapped to sentences in [`crate::strings`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiError {
    /// Bad invitation.
    InvalidInvite,
    /// Bad code.
    InvalidCode,
    /// Bad station ID.
    InvalidId,
    /// Bad address.
    InvalidAddress,
    /// Could not reach.
    Unreachable,
    /// Pairing closed, busy or rate limited.
    Refused,
    /// Handshake failed.
    Rejected,
    /// Following 8 stations.
    TooMany,
    /// Not connected.
    NotConnected,
    /// Not possible on this platform.
    Unsupported,
    /// Anything else.
    Other(String),
}

impl UiError {
    /// The sentence to show.
    pub fn message(&self) -> String {
        use crate::strings as s;
        match self {
            Self::InvalidInvite => s::ERR_INVALID_INVITE.into(),
            Self::InvalidCode => s::ERR_INVALID_CODE.into(),
            Self::InvalidId => s::ERR_INVALID_ID.into(),
            Self::InvalidAddress => s::ERR_INVALID_ADDRESS.into(),
            Self::Unreachable => s::ERR_UNREACHABLE.into(),
            Self::Refused => s::ERR_REFUSED.into(),
            Self::Rejected => s::ERR_REJECTED.into(),
            Self::TooMany => s::ERR_TOO_MANY.into(),
            Self::NotConnected => s::ERR_NOT_CONNECTED.into(),
            Self::Unsupported => s::ERR_GENERIC.into(),
            Self::Other(m) => format!("{} ({m})", s::ERR_GENERIC),
        }
    }
}

/// Persistent UI preferences (not secrets; stored by the host).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Prefs {
    /// Chosen role; `None` until the welcome screen was answered.
    pub role: Option<Role>,
    /// The medical disclaimer was acknowledged.
    pub disclaimer_ok: bool,
    /// Theme.
    pub theme: Theme,
    /// The station should run on launch (desktop).
    pub station_autostart: bool,
    /// The station setup asked about the relay (rendezvous.md FR-RV-1).
    pub relay_asked: bool,
}

impl Prefs {
    /// `key=value` lines.
    pub fn encode(&self) -> String {
        let role = match self.role {
            None => "none",
            Some(Role::Station) => "station",
            Some(Role::Listener) => "listener",
        };
        let theme = match self.theme {
            Theme::System => "system",
            Theme::Night => "night",
            Theme::Day => "day",
        };
        format!(
            "v=1\nrole={role}\ndisclaimer={}\ntheme={theme}\nstation_autostart={}\nrelay_asked={}\n",
            u8::from(self.disclaimer_ok),
            u8::from(self.station_autostart),
            u8::from(self.relay_asked)
        )
    }

    /// Parses [`encode`](Self::encode) output; unknown or broken lines are
    /// ignored (defaults).
    pub fn decode(s: &str) -> Self {
        let mut p = Self::default();
        for line in s.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            match (k.trim(), v.trim()) {
                ("role", "station") => p.role = Some(Role::Station),
                ("role", "listener") => p.role = Some(Role::Listener),
                ("disclaimer", v) => p.disclaimer_ok = v == "1",
                ("theme", "night") => p.theme = Theme::Night,
                ("theme", "day") => p.theme = Theme::Day,
                ("station_autostart", v) => p.station_autostart = v == "1",
                ("relay_asked", v) => p.relay_asked = v == "1",
                _ => {}
            }
        }
        p
    }
}

/// Recent room levels of one station, for the calibration chart
/// (design README §4 "Calibration").
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LevelHistory {
    points: VecDeque<(u64, f32)>,
}

impl LevelHistory {
    /// Window kept.
    pub const WINDOW_MS: u64 = 60_000;
    /// Minimum spacing between points.
    pub const MIN_STEP_MS: u64 = 200;

    /// Records a level at monotonic time `t_ms`.
    pub fn push(&mut self, t_ms: u64, dbfs: f32) {
        if let Some(&(last, _)) = self.points.back() {
            if t_ms < last {
                return;
            }
            if t_ms - last < Self::MIN_STEP_MS {
                // Keep the peak within one step (levels are peaks, §7.5).
                if let Some(p) = self.points.back_mut() {
                    p.1 = p.1.max(dbfs);
                }
                return;
            }
        }
        self.points.push_back((t_ms, dbfs));
        while self
            .points
            .front()
            .is_some_and(|&(t, _)| t_ms.saturating_sub(t) > Self::WINDOW_MS)
        {
            self.points.pop_front();
        }
    }

    /// Points, oldest first.
    pub fn points(&self) -> impl Iterator<Item = (u64, f32)> + '_ {
        self.points.iter().copied()
    }

    /// Latest time.
    pub fn latest(&self) -> Option<u64> {
        self.points.back().map(|p| p.0)
    }

    /// Number of points.
    pub fn len(&self) -> usize {
        self.points.len()
    }

    /// No points.
    pub fn is_empty(&self) -> bool {
        self.points.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_roundtrip_and_defaults() {
        let p = Prefs {
            role: Some(Role::Station),
            disclaimer_ok: true,
            theme: Theme::Day,
            station_autostart: true,
            relay_asked: true,
        };
        assert_eq!(Prefs::decode(&p.encode()), p);
        assert_eq!(Prefs::decode("garbage\nrole=alien"), Prefs::default());
    }

    #[test]
    fn history_keeps_one_minute_and_peaks() {
        let mut h = LevelHistory::default();
        h.push(1_000, -60.0);
        h.push(1_100, -30.0); // same step: peak kept
        assert_eq!(h.points().collect::<Vec<_>>(), vec![(1_000, -30.0)]);
        h.push(500, -10.0); // backwards: ignored
        for i in 1..=400 {
            h.push(1_000 + i * 250, -50.0);
        }
        let first = h.points().next().unwrap().0;
        assert!(h.latest().unwrap() - first <= LevelHistory::WINDOW_MS);
        assert!(h.len() <= 241);
    }
}
