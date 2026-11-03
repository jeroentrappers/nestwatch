// SPDX-License-Identifier: AGPL-3.0-or-later
//! Small types shared by both engines.

use minicbor::{Decode, Encode};

pub use nestwatch_proto::control::{CameraFacing, TalkStatus, VideoStateKind};

/// Listen mode of one listener for one station (protocol.md §7.4,
/// product.md FR-AU-2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Encode, Decode)]
#[cbor(index_only)]
pub enum ListenMode {
    /// Audio is always sent.
    #[n(0)]
    Continuous,
    /// Audio is sent while the room is louder than the threshold.
    #[default]
    #[n(1)]
    Activated,
}

impl ListenMode {
    pub(crate) fn to_wire(self) -> nestwatch_proto::control::ListenMode {
        match self {
            Self::Continuous => nestwatch_proto::control::ListenMode::Continuous,
            Self::Activated => nestwatch_proto::control::ListenMode::Activated,
        }
    }

    #[cfg(feature = "native")]
    pub(crate) fn from_wire(m: nestwatch_proto::control::ListenMode) -> Self {
        match m {
            nestwatch_proto::control::ListenMode::Continuous => Self::Continuous,
            nestwatch_proto::control::ListenMode::Activated => Self::Activated,
        }
    }
}

/// Transport flavour of a session (protocol.md §3.2, §3.3; rendezvous.md
/// §4.2, §5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Transport {
    /// TCP.
    Tcp,
    /// WebSocket (browser listeners).
    WebSocket,
    /// A relay pipe through the rendezvous server (WebSocket framing).
    Relayed,
    /// The direct QUIC path opened by hole punching (TCP framing).
    Direct,
}

impl From<nestwatch_net::LinkKind> for Transport {
    fn from(k: nestwatch_net::LinkKind) -> Self {
        match k {
            nestwatch_net::LinkKind::Stream => Self::Tcp,
            nestwatch_net::LinkKind::Messages => Self::WebSocket,
        }
    }
}

impl Transport {
    pub(crate) fn flavor(self) -> nestwatch_session::Flavor {
        match self {
            Self::Tcp | Self::Direct => nestwatch_session::Flavor::Tcp,
            Self::WebSocket | Self::Relayed => nestwatch_session::Flavor::WebSocket,
        }
    }
}

/// Truncates a display name to the 64-byte wire limit at a character
/// boundary (protocol.md §7.2).
pub(crate) fn clamp_name(name: &str) -> String {
    let name = name.trim();
    let mut end = name.len().min(nestwatch_proto::control::MAX_NAME_LEN);
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_owned()
}

/// Supported audio codecs as `hello.audio` (media.md §2).
pub(crate) fn audio_codecs() -> Vec<u8> {
    nestwatch_media::codec::supported_codecs()
        .iter()
        .map(|c| c.0)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_clamped_at_char_boundaries() {
        assert_eq!(clamp_name("  Nursery "), "Nursery");
        let long = "é".repeat(40);
        let c = clamp_name(&long);
        assert!(c.len() <= 64 && c.chars().all(|ch| ch == 'é'));
    }
}
