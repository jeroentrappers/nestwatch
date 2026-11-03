// SPDX-License-Identifier: AGPL-3.0-or-later
//! The punch socket and its candidates, shared by both engines
//! (rendezvous.md §5.1): a UDP socket, its server-reflexive address from
//! the relay server's STUN service and its local interface addresses.

use std::net::SocketAddr;
use std::time::Duration;

use nestwatch_relay::direct::bind_punch_socket;
use nestwatch_relay::{Candidate, RelayServer};
use tokio::net::UdpSocket;

/// Candidates sent per side at most (OPEN-QUESTIONS RELAY-4).
pub(crate) const MAX_CANDIDATES: usize = 16;

/// Binds a punch socket on `bind` and lists its `udp:` candidates:
/// server-reflexive first (if STUN answers within `stun_timeout`), then the
/// local addresses of the socket's family. `None` if no socket could be
/// bound.
pub(crate) async fn punch_socket(
    bind: SocketAddr,
    server: &RelayServer,
    stun_timeout: Duration,
) -> Option<(UdpSocket, Vec<Candidate>)> {
    let sock = bind_punch_socket(bind).await.ok()?;
    let v4 = bind.is_ipv4();
    let mut out = Vec::new();
    if let Some(stun) = resolve(&server.stun_endpoint().to_string(), v4).await
        && let Ok(a) = nestwatch_relay::stun::stun_binding(&sock, stun, stun_timeout).await
    {
        out.push(Candidate::Udp(a));
    }
    let local = sock.local_addr().ok()?;
    let locals = if local.ip().is_unspecified() {
        nestwatch_relay::local_udp_candidates(local.port())
    } else {
        vec![Candidate::Udp(local)]
    };
    for c in locals {
        if c.udp().is_some_and(|a| a.is_ipv4() == v4) && !out.contains(&c) {
            out.push(c);
        }
    }
    out.truncate(MAX_CANDIDATES);
    Some((sock, out))
}

/// Resolves `host:port` to an address of the wanted family.
async fn resolve(endpoint: &str, v4: bool) -> Option<SocketAddr> {
    tokio::net::lookup_host(endpoint)
        .await
        .ok()?
        .find(|a| a.is_ipv4() == v4)
}
