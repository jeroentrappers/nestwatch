// SPDX-License-Identifier: AGPL-3.0-or-later
//! One UDP socket shared by hole-punching probes and QUIC (rendezvous.md
//! §5.2): a wrapper around quinn's tokio socket that takes probe datagrams
//! out of the receive path and lets the punching code send its own.

use std::fmt;
use std::io::{self, IoSliceMut};
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use quinn::udp::{RecvMeta, Transmit};
use quinn::{AsyncUdpSocket, UdpPoller};
use tokio::sync::mpsc;

/// Probe datagram prefix. The first byte has the QUIC "fixed bit" (0x40)
/// clear, so no QUIC packet ever looks like a probe (OPEN-QUESTIONS
/// RELAY-7).
const PROBE_MAGIC: [u8; 5] = [0x00, b'N', b'W', b'R', b'P'];
/// Length of a probe datagram: magic and kind.
pub(crate) const PROBE_LEN: usize = PROBE_MAGIC.len() + 1;

/// What a probe datagram says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProbeKind {
    /// "Can you hear me?" (sent every 100 ms).
    Probe,
    /// "Yes": the answer to a probe.
    Ack,
}

impl ProbeKind {
    pub(crate) fn datagram(self) -> [u8; PROBE_LEN] {
        let mut d = [0; PROBE_LEN];
        d[..PROBE_MAGIC.len()].copy_from_slice(&PROBE_MAGIC);
        d[PROBE_MAGIC.len()] = match self {
            Self::Probe => 1,
            Self::Ack => 2,
        };
        d
    }

    pub(crate) fn parse(b: &[u8]) -> Option<Self> {
        if b.len() != PROBE_LEN || b[..PROBE_MAGIC.len()] != PROBE_MAGIC {
            return None;
        }
        match b[PROBE_MAGIC.len()] {
            1 => Some(Self::Probe),
            2 => Some(Self::Ack),
            _ => None,
        }
    }
}

/// Probes received: sender address and kind.
pub(crate) type ProbeRx = mpsc::UnboundedReceiver<(SocketAddr, ProbeKind)>;

/// quinn's socket with probes split off into a channel.
pub(crate) struct DemuxSocket {
    inner: Arc<dyn AsyncUdpSocket>,
    probes: mpsc::UnboundedSender<(SocketAddr, ProbeKind)>,
    ipv6: bool,
}

impl fmt::Debug for DemuxSocket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DemuxSocket")
            .field("ipv6", &self.ipv6)
            .finish_non_exhaustive()
    }
}

impl DemuxSocket {
    pub(crate) fn new(inner: Arc<dyn AsyncUdpSocket>) -> io::Result<(Arc<Self>, ProbeRx)> {
        let ipv6 = inner.local_addr()?.is_ipv6();
        let (tx, rx) = mpsc::unbounded_channel();
        Ok((
            Arc::new(Self {
                inner,
                probes: tx,
                ipv6,
            }),
            rx,
        ))
    }

    /// The destination as this socket's family needs it (IPv4 targets of
    /// an IPv6 socket become IPv4-mapped); `None` if unreachable from it.
    pub(crate) fn target(&self, addr: SocketAddr) -> Option<SocketAddr> {
        match (self.ipv6, addr) {
            (true, SocketAddr::V4(v4)) => {
                Some(SocketAddr::new(v4.ip().to_ipv6_mapped().into(), v4.port()))
            }
            (false, SocketAddr::V6(_)) => None,
            _ => Some(addr),
        }
    }

    /// Sends one probe datagram; a full socket buffer just loses it.
    pub(crate) fn send_probe(&self, to: SocketAddr, kind: ProbeKind) {
        let Some(to) = self.target(to) else { return };
        let d = kind.datagram();
        let _ = self.inner.try_send(&Transmit {
            destination: to,
            ecn: None,
            contents: &d,
            segment_size: None,
            src_ip: None,
        });
    }
}

impl AsyncUdpSocket for DemuxSocket {
    fn create_io_poller(self: Arc<Self>) -> Pin<Box<dyn UdpPoller>> {
        self.inner.clone().create_io_poller()
    }

    fn try_send(&self, transmit: &Transmit) -> io::Result<()> {
        self.inner.try_send(transmit)
    }

    fn poll_recv(
        &self,
        cx: &mut Context,
        bufs: &mut [IoSliceMut<'_>],
        meta: &mut [RecvMeta],
    ) -> Poll<io::Result<usize>> {
        if bufs.is_empty() || meta.is_empty() {
            return self.inner.poll_recv(cx, bufs, meta);
        }
        // One datagram (or GRO batch) at a time so probes can be removed
        // without shuffling buffers.
        loop {
            match self.inner.poll_recv(cx, &mut bufs[..1], &mut meta[..1]) {
                Poll::Ready(Ok(n)) if n >= 1 => {
                    let m = &meta[0];
                    let single = m.len <= m.stride || m.stride == 0;
                    if single && let Some(kind) = ProbeKind::parse(&bufs[0][..m.len]) {
                        let from = SocketAddr::new(m.addr.ip().to_canonical(), m.addr.port());
                        let _ = self.probes.send((from, kind));
                        continue;
                    }
                    return Poll::Ready(Ok(1));
                }
                other => return other,
            }
        }
    }

    fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner.local_addr()
    }

    fn max_transmit_segments(&self) -> usize {
        self.inner.max_transmit_segments()
    }

    fn max_receive_segments(&self) -> usize {
        self.inner.max_receive_segments()
    }

    fn may_fragment(&self) -> bool {
        self.inner.may_fragment()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_datagrams() {
        for k in [ProbeKind::Probe, ProbeKind::Ack] {
            assert_eq!(ProbeKind::parse(&k.datagram()), Some(k));
        }
        assert_eq!(ProbeKind::parse(b"\x00NWRP\x03"), None);
        assert_eq!(ProbeKind::parse(b"\x00NWRP\x01x"), None);
        // QUIC long and short headers always have 0x40 set.
        assert_eq!(ProbeKind::datagram(ProbeKind::Probe)[0] & 0x40, 0);
    }
}
