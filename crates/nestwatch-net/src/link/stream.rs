// SPDX-License-Identifier: AGPL-3.0-or-later
//! Byte-stream links: TCP (protocol.md §3.2) and any other ordered,
//! reliable byte stream framed like TCP (for example a QUIC stream of the
//! direct path, rendezvous.md §5.2).

use std::fmt;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

use super::{LinkInfo, LinkKind};
use crate::NetError;

/// Bytes read per [`StreamReader::recv`] call at most.
const READ_CHUNK: usize = 16 * 1024;

type BoxRead = Box<dyn AsyncRead + Send + Unpin>;
type BoxWrite = Box<dyn AsyncWrite + Send + Unpin>;

/// A byte stream (normally TCP) carrying the framed NWP/1 byte stream.
#[derive(Debug)]
pub struct StreamLink {
    reader: StreamReader,
    writer: StreamWriter,
    info: LinkInfo,
}

impl StreamLink {
    /// Wraps a connected socket and enables `TCP_NODELAY` (protocol.md §3.2).
    pub fn new(stream: TcpStream) -> Self {
        // Nagle only adds latency to 20 ms audio records; failure is harmless.
        let _ = stream.set_nodelay(true);
        let info = LinkInfo {
            kind: LinkKind::Stream,
            peer: stream.peer_addr().ok().map(super::canonical),
            local: stream.local_addr().ok().map(super::canonical),
            tls: false,
        };
        let (r, w) = stream.into_split();
        Self::from_io(r, w, info)
    }

    /// Wraps the two halves of any ordered, reliable byte stream. `info.kind`
    /// is forced to [`LinkKind::Stream`]. Shutting down `writer` must signal
    /// end of stream to the peer (like a TCP FIN).
    pub fn from_io<R, W>(reader: R, writer: W, mut info: LinkInfo) -> Self
    where
        R: AsyncRead + Send + Unpin + 'static,
        W: AsyncWrite + Send + Unpin + 'static,
    {
        info.kind = LinkKind::Stream;
        Self {
            reader: StreamReader {
                inner: Box::new(reader),
            },
            writer: StreamWriter {
                inner: Box::new(writer),
            },
            info,
        }
    }

    /// Addresses.
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
    pub fn split(self) -> (StreamReader, StreamWriter) {
        (self.reader, self.writer)
    }
}

/// Read half of a [`StreamLink`].
pub struct StreamReader {
    inner: BoxRead,
}

impl fmt::Debug for StreamReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamReader").finish_non_exhaustive()
    }
}

impl StreamReader {
    /// Reads the next chunk; `Ok(None)` at end of stream. Cancel-safe.
    pub async fn recv(&mut self) -> Result<Option<Vec<u8>>, NetError> {
        let mut buf = vec![0; READ_CHUNK];
        let n = self.inner.read(&mut buf).await?;
        if n == 0 {
            return Ok(None);
        }
        buf.truncate(n);
        Ok(Some(buf))
    }
}

/// Write half of a [`StreamLink`].
pub struct StreamWriter {
    inner: BoxWrite,
}

impl fmt::Debug for StreamWriter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamWriter").finish_non_exhaustive()
    }
}

impl StreamWriter {
    /// Writes all bytes, waiting while the socket buffer is full.
    pub async fn send(&mut self, data: &[u8]) -> Result<(), NetError> {
        self.inner.write_all(data).await?;
        self.inner.flush().await?;
        Ok(())
    }

    /// Shuts down the sending direction (TCP FIN, or the stream's equivalent).
    pub async fn close(&mut self) -> Result<(), NetError> {
        self.inner.shutdown().await?;
        Ok(())
    }
}
