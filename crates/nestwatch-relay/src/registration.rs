// SPDX-License-Identifier: AGPL-3.0-or-later
//! What the station registrations (rendezvous.md §4.1) of the native and the
//! browser client share: options, status, the accepted listener and how a
//! server error ends a control connection (OPEN-QUESTIONS RELAY-3).

use std::time::Duration;

use nestwatch_net::Link;

use crate::RelayConfig;
use crate::candidate::Candidate;
use crate::exchange::PeerInfo;
use crate::msg::ErrorCode;

/// Validity of a session offered in `incoming` (§4.2).
pub(crate) const SESSION_VALIDITY: Duration = Duration::from_secs(30);
/// Incoming requests queued for the application at most; more are dropped
/// (the listener retries).
pub(crate) const REQUEST_QUEUE: usize = 16;

/// Options of a `StationRegistration`.
#[derive(Debug, Clone)]
pub struct RegistrationConfig {
    /// The server.
    pub relay: RelayConfig,
    /// Keep-alive interval on the control connection (§4.1: 20 s): a
    /// WebSocket ping natively, a `ping` message in the browser (WEBST-3).
    pub ping_interval: Duration,
    /// Reconnect when nothing (not even a pong) arrived for this long. The
    /// browser client only applies it once the server answered a `ping`.
    pub dead_after: Duration,
    /// First reconnect delay; doubles up to `backoff_max`.
    pub backoff_min: Duration,
    /// Longest reconnect delay.
    pub backoff_max: Duration,
    /// Candidates sent by `StationRegistration::next`'s automatic accept.
    pub candidates: Vec<Candidate>,
}

impl RegistrationConfig {
    /// Defaults of §4.1: ping every 20 s, dead after 50 s, backoff 1..60 s.
    pub fn new(relay: RelayConfig) -> Self {
        Self {
            relay,
            ping_interval: Duration::from_secs(20),
            dead_after: Duration::from_secs(50),
            backoff_min: Duration::from_secs(1),
            backoff_max: Duration::from_secs(60),
            candidates: Vec::new(),
        }
    }
}

/// State of the control connection, for the station UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistrationStatus {
    /// Opening the control connection.
    Connecting,
    /// Registered; listeners can reach the station.
    Registered {
        /// The station's public address as the server sees it.
        observed: String,
    },
    /// The last attempt failed; the next one follows after `retry_in`.
    Retrying {
        /// What went wrong.
        error: String,
        /// Delay before the next attempt.
        retry_in: Duration,
    },
    /// The server refused for good (`mailbox-taken`, `version`); no more
    /// attempts. "Reset relay identity" (a new mailbox) is the way out of
    /// `mailbox-taken`.
    Rejected(ErrorCode),
    /// Stopped by the application.
    Stopped,
}

/// An accepted listener: the relay pipe and the listener's candidates.
#[derive(Debug)]
pub struct Incoming {
    /// The relay pipe ([`Messages`](nestwatch_net::LinkKind::Messages)).
    pub link: Link,
    /// The listener's candidates and observed address.
    pub peer: PeerInfo,
}

/// How one control connection ended.
pub(crate) enum End {
    Stop,
    Retry(String),
    Slow(String),
    Fatal(ErrorCode),
}

/// RELAY-3: stop for good, retry slowly, or retry with backoff.
pub(crate) fn end_for(code: ErrorCode) -> End {
    match code {
        ErrorCode::MailboxTaken | ErrorCode::Version => End::Fatal(code),
        ErrorCode::RateLimited | ErrorCode::Replaced | ErrorCode::Unavailable => {
            End::Slow(code.to_string())
        }
        other => End::Retry(other.to_string()),
    }
}
