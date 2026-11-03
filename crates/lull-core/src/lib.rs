// SPDX-License-Identifier: AGPL-3.0-or-later
//! Lull station and listener engines (`specs/architecture.md` §1, §3, §5).
//!
//! The engines drive the sans-I/O [`lull_session`] state machines over
//! [`lull_net`] links: bytes received go into the session, the session's
//! `poll_transmit` output goes back to the link, and `poll_timeout` sets the
//! timers. Everything above one session lives here: trust stores, the
//! pairing window, multi-session scheduling, sound activation per listener,
//! talk-back arbitration, the on-demand camera, reconnect policy, alerts and
//! settings.
//!
//! UIs never call into the network directly. They render immutable state
//! snapshots ([`station::StationState`], [`listener::ListenerState`]) from a
//! `watch` channel, react to events from a `broadcast` channel, and send
//! intents as plain method calls. Audio and video are pushed and pulled
//! (`push_capture`, `pull_playback`, ...); whether a camera or microphone is
//! wanted is signalled through events.
//!
//! Platform glue sits behind small ports: [`Storage`] for named blobs (device
//! key, trust stores, settings) and [`Clock`].
//!
//! Features: `native` (default) runs on tokio and provides both engines;
//! `web` builds the listener engine for `wasm32-unknown-unknown` over the
//! browser WebSocket client.

#[cfg(not(any(feature = "native", feature = "web")))]
compile_error!("enable feature `native` or `web`");

mod clock;
mod persist;
mod pump;
mod rt;
mod storage;
mod types;

pub mod listener;
#[cfg(feature = "native")]
pub mod station;

pub use clock::{Clock, ManualClock, SystemClock};
pub use lull_proto::Endpoint;
pub use rt::{MaybeSend, MaybeSync};
#[cfg(feature = "native")]
pub use storage::DirStorage;
pub use storage::{MemoryStorage, Storage, StorageError};
pub use types::{CameraFacing, ListenMode, TalkStatus, Transport, VideoStateKind};

/// Implementation name sent in `hello.app` (protocol.md §7.2).
pub const APP_NAME: &str = concat!("lull-core/", env!("CARGO_PKG_VERSION"));

/// Storage key of the device key (protocol.md §2.1), shared by both roles.
pub const KEY_DEVICE: &str = "device-key";

/// Loads the device key from `storage`, creating and storing one on first
/// use (protocol.md §2.1, CRYPTO-6).
pub fn load_or_create_device_key(
    storage: &dyn Storage,
) -> Result<lull_crypto::DeviceKey, StorageError> {
    persist::load_or_create_device_key(storage)
}

/// Formats a device ID for display (`abcd-efgh-ijkl-mnop`, protocol.md §2.2).
pub fn grouped_id(id: &str) -> String {
    lull_proto::DeviceId::parse(id)
        .map(|d| d.grouped())
        .unwrap_or_else(|_| id.to_owned())
}
