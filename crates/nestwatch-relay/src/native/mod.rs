// SPDX-License-Identifier: AGPL-3.0-or-later
//! Native (tokio) rendezvous client: station registration (§4.1) and
//! listener connect (§4.2).

mod station;
mod ws;

use std::net::SocketAddr;

use nestwatch_net::Link;
use nestwatch_proto::Host;

pub use station::{
    Incoming, IncomingRequest, RegistrationConfig, RegistrationStatus, StationRegistration,
};

use crate::candidate::Candidate;
use crate::exchange::{self, PeerInfo};
use crate::{MailboxId, RelayConfig, RelayError};

/// Listener: reaches the station behind `mailbox` through the relay server
/// (rendezvous.md §4.2). Returns the relay pipe as a
/// [`Messages`](nestwatch_net::LinkKind::Messages) link, ready for an NWP/1
/// listener session (the listener sends its preface first), and the
/// station's candidates for hole punching.
///
/// A server answer `offline` comes back as
/// [`RelayError::Server`]`(`[`ErrorCode::Offline`](crate::ErrorCode::Offline)`)`;
/// the caller retries with backoff.
pub async fn connect_via_relay(
    config: &RelayConfig,
    mailbox: MailboxId,
    candidates: &[Candidate],
) -> Result<(Link, PeerInfo), RelayError> {
    let (ws, info) = ws::open(config).await?;
    let mut link = Link::from_websocket(ws, info);
    let peer = exchange::listener_connect(&mut link, mailbox, candidates, config.timeout).await?;
    Ok((link, peer))
}

/// `udp:` candidates for a socket bound to `port` on all interfaces: every
/// listable interface address (protocol.md §3.5 filtering and order: VPN,
/// then LAN; loopback and link-local IPv6 dropped). Empty where interfaces
/// cannot be listed (Android: the app adds its own).
pub fn local_udp_candidates(port: u16) -> Vec<Candidate> {
    let addrs = nestwatch_net::local::interface_addrs().unwrap_or_default();
    nestwatch_proto::endpoint::order_endpoints(&[], &addrs, port)
        .into_iter()
        .filter_map(|e| match e.host {
            Host::Ipv4(ip) => Some(SocketAddr::from((ip, e.port))),
            Host::Ipv6(ip) => Some(SocketAddr::from((ip, e.port))),
            Host::Dns(_) => None,
        })
        .map(Candidate::Udp)
        .collect()
}
