// SPDX-License-Identifier: AGPL-3.0-or-later
//! One connection to a peer: [`Link`] (protocol.md §3.1–§3.3).
//!
//! A link has one of two flavours:
//!
//! - [`LinkKind::Stream`] (TCP, §3.2): an ordered byte stream. [`Link::recv`]
//!   returns chunks with arbitrary boundaries; the session layer frames the
//!   stream itself (§3.1).
//! - [`LinkKind::Messages`] (WebSocket, §3.3): [`Link::recv`] returns exactly
//!   one binary message and [`Link::send`] sends exactly one. A text message
//!   from the peer is reported as [`NetError::TextMessage`].
//!
//! [`Link::send`] waits until the transport has accepted the bytes, which is
//! how backpressure reaches the caller's scheduler (protocol.md §8).
//! [`Link::split`] gives independent read and write halves so one task can
//! receive while another sends. `recv` is cancel-safe; `send` is not.

use std::net::SocketAddr;

use crate::NetError;

#[cfg(not(target_arch = "wasm32"))]
mod stream;
#[cfg(not(target_arch = "wasm32"))]
mod ws_native;
#[cfg(target_arch = "wasm32")]
mod ws_wasm;

#[cfg(not(target_arch = "wasm32"))]
pub use stream::{StreamLink, StreamReader, StreamWriter};
#[cfg(not(target_arch = "wasm32"))]
pub use ws_native::{MessageLink, MessageReader, MessageWriter, websocket_config};
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use ws_native::{server_config, ws_link};
#[cfg(target_arch = "wasm32")]
pub(crate) use ws_wasm::open;
#[cfg(target_arch = "wasm32")]
pub use ws_wasm::{MessageLink, MessageReader, MessageWriter};

/// Largest message a [`LinkKind::Messages`] link sends: one Noise message
/// (protocol.md §3.3, PROTO-2).
pub const MAX_MESSAGE_LEN: usize = nestwatch_proto::frame::MAX_NOISE_MESSAGE_LEN;

/// Largest WebSocket frame and message accepted from a peer (64 KiB). The
/// session layer checks the exact Noise limit.
pub const WS_MAX_MESSAGE_SIZE: usize = 64 * 1024;

/// Flavour of a [`Link`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkKind {
    /// Byte stream (TCP, protocol.md §3.2): the session layer frames it.
    Stream,
    /// Whole binary messages (WebSocket, protocol.md §3.3).
    Messages,
}

/// Facts about a link, for diagnostics and the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkInfo {
    /// Flavour.
    pub kind: LinkKind,
    /// Remote socket address, when known (not in the browser).
    pub peer: Option<SocketAddr>,
    /// Local socket address, when known.
    pub local: Option<SocketAddr>,
    /// Whether the link runs over TLS (`wss://` or the HTTPS port).
    pub tls: bool,
}

/// One connection (see the [module documentation](self)).
#[derive(Debug)]
pub enum Link {
    /// TCP byte stream.
    #[cfg(not(target_arch = "wasm32"))]
    Stream(StreamLink),
    /// WebSocket messages.
    Messages(MessageLink),
}

impl Link {
    /// Flavour of this link.
    pub fn kind(&self) -> LinkKind {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Stream(_) => LinkKind::Stream,
            Self::Messages(_) => LinkKind::Messages,
        }
    }

    /// Addresses and TLS state.
    pub fn info(&self) -> &LinkInfo {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Stream(l) => l.info(),
            Self::Messages(l) => l.info(),
        }
    }

    /// Receives the next chunk (stream) or message (messages). `Ok(None)`
    /// means the peer closed the connection cleanly. Cancel-safe.
    pub async fn recv(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Stream(l) => l.recv().await,
            Self::Messages(l) => l.recv().await,
        }
    }

    /// Sends bytes (stream) or one binary message (messages), waiting until
    /// the transport accepts them.
    pub async fn send(&mut self, data: &[u8]) -> Result<(), NetError> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Stream(l) => l.send(data).await,
            Self::Messages(l) => l.send(data).await,
        }
    }

    /// Closes the sending direction gracefully (TCP FIN or WebSocket close).
    pub async fn close(&mut self) -> Result<(), NetError> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Stream(l) => l.close().await,
            Self::Messages(l) => l.close().await,
        }
    }

    /// Splits into independent read and write halves.
    pub fn split(self) -> (LinkReader, LinkWriter) {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Stream(l) => {
                let (r, w) = l.split();
                (LinkReader::Stream(r), LinkWriter::Stream(w))
            }
            Self::Messages(l) => {
                let (r, w) = l.split();
                (LinkReader::Messages(r), LinkWriter::Messages(w))
            }
        }
    }
}

/// Read half of a [`Link`].
#[derive(Debug)]
pub enum LinkReader {
    /// TCP byte stream.
    #[cfg(not(target_arch = "wasm32"))]
    Stream(StreamReader),
    /// WebSocket messages.
    Messages(MessageReader),
}

impl LinkReader {
    /// See [`Link::recv`].
    pub async fn recv(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Stream(r) => r.recv().await,
            Self::Messages(r) => r.recv().await,
        }
    }
}

/// Write half of a [`Link`].
#[derive(Debug)]
pub enum LinkWriter {
    /// TCP byte stream.
    #[cfg(not(target_arch = "wasm32"))]
    Stream(StreamWriter),
    /// WebSocket messages.
    Messages(MessageWriter),
}

impl LinkWriter {
    /// See [`Link::send`].
    pub async fn send(&mut self, data: &[u8]) -> Result<(), NetError> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Stream(w) => w.send(data).await,
            Self::Messages(w) => w.send(data).await,
        }
    }

    /// See [`Link::close`].
    pub async fn close(&mut self) -> Result<(), NetError> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Stream(w) => w.close().await,
            Self::Messages(w) => w.close().await,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
/// Shows IPv4 peers of dual-stack sockets (`::ffff:a.b.c.d`) as IPv4.
pub(crate) fn canonical(addr: SocketAddr) -> SocketAddr {
    SocketAddr::new(addr.ip().to_canonical(), addr.port())
}

fn check_len(len: usize) -> Result<(), NetError> {
    if len > MAX_MESSAGE_LEN {
        Err(NetError::MessageTooLarge(len))
    } else {
        Ok(())
    }
}
