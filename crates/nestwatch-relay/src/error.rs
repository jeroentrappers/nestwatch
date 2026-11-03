// SPDX-License-Identifier: AGPL-3.0-or-later
//! Errors of the rendezvous client.

use nestwatch_net::NetError;
use thiserror::Error;

use crate::msg::{DecodeError, ErrorCode};

/// Errors of the rendezvous client.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RelayError {
    /// The server address is not a valid `http(s)://` origin.
    #[error("invalid relay server: {0}")]
    InvalidServer(String),
    /// Transport failure (TCP, TLS, WebSocket).
    #[error(transparent)]
    Net(#[from] NetError),
    /// The server refused the request (rendezvous.md §4).
    #[error("relay server refused: {0}")]
    Server(ErrorCode),
    /// The server sent something that is not valid NWR/1 at this point.
    #[error("relay protocol error: {0}")]
    Protocol(String),
    /// No answer in time.
    #[error("relay request timed out")]
    Timeout,
    /// The server closed the connection.
    #[error("relay connection closed")]
    Closed,
}

impl From<DecodeError> for RelayError {
    fn from(e: DecodeError) -> Self {
        Self::Protocol(e.to_string())
    }
}
