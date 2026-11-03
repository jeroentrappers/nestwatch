// SPDX-License-Identifier: AGPL-3.0-or-later
//! The STUN service (rendezvous.md §5.1): Binding requests on UDP, answered
//! with XOR-MAPPED-ADDRESS, rate-limited per address.

use std::sync::Arc;

use nestwatch_relay::stun;
use tokio::net::UdpSocket;
use tokio::sync::watch;

use crate::hub::Hub;
use crate::metrics::inc;

/// Answers Binding requests on `socket` until `shutdown`.
pub(crate) async fn serve(socket: UdpSocket, hub: Arc<Hub>, mut shutdown: watch::Receiver<bool>) {
    let mut buf = [0u8; 1500];
    loop {
        let r = tokio::select! {
            r = socket.recv_from(&mut buf) => r,
            () = crate::tasks::stopped(&mut shutdown) => return,
        };
        // ICMP errors from earlier sends surface here on some systems.
        let Ok((n, from)) = r else { continue };
        if !stun::is_stun(&buf[..n]) || !hub.stun_allowed(from.ip()) {
            continue;
        }
        if let Some(resp) = stun::answer(&buf[..n], from) {
            inc(&hub.metrics.stun_requests);
            let _ = socket.send_to(&resp, from).await;
        }
    }
}
