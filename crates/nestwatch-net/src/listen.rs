// SPDX-License-Identifier: AGPL-3.0-or-later
//! Station TCP listener with port fallback (protocol.md §3.2).

use std::future::poll_fn;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::task::Poll;

use nestwatch_proto::endpoint::{DEFAULT_TCP_PORT, PORT_FALLBACK_COUNT};
use socket2::{Domain, Protocol, Socket, Type};
use tokio::net::{TcpListener, TcpStream};

use crate::{Link, NetError, StreamLink};

/// Which local addresses to listen on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BindScope {
    /// All interfaces, IPv4 and IPv6 (protocol.md §3.2).
    #[default]
    All,
    /// `127.0.0.1` and `::1` only (tests, local tools).
    Loopback,
}

/// Port selection for one listening service.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListenConfig {
    /// First port tried. `0` lets the OS choose (no fallback).
    pub port: u16,
    /// Number of ports tried, counting `port` itself (protocol.md §3.2: 10).
    pub attempts: u16,
    /// Addresses to bind.
    pub scope: BindScope,
}

impl ListenConfig {
    /// Starts at `port` with the standard fallback count on all interfaces.
    pub fn new(port: u16) -> Self {
        Self {
            port,
            attempts: PORT_FALLBACK_COUNT,
            scope: BindScope::All,
        }
    }

    /// The same configuration restricted to loopback.
    pub fn loopback(mut self) -> Self {
        self.scope = BindScope::Loopback;
        self
    }
}

impl Default for ListenConfig {
    /// Port 47100, falling back to 47101..47109 (protocol.md §3.2).
    fn default() -> Self {
        Self::new(DEFAULT_TCP_PORT)
    }
}

fn listener(addr: SocketAddr, v6_only: Option<bool>) -> io::Result<TcpListener> {
    let domain = if addr.is_ipv4() {
        Domain::IPV4
    } else {
        Domain::IPV6
    };
    let socket = Socket::new(domain, Type::STREAM, Some(Protocol::TCP))?;
    if let Some(only) = v6_only {
        socket.set_only_v6(only)?;
    }
    // Allows rebinding right after a restart (TIME_WAIT); on Windows the
    // option would allow port stealing instead, so it stays off there.
    #[cfg(not(windows))]
    socket.set_reuse_address(true)?;
    socket.set_nonblocking(true)?;
    socket.bind(&addr.into())?;
    socket.listen(128)?;
    TcpListener::from_std(socket.into())
}

/// Binds all sockets for one port; any failure means the port is unusable.
fn bind_port(scope: BindScope, port: u16) -> io::Result<Vec<TcpListener>> {
    let (v4, v6) = match scope {
        BindScope::All => (Ipv4Addr::UNSPECIFIED, Ipv6Addr::UNSPECIFIED),
        BindScope::Loopback => (Ipv4Addr::LOCALHOST, Ipv6Addr::LOCALHOST),
    };
    if scope == BindScope::All {
        // Preferred: one dual-stack socket.
        match listener(SocketAddr::new(v6.into(), port), Some(false)) {
            Ok(l) => return Ok(vec![l]),
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => return Err(e),
            Err(_) => {}
        }
    }
    // Separate sockets: IPv4 is required, IPv6 joins on the same port if
    // the host has it.
    let first = listener(SocketAddr::new(v4.into(), port), None)?;
    let port = first.local_addr()?.port();
    let mut out = vec![first];
    match listener(SocketAddr::new(v6.into(), port), Some(true)) {
        Ok(l) => out.push(l),
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => return Err(e),
        Err(_) => {}
    }
    Ok(out)
}

/// Binds the first free port of `config`; returns the sockets and the port.
pub(crate) fn bind_with_fallback(
    config: ListenConfig,
) -> Result<(Vec<TcpListener>, u16), NetError> {
    let attempts = if config.port == 0 {
        1
    } else {
        config.attempts.max(1)
    };
    let last = config.port.saturating_add(attempts - 1);
    let mut last_err = None;
    for port in config.port..=last {
        match bind_port(config.scope, port) {
            Ok(ls) => {
                let bound = ls[0].local_addr()?.port();
                return Ok((ls, bound));
            }
            Err(e) => last_err = Some(e),
        }
    }
    Err(NetError::NoPort {
        first: config.port,
        last,
        source: last_err.unwrap_or_else(|| io::Error::other("no port tried")),
    })
}

/// Accepts from whichever socket is ready, rotating for fairness.
pub(crate) async fn accept_any(
    listeners: &[TcpListener],
    next: &mut usize,
) -> io::Result<(TcpStream, SocketAddr)> {
    poll_fn(|cx| {
        let n = listeners.len();
        for i in 0..n {
            let idx = (*next + i) % n;
            if let Poll::Ready(r) = listeners[idx].poll_accept(cx) {
                *next = (idx + 1) % n;
                return Poll::Ready(r);
            }
        }
        Poll::Pending
    })
    .await
}

/// The station's TCP listener (protocol.md §3.2).
#[derive(Debug)]
pub struct StationListener {
    listeners: Vec<TcpListener>,
    port: u16,
    next: usize,
}

impl StationListener {
    /// Binds the first free port of `config` on IPv4 and IPv6.
    pub fn bind(config: ListenConfig) -> Result<Self, NetError> {
        let (listeners, port) = bind_with_fallback(config)?;
        Ok(Self {
            listeners,
            port,
            next: 0,
        })
    }

    /// The bound port, to advertise in mDNS and endpoints.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Local socket addresses (one dual-stack socket, or one per family).
    pub fn local_addrs(&self) -> Vec<SocketAddr> {
        self.listeners
            .iter()
            .filter_map(|l| l.local_addr().ok())
            .collect()
    }

    /// Waits for the next connection and returns it as a
    /// [`Stream`](crate::LinkKind::Stream) link with `TCP_NODELAY` set.
    /// Cancel-safe.
    pub async fn accept(&mut self) -> Result<Link, NetError> {
        let (stream, _) = accept_any(&self.listeners, &mut self.next).await?;
        Ok(Link::Stream(StreamLink::new(stream)))
    }
}
