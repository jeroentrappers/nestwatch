// SPDX-License-Identifier: AGPL-3.0-or-later
//! Nestwatch station and listener engines (`specs/architecture.md` §1, §3, §5).
//!
//! The engines drive the sans-I/O [`nestwatch_session`] state machines over
//! [`nestwatch_net`] links: bytes received go into the session, the session's
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
//! `web` builds both for `wasm32-unknown-unknown` over the browser
//! WebSocket client: the listener, and the station as a relay-only station
//! (rendezvous.md FR-RV-7, product.md §3.10).

#![cfg_attr(
    // `web` built for a native target (`cargo check`) has no browser
    // WebSocket: the relay paths are stubs and much is unused.
    all(not(feature = "native"), not(target_arch = "wasm32")),
    allow(dead_code, unused_imports)
)]

#[cfg(not(any(feature = "native", feature = "web")))]
compile_error!("enable feature `native` or `web`");

mod clock;
mod persist;
mod pump;
#[cfg(feature = "native")]
mod punch;
mod rt;
mod storage;
mod types;

pub mod listener;
pub mod station;

pub use clock::{Clock, ManualClock, SystemClock};
pub use nestwatch_proto::Endpoint;
pub use rt::{MaybeSend, MaybeSync};
#[cfg(feature = "native")]
pub use storage::DirStorage;
pub use storage::{MemoryStorage, Storage, StorageError};
pub use types::{CameraFacing, ListenMode, TalkStatus, Transport, VideoStateKind};

/// Implementation name sent in `hello.app` (protocol.md §7.2).
pub const APP_NAME: &str = concat!("nestwatch-core/", env!("CARGO_PKG_VERSION"));

/// Storage key of the device key (protocol.md §2.1), shared by both roles.
pub const KEY_DEVICE: &str = "device-key";

/// Loads the device key from `storage`, creating and storing one on first
/// use (protocol.md §2.1, CRYPTO-6).
pub fn load_or_create_device_key(
    storage: &dyn Storage,
) -> Result<nestwatch_crypto::DeviceKey, StorageError> {
    persist::load_or_create_device_key(storage)
}

/// Formats a device ID for display (`abcd-efgh-ijkl-mnop`, protocol.md §2.2).
pub fn grouped_id(id: &str) -> String {
    nestwatch_proto::DeviceId::parse(id)
        .map(|d| d.grouped())
        .unwrap_or_else(|_| id.to_owned())
}
