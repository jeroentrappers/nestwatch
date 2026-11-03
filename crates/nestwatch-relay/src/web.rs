// SPDX-License-Identifier: AGPL-3.0-or-later
//! Browser listener (wasm32): relayed connect over the browser WebSocket
//! (rendezvous.md FR-RV-5). The page is served by the relay server itself,
//! so `connect-src 'self'` covers the rendezvous WebSocket.

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
