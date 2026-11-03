// SPDX-License-Identifier: AGPL-3.0-or-later
//! The listener side of the relay (rendezvous.md §4.2, §5): connecting
//! through a station's mailbox and, while relayed, moving to a direct path
//! (FR-RV-3).

use nestwatch_proto::RelayRef;
use nestwatch_relay::{MailboxId, RelayConfig, RelayServer};

use super::follow::Dialed;
use super::{NetPath, Shared};

fn relay_config(shared: &Shared, origin: &str) -> Result<RelayConfig, String> {
    let server = RelayServer::parse(origin).map_err(|e| e.to_string())?;
    #[cfg(feature = "native")]
    let server = match shared.cfg.relay.stun_port {
        Some(p) => server.with_stun_port(p),
        None => server,
    };
    let mut c = RelayConfig::new(server);
    c.timeout = shared.cfg.relay.timeout;
    Ok(c)
}

/// Connects through the relay (§4.2). Native listeners first bind a punch
/// socket and send its candidates, so the session can later move to a
/// direct path.
#[cfg(feature = "native")]
pub(super) async fn dial(shared: &Shared, r: &RelayRef) -> Result<Dialed, String> {
    let cfg = relay_config(shared, &r.server)?;
    let opts = &shared.cfg.relay;
    let (socket, candidates) = if opts.direct {
        crate::punch::punch_socket(opts.punch_bind, &cfg.server, opts.stun_timeout)
            .await
            .map_or((None, Vec::new()), |(s, c)| (Some(s), c))
    } else {
        (None, Vec::new())
    };
    let (link, peer) = nestwatch_relay::connect_via_relay(&cfg, MailboxId(r.mailbox), &candidates)
        .await
        .map_err(|e| e.to_string())?;
    let punch = socket.map(|socket| native::Punch {
        socket,
        peer: peer.candidates,
        relay: r.clone(),
    });
    Ok(Dialed {
        link,
        path: NetPath::Relayed,
        punch,
    })
}

/// Connects through the relay from the browser (FR-RV-5): only reachable
/// when the page is served by that server (`connect-src 'self'`).
#[cfg(not(feature = "native"))]
pub(super) async fn dial(shared: &Shared, r: &RelayRef) -> Result<Dialed, String> {
    let cfg = relay_config(shared, &r.server)?;
    #[cfg(target_arch = "wasm32")]
    {
        let (link, _) = nestwatch_relay::connect_via_relay(&cfg, MailboxId(r.mailbox), &[])
            .await
            .map_err(|e| e.to_string())?;
        Ok(Dialed {
            link,
            path: NetPath::Relayed,
            punch: None,
        })
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = cfg;
        Err("the relay needs the browser WebSocket".into())
    }
}

#[cfg(feature = "native")]
pub(super) use native::{Punch, upgrade};

#[cfg(feature = "native")]
mod native {
    use std::sync::Arc;
    use std::time::Duration;

    use nestwatch_crypto::PublicKey;
    use nestwatch_proto::RelayRef;
    use nestwatch_relay::Candidate;
    use nestwatch_relay::direct::{PunchRole, punch_direct};
    use nestwatch_session::ListenerMode;
    use tokio::net::UdpSocket;
    use tokio::time::Instant;

    use super::super::follow::{Dialed, run_session};
    use super::super::{NetPath, Route, Shared};
    use super::relay_config;
    use crate::rt;

    /// What hole punching needs after a relayed connect (§5.2): the socket
    /// the candidates were learned on and the station's candidates.
    pub(crate) struct Punch {
        pub socket: UdpSocket,
        pub peer: Vec<Candidate>,
        pub relay: RelayRef,
    }

    /// Whether the relayed session `serial` still serves station `id`.
    fn current(shared: &Shared, id: &str, serial: u64) -> bool {
        let g = shared.lock();
        g.running
            && g.stations
                .get(id)
                .is_some_and(|rt| rt.cmd.as_ref().is_some_and(|(s, _)| *s == serial))
    }

    /// While the relayed session `relayed` lives: punch a direct path and,
    /// once it forms, run a fresh trusted session over it that replaces the
    /// relayed one (§5.2 items 3, 4). Without a path, retry after the retry
    /// interval or a network change, with a fresh candidate exchange
    /// (§5.2 item 5).
    pub(crate) async fn upgrade(
        shared: Arc<Shared>,
        id: String,
        key: PublicKey,
        relayed: u64,
        punch: Punch,
    ) {
        if !shared.cfg.relay.direct {
            return;
        }
        let relay = punch.relay.clone();
        let mut punch = Some(punch);
        let delay = shared.cfg.relay.first_punch_after;
        if !delay.is_zero() {
            rt::sleep(delay).await;
        }
        loop {
            if !current(&shared, &id, relayed) {
                return;
            }
            if let Some(p) = punch.take()
                && let Some((dialed, route, label)) = try_direct(&shared, p).await
            {
                if !current(&shared, &id, relayed) {
                    return;
                }
                let mode = ListenerMode::Trusted { station: key };
                run_session(
                    &shared,
                    dialed,
                    route,
                    label,
                    mode,
                    Some(id),
                    None,
                    Vec::new(),
                    Some(relayed),
                )
                .await;
                // The reconnect loop takes over when the direct session ends.
                return;
            }
            let deadline = Instant::now() + shared.cfg.relay.retry_interval;
            loop {
                let notified = shared.network.notified();
                tokio::select! {
                    () = rt::sleep(Duration::from_secs(1)) => {
                        if !current(&shared, &id, relayed) {
                            return;
                        }
                        if Instant::now() >= deadline {
                            break;
                        }
                    }
                    () = notified => break,
                }
            }
            punch = exchange(&shared, &relay).await;
        }
    }

    /// A fresh candidate exchange through the rendezvous for a retry: a
    /// new punch socket, `connect` with its candidates, and the pipe closed
    /// again at once (the station punches from its side on `incoming`).
    async fn exchange(shared: &Shared, r: &RelayRef) -> Option<Punch> {
        let cfg = relay_config(shared, &r.server).ok()?;
        let opts = &shared.cfg.relay;
        let (socket, candidates) =
            crate::punch::punch_socket(opts.punch_bind, &cfg.server, opts.stun_timeout).await?;
        let (mut link, peer) = nestwatch_relay::connect_via_relay(
            &cfg,
            nestwatch_relay::MailboxId(r.mailbox),
            &candidates,
        )
        .await
        .ok()?;
        let _ = rt::timeout(Duration::from_secs(2), link.close()).await;
        Some(Punch {
            socket,
            peer: peer.candidates,
            relay: r.clone(),
        })
    }

    /// Punches to the station's `udp:` candidates (QUIC, path "direct"),
    /// else tries its `tcp:` candidates as ordinary endpoints (§5.2, last
    /// paragraph).
    async fn try_direct(shared: &Shared, p: Punch) -> Option<(Dialed, Option<Route>, String)> {
        let udp: Vec<Candidate> = p
            .peer
            .iter()
            .filter(|c| c.udp().is_some())
            .cloned()
            .collect();
        if !udp.is_empty()
            && let Some(link) = punch_direct(p.socket, &udp, PunchRole::Listener).await
        {
            let label = match link.info().peer {
                Some(a) => format!("direct {a}"),
                None => "direct".into(),
            };
            let dialed = Dialed {
                link,
                path: NetPath::Direct,
                punch: None,
            };
            return Some((dialed, None, label));
        }
        for c in p.peer {
            let Candidate::Tcp(e) = c else { continue };
            if let Ok(link) = nestwatch_net::dial::dial(&e, shared.cfg.dial_timeout).await {
                let path = match link.info().peer {
                    Some(a) if nestwatch_proto::endpoint::is_vpn_address(a.ip()) => NetPath::Vpn,
                    Some(a) if is_public(a.ip()) => NetPath::Direct,
                    _ => NetPath::HomeNetwork,
                };
                let route = Route::Tcp(e);
                let label = route.to_string();
                let dialed = Dialed {
                    link,
                    path,
                    punch: None,
                };
                return Some((dialed, Some(route), label));
            }
        }
        None
    }

    /// A globally routed address (not private, loopback or link-local).
    fn is_public(ip: std::net::IpAddr) -> bool {
        match ip.to_canonical() {
            std::net::IpAddr::V4(v4) => {
                !(v4.is_private()
                    || v4.is_loopback()
                    || v4.is_link_local()
                    || v4.is_unspecified()
                    || nestwatch_proto::endpoint::is_vpn_address(v4.into()))
            }
            std::net::IpAddr::V6(v6) => {
                let s = v6.segments()[0];
                !(v6.is_loopback()
                    || v6.is_unspecified()
                    || (s & 0xfe00) == 0xfc00
                    || (s & 0xffc0) == 0xfe80)
            }
        }
    }
}
