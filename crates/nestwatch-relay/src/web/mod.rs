// SPDX-License-Identifier: AGPL-3.0-or-later
//! Browser client (wasm32) over the browser WebSocket: the listener's
//! relayed connect (rendezvous.md FR-RV-5) and the relay-only station's
//! registration (FR-RV-7, [`StationRegistration`]). When the page is served
//! by the relay server itself, `connect-src 'self'` covers the rendezvous
//! WebSocket.

mod station;

pub use station::{IncomingRequest, StationRegistration};

use nestwatch_net::Link;

use crate::candidate::Candidate;
use crate::exchange::{self, PeerInfo, with_timeout};
use crate::{MailboxId, RelayConfig, RelayError};

/// Listener: reaches the station behind `mailbox` through the relay server
/// (rendezvous.md §4.2) and returns the relay pipe, ready for an NWP/1
/// listener session. Browsers cannot punch holes, so `candidates` is
/// normally empty.
pub async fn connect_via_relay(
    config: &RelayConfig,
    mailbox: MailboxId,
    candidates: &[Candidate],
) -> Result<(Link, PeerInfo), RelayError> {
    let url = config.server.ws_url().to_owned();
    let mut link = with_timeout(config.timeout, async move {
        Ok(nestwatch_net::ws_client::connect(&url).await?)
    })
    .await?;
    let peer = exchange::listener_connect(&mut link, mailbox, candidates, config.timeout).await?;
    Ok((link, peer))
}
