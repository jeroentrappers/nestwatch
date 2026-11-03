// SPDX-License-Identifier: AGPL-3.0-or-later
//! WebSocket message links on native targets (protocol.md §3.3).

use std::fmt;
use std::pin::Pin;

use futures_util::{Sink, SinkExt, Stream, StreamExt};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_tungstenite::tungstenite::{Error as WsError, Message};

use super::{LinkInfo, WS_MAX_MESSAGE_SIZE, check_len};
use crate::NetError;

type BoxStream = Pin<Box<dyn Stream<Item = Result<Message, WsError>> + Send>>;
type BoxSink = Pin<Box<dyn Sink<Message, Error = WsError> + Send>>;

/// WebSocket configuration used on both sides: frames and messages are
/// limited to [`WS_MAX_MESSAGE_SIZE`], and the write buffer is small so
/// that `send` applies backpressure quickly.
pub(crate) fn server_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_message_size(Some(WS_MAX_MESSAGE_SIZE))
        .max_frame_size(Some(WS_MAX_MESSAGE_SIZE))
        .write_buffer_size(0)
        .max_write_buffer_size(4 * WS_MAX_MESSAGE_SIZE)
}

/// Wraps an established WebSocket into a [`crate::Link`].
pub(crate) fn ws_link<S>(ws: WebSocketStream<S>, info: LinkInfo) -> crate::Link
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (tx, rx) = ws.split();
    crate::Link::Messages(MessageLink {
        reader: MessageReader {
            inner: Box::pin(rx),
        },
        writer: MessageWriter {
            inner: Box::pin(tx),
            closed: false,
        },
        info,
    })
}

fn ws_err(e: WsError) -> NetError {
    match e {
        WsError::Io(e) => NetError::Io(e),
        WsError::ConnectionClosed | WsError::AlreadyClosed => NetError::Closed,
        other => NetError::WebSocket(other.to_string()),
    }
}

/// A WebSocket connection carrying one Noise message per binary message.
#[derive(Debug)]
pub struct MessageLink {
    reader: MessageReader,
    writer: MessageWriter,
    info: LinkInfo,
}

impl MessageLink {
    /// Addresses and TLS state.
    pub fn info(&self) -> &LinkInfo {
        &self.info
    }

    /// See [`crate::Link::recv`].
    pub async fn recv(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        self.reader.recv().await
    }

    /// See [`crate::Link::send`].
    pub async fn send(&mut self, data: &[u8]) -> Result<(), NetError> {
        self.writer.send(data).await
    }

    /// See [`crate::Link::close`].
    pub async fn close(&mut self) -> Result<(), NetError> {
        self.writer.close().await
    }

    /// Splits into read and write halves.
    pub fn split(self) -> (MessageReader, MessageWriter) {
        (self.reader, self.writer)
    }
}

/// Read half of a [`MessageLink`].
pub struct MessageReader {
    inner: BoxStream,
}

impl fmt::Debug for MessageReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageReader").finish_non_exhaustive()
    }
}

impl MessageReader {
    /// Next binary message; `Ok(None)` after a close. Pings are answered
    /// by the WebSocket layer and never surface. Cancel-safe.
    pub async fn recv(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        loop {
            match self.inner.next().await {
                None => return Ok(None),
                Some(Ok(Message::Binary(b))) => return Ok(Some(b.into())),
                Some(Ok(Message::Text(_))) => return Err(NetError::TextMessage),
                Some(Ok(Message::Close(_))) => return Ok(None),
                Some(Ok(Message::Ping(_) | Message::Pong(_) | Message::Frame(_))) => {}
                Some(Err(WsError::ConnectionClosed | WsError::AlreadyClosed)) => return Ok(None),
                Some(Err(e)) => return Err(ws_err(e)),
            }
        }
    }
}

/// Write half of a [`MessageLink`].
pub struct MessageWriter {
    inner: BoxSink,
    closed: bool,
}

impl fmt::Debug for MessageWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MessageWriter")
            .field("closed", &self.closed)
            .finish_non_exhaustive()
    }
}

impl MessageWriter {
    /// Sends one binary message and flushes it to the socket.
    pub async fn send(&mut self, data: &[u8]) -> Result<(), NetError> {
        check_len(data.len())?;
        if self.closed {
            return Err(NetError::Closed);
        }
        self.inner
            .send(Message::Binary(data.to_vec().into()))
            .await
            .map_err(ws_err)
    }

    /// Sends a WebSocket close frame. Closing twice is a no-op.
    pub async fn close(&mut self) -> Result<(), NetError> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        match self.inner.close().await {
            Ok(()) | Err(WsError::ConnectionClosed | WsError::AlreadyClosed) => Ok(()),
            Err(e) => Err(ws_err(e)),
        }
    }
}

impl crate::Link {
    /// Wraps a WebSocket that the caller established itself (for example a
    /// rendezvous connection that turns into a relay pipe after an NWR/1
    /// exchange, rendezvous.md §4.2) into a
    /// [`Messages`](crate::LinkKind::Messages) link. Configure it with
    /// [`websocket_config`] so the 64 KiB limit applies.
    pub fn from_websocket<S>(ws: WebSocketStream<S>, mut info: LinkInfo) -> Self
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        info.kind = crate::LinkKind::Messages;
        ws_link(ws, info)
    }
}

/// The WebSocket configuration every Nestwatch WebSocket uses (frames and
/// messages limited to [`WS_MAX_MESSAGE_SIZE`], small write buffer).
pub fn websocket_config() -> WebSocketConfig {
    server_config()
}
