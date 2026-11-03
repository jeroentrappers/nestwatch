// SPDX-License-Identifier: AGPL-3.0-or-later
//! Error types. None of them carries key material, PSKs, codes or media.

use lull_crypto::{HandshakeError, TransportError};
use lull_proto::FrameError;
use lull_proto::control::ControlError;
use lull_proto::preface::{PrefaceError, Status};
use lull_proto::record::RecordError;
use lull_proto::video::VideoError;
use thiserror::Error;

/// Which deadline expired.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeoutKind {
    /// No listener preface within 5 s (station, protocol.md §4).
    Preface,
    /// Handshake and `hello` exchange not done within 15 s (protocol.md §4).
    Setup,
    /// Nothing received for 15 s (protocol.md §7.9, NFR-REL-1).
    Idle,
}

/// A violation by the peer that ends the session with a `protocol-error`
/// close once the transport is encrypted (protocol.md §6, §7.12).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProtocolError {
    /// Invalid frame length or a WebSocket text message (protocol.md §3).
    #[error("framing: {0}")]
    Framing(#[from] FrameError),
    /// A WebSocket preface message that is not exactly 6 bytes.
    #[error("websocket preface message must be 6 bytes")]
    PrefaceLength,
    /// A record whose body does not parse (protocol.md §6).
    #[error("malformed record: {0}")]
    MalformedRecord(#[from] RecordError),
    /// A record arrived out of the setup order of protocol.md §7.2.
    #[error("expected {expected}, got {got}")]
    Unexpected {
        /// What the setup order requires next.
        expected: &'static str,
        /// What arrived (record or message type).
        got: String,
    },
    /// `hello` with the wrong `role`, or a second `hello`.
    #[error("invalid hello")]
    InvalidHello,
    /// No audio codec in common; codec 2 is mandatory (media.md §2).
    #[error("no common audio codec")]
    NoCommonCodec,
    /// `audio-config` names a codec the listener did not offer.
    #[error("audio-config codec {0} was not offered")]
    UnofferedCodec(u8),
    /// `paired.station_id` does not match the station's static key.
    #[error("paired station_id does not match the station key")]
    StationIdMismatch,
}

/// Why a session failed. Every failure is terminal.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SessionError {
    /// The peer's preface was not an LWP/1 preface.
    #[error("bad preface: {0}")]
    Preface(PrefaceError),
    /// The station answered (listener) or had to answer (station) with a
    /// non-`ok` status (protocol.md §4).
    #[error("refused with status {0:?}")]
    Refused(Status),
    /// Station only: a `trusted` listener whose key is not trusted. Nothing
    /// was sent (protocol.md §5.1).
    #[error("listener key not trusted")]
    Untrusted,
    /// The Noise handshake failed; nothing further was sent.
    #[error("handshake: {0}")]
    Handshake(HandshakeError),
    /// A transport message failed to decrypt (security.md §3).
    #[error("transport: {0}")]
    Transport(TransportError),
    /// The peer violated the protocol; a `protocol-error` close was sent if
    /// the transport was up.
    #[error("protocol: {0}")]
    Protocol(#[from] ProtocolError),
    /// A deadline expired.
    #[error("timeout: {0:?}")]
    Timeout(TimeoutKind),
    /// The CONTROL queue could not drain for 10 s (protocol.md §8).
    #[error("control queue stalled")]
    ControlStalled,
    /// The I/O layer reported the connection as lost.
    #[error("connection lost")]
    ConnectionLost,
    /// A message supplied locally (for example the policy's `status`)
    /// could not be encoded; the session cannot finish setup.
    #[error("local message invalid: {0}")]
    LocalMessage(ControlError),
    /// Input of the other transport flavour was fed to this session.
    #[error("input does not match the session's transport flavour")]
    FlavorMismatch,
}

/// A command could not be carried out. The session is unaffected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SendError {
    /// Setup (protocol.md §7.2) has not finished yet.
    #[error("session not connected yet")]
    NotConnected,
    /// The session is closing or closed.
    #[error("session closed")]
    Closed,
    /// This message type may not be sent in this direction or is managed by
    /// the session itself (`hello`, `close`, setup messages).
    #[error("message {0:?} not allowed here")]
    NotAllowed(String),
    /// AUDIO with the wrong stream for this role (protocol.md §6.1).
    #[error("wrong audio stream for this role")]
    WrongStream,
    /// AUDIO with a codec other than the session's `audio-config` codec.
    #[error("audio codec {0} is not the session codec")]
    WrongCodec(u8),
    /// Listener only: AUDIO stream 1 without `talk-state granted`
    /// (protocol.md §7.8).
    #[error("talk-back not granted")]
    TalkNotGranted,
    /// Station only: the listener did not offer this video codec.
    #[error("video codec {0} not accepted by the listener")]
    VideoCodecNotAccepted(u8),
    /// The record could not be encoded.
    #[error("encode: {0}")]
    Encode(#[from] RecordError),
    /// The video frame could not be fragmented.
    #[error("video: {0}")]
    Video(#[from] VideoError),
}

/// Invalid session configuration.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConfigError {
    /// The local `hello` has the other role.
    #[error("hello role does not match the session role")]
    WrongRole,
    /// The local `hello` does not list audio codec 2 (media.md §2).
    #[error("hello must list audio codec 2")]
    MissingMandatoryCodec,
    /// The local `hello` is out of range (for example a long name).
    #[error("invalid hello: {0}")]
    InvalidHello(#[from] ControlError),
}
