// SPDX-License-Identifier: AGPL-3.0-or-later
//! CBOR control messages (protocol.md §7).

use thiserror::Error;

use crate::cbor::{MapReader, MapWriter};
use crate::device_id::DeviceId;

/// Maximum length of a display name in bytes (protocol.md §7.2, §3.5).
pub const MAX_NAME_LEN: usize = 64;
/// Allowed `listen.threshold` range in dBFS (protocol.md §7.4).
pub const THRESHOLD_RANGE: core::ops::RangeInclusive<f32> = -90.0..=0.0;
/// Maximum `listen.hold_ms` (protocol.md §7.4).
pub const MAX_HOLD_MS: u32 = 60_000;
/// Maximum `listen.preroll_ms` (protocol.md §7.4).
pub const MAX_PREROLL_MS: u32 = 2_000;

/// Capability names used in `hello.caps` (protocol.md §7.2).
pub mod caps {
    /// Audio streaming.
    pub const AUDIO: &str = "audio";
    /// Sound activation.
    pub const VOX: &str = "vox";
    /// Talk-back.
    pub const TALK: &str = "talk";
    /// Video.
    pub const VIDEO: &str = "video";
    /// Battery reporting.
    pub const BATTERY: &str = "battery";
}

/// Errors decoding or validating a control message.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ControlError {
    /// The bytes are not well-formed CBOR.
    #[error("malformed CBOR: {0}")]
    Malformed(String),
    /// The data item is not a map (protocol.md §7.1).
    #[error("control message is not a CBOR map")]
    NotAMap,
    /// Bytes follow the single data item.
    #[error("trailing bytes after control message")]
    TrailingBytes,
    /// A text key occurs twice (RFC 8949 §5.6).
    #[error("duplicate key {0:?}")]
    DuplicateKey(String),
    /// A required key is missing (protocol.md §7.1).
    #[error("missing required key {0:?}")]
    MissingKey(&'static str),
    /// A known key has a value of the wrong CBOR type.
    #[error("wrong type for key {0:?}")]
    WrongType(&'static str),
    /// A value is outside its specified range.
    #[error("value out of range for key {0:?}")]
    OutOfRange(&'static str),
    /// A text value is not one of the specified alternatives.
    #[error("invalid value for key {0:?}")]
    InvalidValue(&'static str),
}

macro_rules! text_enum {
    ($(#[$m:meta])* $name:ident { $($(#[$vm:meta])* $var:ident = $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        pub enum $name { $($(#[$vm])* $var),+ }

        impl $name {
            /// Wire string.
            pub const fn as_str(self) -> &'static str {
                match self { $(Self::$var => $s),+ }
            }

            /// Parses a wire string.
            #[allow(clippy::should_implement_trait)]
            pub fn from_str(s: &str) -> Option<Self> {
                match s { $($s => Some(Self::$var),)+ _ => None }
            }
        }
    };
}

text_enum!(
    /// `hello.role` (protocol.md §7.2).
    Role {
        /// A station.
        Station = "station",
        /// A listener.
        Listener = "listener",
    }
);

text_enum!(
    /// `listen.mode` (protocol.md §7.4).
    ListenMode {
        /// Always send audio.
        Continuous = "continuous",
        /// Sound activation.
        Activated = "activated",
    }
);

text_enum!(
    /// `video-options.camera` (protocol.md §7.7).
    CameraFacing {
        /// Front camera.
        Front = "front",
        /// Back camera.
        Back = "back",
    }
);

text_enum!(
    /// `video-state.state` (protocol.md §7.7).
    VideoStateKind {
        /// Frames are being sent.
        Streaming = "streaming",
        /// Video stopped.
        Stopped = "stopped",
        /// No camera available.
        Unavailable = "unavailable",
        /// OS permission denied.
        Denied = "denied",
        /// Disabled by user setting.
        Disabled = "disabled",
    }
);

text_enum!(
    /// `talk-state.state` (protocol.md §7.8).
    TalkStatus {
        /// The requester may talk.
        Granted = "granted",
        /// Another listener is talking.
        Busy = "busy",
        /// Nobody is talking.
        Idle = "idle",
        /// Talk-back disabled on the station.
        Disabled = "disabled",
    }
);

/// `close.reason` (protocol.md §7.12). Unknown reasons are kept as
/// [`CloseReason::Other`] so a close is never rejected.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CloseReason {
    /// Peer shuts down.
    Shutdown,
    /// Listener was revoked (protocol.md §7.11).
    Revoked,
    /// Peer violated the protocol.
    ProtocolError,
    /// Same listener connected again (protocol.md §9).
    Replaced,
    /// Session idle.
    Idle,
    /// A reason not defined in NWP/1.
    Other(String),
}

impl CloseReason {
    /// Wire string.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Shutdown => "shutdown",
            Self::Revoked => "revoked",
            Self::ProtocolError => "protocol-error",
            Self::Replaced => "replaced",
            Self::Idle => "idle",
            Self::Other(s) => s,
        }
    }

    /// Parses a wire string.
    pub fn parse(s: &str) -> Self {
        match s {
            "shutdown" => Self::Shutdown,
            "revoked" => Self::Revoked,
            "protocol-error" => Self::ProtocolError,
            "replaced" => Self::Replaced,
            "idle" => Self::Idle,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// `hello` (protocol.md §7.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    /// Display name, at most [`MAX_NAME_LEN`] bytes.
    pub name: String,
    /// Sender role.
    pub role: Role,
    /// Implementation and version, for diagnostics.
    pub app: String,
    /// Capability names (see [`caps`]); unknown names are preserved.
    pub caps: Vec<String>,
    /// Supported audio codecs in preference order.
    pub audio: Vec<u8>,
    /// Supported video codecs.
    pub video: Vec<u8>,
}

impl Hello {
    /// Whether `caps` contains `cap`.
    pub fn has_cap(&self, cap: &str) -> bool {
        self.caps.iter().any(|c| c == cap)
    }
}

/// `listen` (protocol.md §7.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Listen {
    /// Continuous or sound-activated audio.
    pub mode: ListenMode,
    /// Activation threshold in dBFS, within [`THRESHOLD_RANGE`].
    pub threshold: f32,
    /// Hold time, at most [`MAX_HOLD_MS`].
    pub hold_ms: u32,
    /// Pre-roll, at most [`MAX_PREROLL_MS`].
    pub preroll_ms: u32,
}

impl Default for Listen {
    /// Product defaults: activated, -50 dBFS, 4 s hold, 500 ms pre-roll
    /// (product.md FR-AU-4).
    fn default() -> Self {
        Self {
            mode: ListenMode::Activated,
            threshold: -50.0,
            hold_ms: 4_000,
            preroll_ms: 500,
        }
    }
}

/// `status` (protocol.md §7.6).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StationStatus {
    /// Current station name.
    pub name: String,
    /// Battery percent (0..=100), `None` if unknown.
    pub battery: Option<u8>,
    /// Charging state, `None` if unknown.
    pub charging: Option<bool>,
    /// Number of connected listeners.
    pub listeners: u32,
    /// Camera currently sending to anyone.
    pub camera: bool,
    /// Name of the listener talking.
    pub talker: Option<String>,
}

/// `video-start` (protocol.md §7.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoStart {
    /// Maximum width.
    pub max_w: u32,
    /// Maximum height.
    pub max_h: u32,
    /// Maximum frame rate.
    pub max_fps: u32,
    /// Accepted video codecs.
    pub codecs: Vec<u8>,
}

/// `video-state` (protocol.md §7.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoState {
    /// Video state.
    pub state: VideoStateKind,
    /// Codec in use.
    pub codec: Option<u8>,
    /// Frame width.
    pub w: Option<u32>,
    /// Frame height.
    pub h: Option<u32>,
    /// Frame rate.
    pub fps: Option<u32>,
}

/// A control message (protocol.md §7).
#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    /// `hello` (§7.2).
    Hello(Hello),
    /// `audio-config` (§7.3).
    AudioConfig {
        /// Codec of stream 0 and stream 1.
        codec: u8,
    },
    /// `listen` (§7.4).
    Listen(Listen),
    /// `level` (§7.5).
    Level {
        /// Peak short-term level in dBFS.
        dbfs: f32,
        /// Audio currently flows to this listener.
        open: bool,
    },
    /// `activity` (§7.5).
    Activity {
        /// Sound activation opened (`true`) or closed.
        open: bool,
    },
    /// `status` (§7.6).
    Status(StationStatus),
    /// `video-start` (§7.7).
    VideoStart(VideoStart),
    /// `video-stop` (§7.7).
    VideoStop,
    /// `video-options` (§7.7).
    VideoOptions {
        /// Camera to use.
        camera: CameraFacing,
        /// Night image.
        night: bool,
    },
    /// `video-state` (§7.7).
    VideoState(VideoState),
    /// `talk-request` (§7.8).
    TalkRequest,
    /// `talk-release` (§7.8).
    TalkRelease,
    /// `talk-state` (§7.8).
    TalkState {
        /// Talk-back state.
        state: TalkStatus,
        /// Name of the talking listener.
        by: Option<String>,
    },
    /// `paired` (§7.10).
    Paired {
        /// The station's device ID.
        station_id: DeviceId,
    },
    /// `unpair` (§7.10).
    Unpair,
    /// `close` (§7.12).
    Close {
        /// Why the session ends.
        reason: CloseReason,
        /// Optional diagnostic text.
        detail: Option<String>,
    },
    /// A message type this implementation does not know; receivers ignore it
    /// (§7.1, §10).
    Unknown {
        /// The `t` value.
        t: String,
    },
}

fn check_name(name: &str, key: &'static str) -> Result<(), ControlError> {
    if name.len() > MAX_NAME_LEN {
        Err(ControlError::OutOfRange(key))
    } else {
        Ok(())
    }
}

fn to_u8(v: u64, key: &'static str) -> Result<u8, ControlError> {
    u8::try_from(v).map_err(|_| ControlError::OutOfRange(key))
}

fn to_u32(v: u64, key: &'static str) -> Result<u32, ControlError> {
    u32::try_from(v).map_err(|_| ControlError::OutOfRange(key))
}

fn to_f32(v: f64, key: &'static str) -> Result<f32, ControlError> {
    let x = v as f32;
    if x.is_finite() {
        Ok(x)
    } else {
        Err(ControlError::OutOfRange(key))
    }
}

fn codec_list(m: &MapReader<'_>, key: &'static str) -> Result<Vec<u8>, ControlError> {
    m.uint_array(key)?
        .into_iter()
        .map(|v| to_u8(v, key))
        .collect()
}

fn text_value<T>(
    m: &MapReader<'_>,
    key: &'static str,
    parse: impl Fn(&str) -> Option<T>,
) -> Result<T, ControlError> {
    parse(m.str(key)?).ok_or(ControlError::InvalidValue(key))
}

impl Control {
    /// The `t` value of this message.
    pub fn message_type(&self) -> &str {
        match self {
            Self::Hello(_) => "hello",
            Self::AudioConfig { .. } => "audio-config",
            Self::Listen(_) => "listen",
            Self::Level { .. } => "level",
            Self::Activity { .. } => "activity",
            Self::Status(_) => "status",
            Self::VideoStart(_) => "video-start",
            Self::VideoStop => "video-stop",
            Self::VideoOptions { .. } => "video-options",
            Self::VideoState(_) => "video-state",
            Self::TalkRequest => "talk-request",
            Self::TalkRelease => "talk-release",
            Self::TalkState { .. } => "talk-state",
            Self::Paired { .. } => "paired",
            Self::Unpair => "unpair",
            Self::Close { .. } => "close",
            Self::Unknown { t } => t,
        }
    }

    /// Checks the ranges stated in protocol.md §7.
    pub fn validate(&self) -> Result<(), ControlError> {
        match self {
            Self::Hello(h) => check_name(&h.name, "name"),
            Self::Listen(l) => {
                if !THRESHOLD_RANGE.contains(&l.threshold) {
                    return Err(ControlError::OutOfRange("threshold"));
                }
                if l.hold_ms > MAX_HOLD_MS {
                    return Err(ControlError::OutOfRange("hold_ms"));
                }
                if l.preroll_ms > MAX_PREROLL_MS {
                    return Err(ControlError::OutOfRange("preroll_ms"));
                }
                Ok(())
            }
            Self::Level { dbfs, .. } if !dbfs.is_finite() => Err(ControlError::OutOfRange("dbfs")),
            Self::Status(s) => {
                check_name(&s.name, "name")?;
                if s.battery.is_some_and(|b| b > 100) {
                    return Err(ControlError::OutOfRange("battery"));
                }
                s.talker
                    .as_deref()
                    .map_or(Ok(()), |t| check_name(t, "talker"))
            }
            Self::TalkState { by: Some(by), .. } => check_name(by, "by"),
            _ => Ok(()),
        }
    }

    /// Encodes the message as one CBOR map in deterministic encoding
    /// (protocol.md §7.1, RFC 8949 §4.2.1). Optional keys whose value is
    /// `None` are omitted; nullable required keys are written as `null`.
    pub fn encode(&self) -> Result<Vec<u8>, ControlError> {
        self.validate()?;
        let mut w = MapWriter::new(self.message_type());
        match self {
            Self::Hello(h) => {
                w.str("name", &h.name);
                w.str("role", h.role.as_str());
                w.str("app", &h.app);
                w.str_array("caps", h.caps.iter().map(String::as_str));
                w.uint_array("audio", &h.audio);
                w.uint_array("video", &h.video);
            }
            Self::AudioConfig { codec } => w.uint("codec", (*codec).into()),
            Self::Listen(l) => {
                w.str("mode", l.mode.as_str());
                w.float("threshold", l.threshold);
                w.uint("hold_ms", l.hold_ms.into());
                w.uint("preroll_ms", l.preroll_ms.into());
            }
            Self::Level { dbfs, open } => {
                w.float("dbfs", *dbfs);
                w.bool("open", *open);
            }
            Self::Activity { open } => w.bool("open", *open),
            Self::Status(s) => {
                w.str("name", &s.name);
                match s.battery {
                    Some(b) => w.uint("battery", b.into()),
                    None => w.null("battery"),
                }
                match s.charging {
                    Some(c) => w.bool("charging", c),
                    None => w.null("charging"),
                }
                w.uint("listeners", s.listeners.into());
                w.bool("camera", s.camera);
                w.opt_str("talker", s.talker.as_deref());
            }
            Self::VideoStart(v) => {
                w.uint("max_w", v.max_w.into());
                w.uint("max_h", v.max_h.into());
                w.uint("max_fps", v.max_fps.into());
                w.uint_array("codecs", &v.codecs);
            }
            Self::VideoOptions { camera, night } => {
                w.str("camera", camera.as_str());
                w.bool("night", *night);
            }
            Self::VideoState(v) => {
                w.str("state", v.state.as_str());
                if let Some(c) = v.codec {
                    w.uint("codec", c.into());
                }
                for (key, val) in [("w", v.w), ("h", v.h), ("fps", v.fps)] {
                    if let Some(x) = val {
                        w.uint(key, x.into());
                    }
                }
            }
            Self::TalkState { state, by } => {
                w.str("state", state.as_str());
                if let Some(by) = by {
                    w.str("by", by);
                }
            }
            Self::Paired { station_id } => w.str("station_id", station_id.as_str()),
            Self::Close { reason, detail } => {
                w.str("reason", reason.as_str());
                if let Some(d) = detail {
                    w.str("detail", d);
                }
            }
            Self::VideoStop
            | Self::TalkRequest
            | Self::TalkRelease
            | Self::Unpair
            | Self::Unknown { .. } => {}
        }
        Ok(w.finish())
    }

    /// Decodes one CBOR control message (protocol.md §7.1).
    ///
    /// Unknown keys are ignored and unknown types yield [`Control::Unknown`];
    /// missing required keys, wrong types and out-of-range values are errors.
    pub fn decode(bytes: &[u8]) -> Result<Self, ControlError> {
        let m = MapReader::parse(bytes)?;
        let msg = match m.str("t")? {
            "hello" => Self::Hello(Hello {
                name: m.str("name")?.to_owned(),
                role: text_value(&m, "role", Role::from_str)?,
                app: m.str("app")?.to_owned(),
                caps: m
                    .str_array("caps")?
                    .into_iter()
                    .map(str::to_owned)
                    .collect(),
                audio: codec_list(&m, "audio")?,
                video: codec_list(&m, "video")?,
            }),
            "audio-config" => Self::AudioConfig {
                codec: to_u8(m.uint("codec")?, "codec")?,
            },
            "listen" => Self::Listen(Listen {
                mode: text_value(&m, "mode", ListenMode::from_str)?,
                threshold: to_f32(m.float("threshold")?, "threshold")?,
                hold_ms: to_u32(m.uint("hold_ms")?, "hold_ms")?,
                preroll_ms: to_u32(m.uint("preroll_ms")?, "preroll_ms")?,
            }),
            "level" => Self::Level {
                dbfs: to_f32(m.float("dbfs")?, "dbfs")?,
                open: m.bool("open")?,
            },
            "activity" => Self::Activity {
                open: m.bool("open")?,
            },
            "status" => Self::Status(StationStatus {
                name: m.str("name")?.to_owned(),
                battery: m
                    .nullable_uint("battery")?
                    .map(|b| to_u8(b, "battery"))
                    .transpose()?,
                charging: m.nullable_bool("charging")?,
                listeners: to_u32(m.uint("listeners")?, "listeners")?,
                camera: m.bool("camera")?,
                talker: m.nullable_str("talker")?.map(str::to_owned),
            }),
            "video-start" => Self::VideoStart(VideoStart {
                max_w: to_u32(m.uint("max_w")?, "max_w")?,
                max_h: to_u32(m.uint("max_h")?, "max_h")?,
                max_fps: to_u32(m.uint("max_fps")?, "max_fps")?,
                codecs: codec_list(&m, "codecs")?,
            }),
            "video-stop" => Self::VideoStop,
            "video-options" => Self::VideoOptions {
                camera: text_value(&m, "camera", CameraFacing::from_str)?,
                night: m.bool("night")?,
            },
            "video-state" => Self::VideoState(VideoState {
                state: text_value(&m, "state", VideoStateKind::from_str)?,
                codec: m
                    .opt_uint("codec")?
                    .map(|c| to_u8(c, "codec"))
                    .transpose()?,
                w: m.opt_uint("w")?.map(|v| to_u32(v, "w")).transpose()?,
                h: m.opt_uint("h")?.map(|v| to_u32(v, "h")).transpose()?,
                fps: m.opt_uint("fps")?.map(|v| to_u32(v, "fps")).transpose()?,
            }),
            "talk-request" => Self::TalkRequest,
            "talk-release" => Self::TalkRelease,
            "talk-state" => Self::TalkState {
                state: text_value(&m, "state", TalkStatus::from_str)?,
                by: m.opt_str("by")?.map(str::to_owned),
            },
            "paired" => Self::Paired {
                station_id: DeviceId::parse(m.str("station_id")?)
                    .map_err(|_| ControlError::InvalidValue("station_id"))?,
            },
            "unpair" => Self::Unpair,
            "close" => Self::Close {
                reason: CloseReason::parse(m.str("reason")?),
                detail: m.opt_str("detail")?.map(str::to_owned),
            },
            other => Self::Unknown {
                t: other.to_owned(),
            },
        };
        msg.validate()?;
        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub(crate) fn all_messages() -> Vec<Control> {
        vec![
            Control::Hello(Hello {
                name: "Nursery".into(),
                role: Role::Station,
                app: "nestwatch-test/0.1".into(),
                caps: vec![caps::AUDIO.into(), caps::VOX.into(), caps::VIDEO.into()],
                audio: vec![1, 2],
                video: vec![1],
            }),
            Control::AudioConfig { codec: 2 },
            Control::Listen(Listen::default()),
            Control::Listen(Listen {
                mode: ListenMode::Continuous,
                threshold: -37.25,
                hold_ms: 0,
                preroll_ms: 2000,
            }),
            Control::Level {
                dbfs: -61.3,
                open: false,
            },
            Control::Activity { open: true },
            Control::Status(StationStatus {
                name: "Nursery".into(),
                battery: Some(87),
                charging: Some(false),
                listeners: 2,
                camera: true,
                talker: Some("Dad's phone".into()),
            }),
            Control::Status(StationStatus {
                name: "Nursery".into(),
                battery: None,
                charging: None,
                listeners: 0,
                camera: false,
                talker: None,
            }),
            Control::VideoStart(VideoStart {
                max_w: 640,
                max_h: 480,
                max_fps: 10,
                codecs: vec![1],
            }),
            Control::VideoStop,
            Control::VideoOptions {
                camera: CameraFacing::Back,
                night: true,
            },
            Control::VideoState(VideoState {
                state: VideoStateKind::Streaming,
                codec: Some(1),
                w: Some(640),
                h: Some(480),
                fps: Some(10),
            }),
            Control::VideoState(VideoState {
                state: VideoStateKind::Denied,
                codec: None,
                w: None,
                h: None,
                fps: None,
            }),
            Control::TalkRequest,
            Control::TalkRelease,
            Control::TalkState {
                state: TalkStatus::Busy,
                by: Some("Mum".into()),
            },
            Control::TalkState {
                state: TalkStatus::Idle,
                by: None,
            },
            Control::Paired {
                station_id: DeviceId::parse("abcdefghijklmnop").unwrap(),
            },
            Control::Unpair,
            Control::Close {
                reason: CloseReason::ProtocolError,
                detail: Some("bad record".into()),
            },
            Control::Close {
                reason: CloseReason::Other("future-reason".into()),
                detail: None,
            },
        ]
    }

    #[test]
    fn round_trip_every_message_section_7() {
        for msg in all_messages() {
            let bytes = msg.encode().unwrap();
            assert_eq!(Control::decode(&bytes).unwrap(), msg, "{}", hex(&bytes));
        }
    }

    #[test]
    fn known_encodings() {
        // {"t": "unpair"}
        assert_eq!(
            hex(&Control::Unpair.encode().unwrap()),
            "a1617466756e70616972"
        );
        // {"t": "activity", "open": true}; "t" sorts before "open".
        assert_eq!(
            hex(&Control::Activity { open: true }.encode().unwrap()),
            "a26174686163746976697479646f70656ef5"
        );
        // -50.0 is encoded as a half float.
        let listen = Control::Listen(Listen::default()).encode().unwrap();
        assert!(hex(&listen).contains("697468726573686f6c64f9d240"));
    }

    #[test]
    fn unknown_type_is_surfaced_not_rejected() {
        let mut w = MapWriter::new("night-light");
        w.uint("level", 3);
        assert_eq!(
            Control::decode(&w.finish()).unwrap(),
            Control::Unknown {
                t: "night-light".into()
            }
        );
    }

    #[test]
    fn unknown_keys_are_ignored_section_7_1() {
        let mut w = MapWriter::new("activity");
        w.bool("open", false);
        w.str("future", "x");
        w.str_array("more", ["a", "b"].into_iter());
        assert_eq!(
            Control::decode(&w.finish()).unwrap(),
            Control::Activity { open: false }
        );
    }

    #[test]
    fn missing_keys_are_errors_section_7_1() {
        assert_eq!(
            Control::decode(&MapWriter::new("activity").finish()),
            Err(ControlError::MissingKey("open"))
        );
        assert_eq!(Control::decode(&[0xa0]), Err(ControlError::MissingKey("t")));
        // status.talker is required but nullable.
        let mut w = MapWriter::new("status");
        w.str("name", "n");
        w.null("battery");
        w.null("charging");
        w.uint("listeners", 1);
        w.bool("camera", false);
        assert_eq!(
            Control::decode(&w.finish()),
            Err(ControlError::MissingKey("talker"))
        );
    }

    #[test]
    fn wrong_types_are_errors() {
        let mut w = MapWriter::new("activity");
        w.uint("open", 1);
        assert_eq!(
            Control::decode(&w.finish()),
            Err(ControlError::WrongType("open"))
        );
        let bytes = [0xa1, 0x61, b't', 0x01];
        assert_eq!(Control::decode(&bytes), Err(ControlError::WrongType("t")));
        let mut w2 = MapWriter::new("listen");
        w2.str("mode", "activated");
        w2.uint("threshold", 0);
        w2.uint("hold_ms", 0);
        w2.uint("preroll_ms", 0);
        assert_eq!(
            Control::decode(&w2.finish()),
            Err(ControlError::WrongType("threshold"))
        );
        let mut w3 = MapWriter::new("audio-config");
        w3.float("codec", 2.0);
        assert_eq!(
            Control::decode(&w3.finish()),
            Err(ControlError::WrongType("codec"))
        );
        // Negative integer for a uint.
        let bytes = [
            0xa2, 0x61, b't', 0x6c, b'a', b'u', b'd', b'i', b'o', b'-', b'c', b'o', b'n', b'f',
            b'i', b'g', 0x65, b'c', b'o', b'd', b'e', b'c', 0x20,
        ];
        assert_eq!(
            Control::decode(&bytes),
            Err(ControlError::WrongType("codec"))
        );
    }

    #[test]
    fn invalid_enum_values() {
        let mut w = MapWriter::new("video-options");
        w.str("camera", "side");
        w.bool("night", false);
        assert_eq!(
            Control::decode(&w.finish()),
            Err(ControlError::InvalidValue("camera"))
        );
        let mut w = MapWriter::new("paired");
        w.str("station_id", "not-an-id");
        assert_eq!(
            Control::decode(&w.finish()),
            Err(ControlError::InvalidValue("station_id"))
        );
    }

    fn listen_bytes(threshold: f32, hold: u64, preroll: u64) -> Vec<u8> {
        let mut w = MapWriter::new("listen");
        w.str("mode", "activated");
        w.float("threshold", threshold);
        w.uint("hold_ms", hold);
        w.uint("preroll_ms", preroll);
        w.finish()
    }

    #[test]
    fn listen_ranges_section_7_4() {
        assert!(Control::decode(&listen_bytes(-90.0, 60_000, 2_000)).is_ok());
        assert!(Control::decode(&listen_bytes(0.0, 0, 0)).is_ok());
        assert_eq!(
            Control::decode(&listen_bytes(-90.5, 0, 0)),
            Err(ControlError::OutOfRange("threshold"))
        );
        assert_eq!(
            Control::decode(&listen_bytes(0.5, 0, 0)),
            Err(ControlError::OutOfRange("threshold"))
        );
        assert_eq!(
            Control::decode(&listen_bytes(f32::NAN, 0, 0)),
            Err(ControlError::OutOfRange("threshold"))
        );
        assert_eq!(
            Control::decode(&listen_bytes(-1.0, 60_001, 0)),
            Err(ControlError::OutOfRange("hold_ms"))
        );
        assert_eq!(
            Control::decode(&listen_bytes(-1.0, 0, 2_001)),
            Err(ControlError::OutOfRange("preroll_ms"))
        );
        assert_eq!(
            Control::decode(&listen_bytes(-1.0, u64::MAX, 0)),
            Err(ControlError::OutOfRange("hold_ms"))
        );
        let bad = Control::Listen(Listen {
            hold_ms: 70_000,
            ..Listen::default()
        });
        assert_eq!(bad.encode(), Err(ControlError::OutOfRange("hold_ms")));
    }

    #[test]
    fn float_widths_are_all_accepted() {
        // threshold as f64 -12.5
        let mut bytes = listen_bytes(-12.5, 1, 1);
        let needle = [0xf9, 0xca, 0x40];
        let pos = bytes.windows(3).position(|w| w == needle).unwrap();
        bytes.splice(pos..pos + 3, {
            let mut f = vec![0xfb];
            f.extend_from_slice(&(-12.5f64).to_be_bytes());
            f
        });
        match Control::decode(&bytes).unwrap() {
            Control::Listen(l) => assert_eq!(l.threshold, -12.5),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn name_limit_64_bytes_section_7_2() {
        let ok = "é".repeat(32);
        let too_long = format!("{ok}x");
        let mk = |name: &str| {
            Control::Hello(Hello {
                name: name.into(),
                role: Role::Listener,
                app: "a".into(),
                caps: vec![],
                audio: vec![2],
                video: vec![],
            })
        };
        let bytes = mk(&ok).encode().unwrap();
        assert!(Control::decode(&bytes).is_ok());
        assert_eq!(
            mk(&too_long).encode(),
            Err(ControlError::OutOfRange("name"))
        );
        let mut w = MapWriter::new("hello");
        w.str("name", &too_long);
        w.str("role", "listener");
        w.str("app", "a");
        w.str_array("caps", [].into_iter());
        w.uint_array("audio", &[]);
        w.uint_array("video", &[]);
        assert_eq!(
            Control::decode(&w.finish()),
            Err(ControlError::OutOfRange("name"))
        );
    }

    #[test]
    fn codec_ids_must_fit_u8() {
        let mut w = MapWriter::new("audio-config");
        w.uint("codec", 256);
        assert_eq!(
            Control::decode(&w.finish()),
            Err(ControlError::OutOfRange("codec"))
        );
    }

    #[test]
    fn battery_range() {
        let mut s = StationStatus {
            name: "n".into(),
            battery: Some(101),
            charging: None,
            listeners: 0,
            camera: false,
            talker: None,
        };
        assert_eq!(
            Control::Status(s.clone()).encode(),
            Err(ControlError::OutOfRange("battery"))
        );
        s.battery = Some(100);
        assert!(Control::Status(s).encode().is_ok());
    }

    #[test]
    fn optional_keys_accept_null() {
        let mut w = MapWriter::new("talk-state");
        w.str("state", "idle");
        w.null("by");
        assert_eq!(
            Control::decode(&w.finish()).unwrap(),
            Control::TalkState {
                state: TalkStatus::Idle,
                by: None
            }
        );
    }

    #[test]
    fn malformed_inputs_never_panic() {
        // Truncations and single-byte mutations of every valid message.
        for msg in all_messages() {
            let bytes = msg.encode().unwrap();
            for n in 0..bytes.len() {
                assert!(Control::decode(&bytes[..n]).is_err());
            }
            for i in 0..bytes.len() {
                for flip in [0x01u8, 0x20, 0x80, 0xff] {
                    let mut b = bytes.clone();
                    b[i] ^= flip;
                    let _ = Control::decode(&b);
                }
            }
        }
    }

    #[test]
    fn hello_has_cap() {
        if let Control::Hello(h) = &all_messages()[0] {
            assert!(h.has_cap(caps::VOX));
            assert!(!h.has_cap(caps::TALK));
        }
    }
}
