// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sans-I/O NWP/1 session state machines (`specs/architecture.md` §3).
//!
//! A [`ListenerSession`] (Noise initiator) or [`StationSession`] (responder)
//! owns no socket, timer or thread. The I/O layer drives it with four calls:
//!
//! - feed received data with `handle_bytes` (TCP) or `handle_ws_message`
//!   (WebSocket), and report a dead connection with `handle_connection_lost`;
//! - pull bytes to send with `poll_transmit(now, max)` whenever the socket
//!   is writable, so real socket backpressure reaches the priority queues
//!   (protocol.md §8);
//! - ask `poll_timeout` for the next deadline and call `handle_timeout`
//!   when it passes;
//! - drain typed events with `poll_event`.
//!
//! Time is a monotonic millisecond counter supplied by the caller. Exactly
//! one terminal event (`Closed` or `Failed`) is emitted per session; after
//! it the caller keeps calling `poll_transmit` until it returns `None` (or
//! [`is_finished`](ListenerSession::is_finished) is true) and then closes the
//! transport.

mod common;
mod error;
mod listener;
mod station;

#[cfg(test)]
mod tests;

pub use crate::common::{Flavor, Stats};
pub use error::{ConfigError, ProtocolError, SendError, SessionError, TimeoutKind};
pub use listener::{ListenerConfig, ListenerEvent, ListenerMode, ListenerSession};
pub use station::{StationConfig, StationEvent, StationPolicy, StationSession};

/// The station must have received the listener preface 5 s after accepting
/// the connection (protocol.md §4).
pub const PREFACE_TIMEOUT_MS: u64 = 5_000;
/// Handshake and setup (both `hello`s) must finish 15 s after the session
/// starts (protocol.md §4, OPEN-QUESTIONS SESSION-2).
pub const SETUP_TIMEOUT_MS: u64 = 15_000;
/// Send PING after 5 s without sending anything (protocol.md §7.9).
pub const PING_INTERVAL_MS: u64 = 5_000;
/// Close after 15 s without receiving anything (protocol.md §7.9, NFR-REL-1).
pub const IDLE_TIMEOUT_MS: u64 = 15_000;
/// Close when the CONTROL queue makes no progress for 10 s (protocol.md §8).
pub const CONTROL_STALL_MS: u64 = 10_000;
/// The AUDIO queue holds 500 ms of 20 ms frames (protocol.md §8).
pub const AUDIO_QUEUE_FRAMES: usize = 25;
/// Default VIDEO fragment size. Small enough that a queued AUDIO or CONTROL
/// record waits behind at most one modest fragment.
pub const DEFAULT_VIDEO_FRAGMENT_LEN: usize = 8 * 1024;
/// Close reason sent when a direction's Noise nonce is about to run out
/// (protocol.md §5.6, OPEN-QUESTIONS SESSION-6).
pub const NONCE_LIMIT_REASON: &str = "nonce-limit";
