// SPDX-License-Identifier: AGPL-3.0-or-later
//! Plaintext preface exchange and Noise prologue (protocol.md §4).

use thiserror::Error;

/// Magic bytes opening both prefaces (protocol.md §4).
pub const MAGIC: [u8; 4] = *b"NEST";
/// Protocol version carried in the preface (protocol.md §4, §10).
pub const VERSION: u8 = 1;
/// Length of either preface in bytes (protocol.md §4).
pub const PREFACE_LEN: usize = 6;
/// Fixed prefix of the Noise prologue (protocol.md §4).
pub const PROLOGUE_PREFIX: &[u8; 11] = b"nestwatch/1";
/// Length of the Noise prologue: `"nestwatch/1" || listener || station` (protocol.md §4).
pub const PROLOGUE_LEN: usize = PROLOGUE_PREFIX.len() + 2 * PREFACE_LEN;

/// Errors decoding a preface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PrefaceError {
    /// Fewer than [`PREFACE_LEN`] bytes were supplied.
    #[error("preface truncated: {0} of 6 bytes")]
    Truncated(usize),
    /// The first four bytes were not `"NEST"`.
    #[error("bad preface magic")]
    BadMagic,
    /// The peer speaks a version other than [`VERSION`].
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u8),
    /// Unknown handshake mode in a listener preface.
    #[error("unknown handshake mode {0}")]
    UnknownMode(u8),
    /// Unknown status in a station preface.
    #[error("unknown preface status {0}")]
    UnknownStatus(u8),
}

/// Handshake mode requested by the listener (protocol.md §4, §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Mode {
    /// Reconnect of an already paired listener (`IK`, §5.1).
    Trusted = 1,
    /// First pairing with an invitation (`IKpsk2`, §5.2).
    Invite = 2,
    /// First pairing with a pairing code (`XXpsk3`, §5.3).
    Code = 3,
}

impl Mode {
    /// Wire value of the mode.
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Parses a wire value.
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::Trusted),
            2 => Some(Self::Invite),
            3 => Some(Self::Code),
            _ => None,
        }
    }

    /// Whether this mode is a first pairing that requires open pairing mode.
    pub const fn is_pairing(self) -> bool {
        matches!(self, Self::Invite | Self::Code)
    }
}

/// Station answer to the listener preface (protocol.md §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Status {
    /// Proceed with the handshake.
    Ok = 0,
    /// Version not supported; close.
    UnsupportedVersion = 1,
    /// Pairing mode requested but not open; close.
    PairingClosed = 2,
    /// Listener limit reached; close.
    Busy = 3,
    /// Too many failed attempts; retry after [`RATE_LIMIT_RETRY_SECS`]; close.
    RateLimited = 4,
}

/// Seconds a listener waits after [`Status::RateLimited`] (protocol.md §4).
pub const RATE_LIMIT_RETRY_SECS: u64 = 30;

impl Status {
    /// Wire value of the status.
    pub const fn to_u8(self) -> u8 {
        self as u8
    }

    /// Parses a wire value.
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Ok),
            1 => Some(Self::UnsupportedVersion),
            2 => Some(Self::PairingClosed),
            3 => Some(Self::Busy),
            4 => Some(Self::RateLimited),
            _ => None,
        }
    }
}

fn check_header(bytes: &[u8]) -> Result<[u8; PREFACE_LEN], PrefaceError> {
    let raw: [u8; PREFACE_LEN] = bytes
        .get(..PREFACE_LEN)
        .and_then(|b| b.try_into().ok())
        .ok_or(PrefaceError::Truncated(bytes.len()))?;
    if raw[..4] != MAGIC {
        return Err(PrefaceError::BadMagic);
    }
    if raw[4] != VERSION {
        return Err(PrefaceError::UnsupportedVersion(raw[4]));
    }
    Ok(raw)
}

/// Preface sent by the listener: `"NEST" || version || mode` (protocol.md §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ListenerPreface {
    /// Requested handshake mode.
    pub mode: Mode,
}

impl ListenerPreface {
    /// Creates a version-1 listener preface.
    pub const fn new(mode: Mode) -> Self {
        Self { mode }
    }

    /// Encodes the 6 preface bytes.
    pub const fn encode(&self) -> [u8; PREFACE_LEN] {
        [
            MAGIC[0],
            MAGIC[1],
            MAGIC[2],
            MAGIC[3],
            VERSION,
            self.mode.to_u8(),
        ]
    }

    /// Decodes the first 6 bytes of `bytes`; trailing bytes are ignored.
    ///
    /// [`PrefaceError::UnsupportedVersion`] tells a station to answer
    /// [`Status::UnsupportedVersion`].
    pub fn decode(bytes: &[u8]) -> Result<Self, PrefaceError> {
        let raw = check_header(bytes)?;
        let mode = Mode::from_u8(raw[5]).ok_or(PrefaceError::UnknownMode(raw[5]))?;
        Ok(Self { mode })
    }
}

/// Preface sent by the station: `"NEST" || version || status` (protocol.md §4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StationPreface {
    /// Station answer.
    pub status: Status,
}

impl StationPreface {
    /// Creates a version-1 station preface.
    pub const fn new(status: Status) -> Self {
        Self { status }
    }

    /// Encodes the 6 preface bytes.
    pub const fn encode(&self) -> [u8; PREFACE_LEN] {
        [
            MAGIC[0],
            MAGIC[1],
            MAGIC[2],
            MAGIC[3],
            VERSION,
            self.status.to_u8(),
        ]
    }

    /// Decodes the first 6 bytes of `bytes`; trailing bytes are ignored.
    pub fn decode(bytes: &[u8]) -> Result<Self, PrefaceError> {
        let raw = check_header(bytes)?;
        let status = Status::from_u8(raw[5]).ok_or(PrefaceError::UnknownStatus(raw[5]))?;
        Ok(Self { status })
    }
}

/// Builds the Noise prologue `"nestwatch/1" || listener_preface || station_preface`
/// from the raw bytes as sent on the wire (protocol.md §4).
pub fn prologue(
    listener_preface: &[u8; PREFACE_LEN],
    station_preface: &[u8; PREFACE_LEN],
) -> [u8; PROLOGUE_LEN] {
    let mut out = [0u8; PROLOGUE_LEN];
    let tag = PROLOGUE_PREFIX.len();
    out[..tag].copy_from_slice(PROLOGUE_PREFIX);
    out[tag..tag + PREFACE_LEN].copy_from_slice(listener_preface);
    out[tag + PREFACE_LEN..].copy_from_slice(station_preface);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listener_preface_round_trip_all_modes() {
        for mode in [Mode::Trusted, Mode::Invite, Mode::Code] {
            let bytes = ListenerPreface::new(mode).encode();
            assert_eq!(&bytes[..5], b"NEST\x01");
            assert_eq!(bytes[5], mode as u8);
            assert_eq!(ListenerPreface::decode(&bytes).unwrap().mode, mode);
        }
    }

    #[test]
    fn station_preface_round_trip_all_statuses() {
        for v in 0..=4u8 {
            let status = Status::from_u8(v).unwrap();
            let bytes = StationPreface::new(status).encode();
            assert_eq!(bytes, [b'N', b'E', b'S', b'T', 1, v]);
            assert_eq!(StationPreface::decode(&bytes).unwrap().status, status);
        }
        assert_eq!(Status::from_u8(5), None);
    }

    #[test]
    fn preface_errors() {
        assert_eq!(
            ListenerPreface::decode(b"NEST\x01"),
            Err(PrefaceError::Truncated(5))
        );
        assert_eq!(
            ListenerPreface::decode(b"LUL!\x01\x01"),
            Err(PrefaceError::BadMagic)
        );
        assert_eq!(
            ListenerPreface::decode(b"NEST\x02\x01"),
            Err(PrefaceError::UnsupportedVersion(2))
        );
        assert_eq!(
            ListenerPreface::decode(b"NEST\x01\x00"),
            Err(PrefaceError::UnknownMode(0))
        );
        assert_eq!(
            ListenerPreface::decode(b"NEST\x01\x04"),
            Err(PrefaceError::UnknownMode(4))
        );
        assert_eq!(
            StationPreface::decode(b"NEST\x01\x09"),
            Err(PrefaceError::UnknownStatus(9))
        );
    }

    #[test]
    fn decode_ignores_trailing_bytes() {
        let mut bytes = ListenerPreface::new(Mode::Code).encode().to_vec();
        bytes.extend_from_slice(&[0, 42]);
        assert_eq!(ListenerPreface::decode(&bytes).unwrap().mode, Mode::Code);
    }

    #[test]
    fn prologue_is_23_bytes() {
        let l = ListenerPreface::new(Mode::Invite).encode();
        let s = StationPreface::new(Status::Ok).encode();
        let p = prologue(&l, &s);
        assert_eq!(p.len(), 23);
        assert_eq!(&p, b"nestwatch/1NEST\x01\x02NEST\x01\x00");
    }

    #[test]
    fn pairing_modes() {
        assert!(!Mode::Trusted.is_pairing());
        assert!(Mode::Invite.is_pairing());
        assert!(Mode::Code.is_pairing());
    }
}
