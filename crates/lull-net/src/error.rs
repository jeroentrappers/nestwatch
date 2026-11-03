// SPDX-License-Identifier: AGPL-3.0-or-later
//! Error type shared by all transports.

use std::io;

use thiserror::Error;

/// Errors of the transport layer. Library errors from dependencies are
/// carried as text so the type is the same on every target.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum NetError {
    /// Socket or file I/O failed.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// The operation did not finish in time.
    #[error("timed out")]
    Timeout,
    /// The peer sent a WebSocket text message (protocol.md §3.3: protocol error).
    #[error("WebSocket text message received")]
    TextMessage,
    /// A message exceeds [`crate::MAX_MESSAGE_LEN`].
    #[error("message of {0} bytes is too large")]
    MessageTooLarge(usize),
    /// The link is already closed.
    #[error("link closed")]
    Closed,
    /// WebSocket protocol or handshake failure.
    #[error("WebSocket error: {0}")]
    WebSocket(String),
    /// TLS failure.
    #[error("TLS error: {0}")]
    Tls(String),
    /// Certificate generation or parsing failed.
    #[error("certificate error: {0}")]
    Certificate(String),
    /// mDNS daemon failure.
    #[error("mDNS error: {0}")]
    Mdns(String),
    /// A URL could not be used.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),
    /// No endpoint (or no resolved address) to try.
    #[error("no endpoints to try")]
    NoEndpoints,
    /// No port in the fallback range could be bound (protocol.md §3.2).
    #[error("no port available in {first}..={last}: {source}")]
    NoPort {
        /// First port tried.
        first: u16,
        /// Last port tried.
        last: u16,
        /// Error for the last port.
        source: io::Error,
    },
    /// Browser API failure (wasm32).
    #[error("browser error: {0}")]
    Browser(String),
}
