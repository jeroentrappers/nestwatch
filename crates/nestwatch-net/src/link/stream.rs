// SPDX-License-Identifier: AGPL-3.0-or-later
//! TCP byte-stream links (protocol.md §3.2).

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

use super::{LinkInfo, LinkKind};
use crate::NetError;

/// Bytes read per [`StreamReader::recv`] call at most.
const READ_CHUNK: usize = 16 * 1024;

/// A TCP connection carrying the framed NWP/1 byte stream.
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
        Self {
            reader: StreamReader { inner: r },
            writer: StreamWriter { inner: w },
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
#[derive(Debug)]
pub struct StreamReader {
    inner: OwnedReadHalf,
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
#[derive(Debug)]
pub struct StreamWriter {
    inner: OwnedWriteHalf,
}

impl StreamWriter {
    /// Writes all bytes, waiting while the socket buffer is full.
    pub async fn send(&mut self, data: &[u8]) -> Result<(), NetError> {
        self.inner.write_all(data).await?;
        Ok(())
    }

    /// Shuts down the sending direction (TCP FIN).
    pub async fn close(&mut self) -> Result<(), NetError> {
        self.inner.shutdown().await?;
        Ok(())
    }
}
