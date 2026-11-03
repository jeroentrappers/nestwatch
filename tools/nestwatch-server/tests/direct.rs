// SPDX-License-Identifier: AGPL-3.0-or-later
//! The full direct-path flow on loopback (rendezvous.md §5): STUN against
//! the server, candidate exchange through the rendezvous, hole punching,
//! QUIC, and a real NWP/1 session over the QUIC stream, while the relayed
//! session it replaces is still up.

mod common;

use common::*;
use nestwatch_net::LinkKind;
use nestwatch_relay::direct::{PunchRole, bind_punch_socket, punch_direct};
use nestwatch_relay::stun::stun_binding;
use nestwatch_relay::{Candidate, MailboxIdentity, connect_via_relay};

#[tokio::test]
async fn rendezvous_then_direct_quic_carries_an_nwp_session() {
    let srv = TestServer::start(|_| {}).await;
    let stun = srv.server.stun_addr().unwrap();
    let identity = MailboxIdentity::generate().unwrap();
    let mailbox = identity.id();
    let mut reg = srv.registration(identity);
    registered(&reg).await;
    let trust = Trust::default();
    trust
        .trusted
        .lock()
        .unwrap()
        .push(*listener_key().public_key());

    // Station: on each request, STUN from a fresh punch socket, accept with
    // that candidate, run NWP/1 on the relay pipe and punch in parallel.
    let station_trust = trust.clone();
    let station = tokio::spawn(async move {
        let req = reg.next_request().await.unwrap();
        let sock = bind_punch_socket("127.0.0.1:0".parse().unwrap())
            .await
            .unwrap();
        let srflx = stun_binding(&sock, stun, WAIT).await.unwrap();
        let mine = vec![
            Candidate::Udp(srflx),
            Candidate::parse("udp:192.0.2.44:9").unwrap(),
        ];
        let incoming = req.accept(&mine).await.unwrap();
        let relayed = tokio::spawn(run_station(incoming.link, station_trust.clone(), 1000));
        let direct = punch_direct(sock, &incoming.peer.candidates, PunchRole::Station)
            .await
            .expect("station direct link");
        assert_eq!(direct.kind(), LinkKind::Stream);
        let report = run_station(direct, station_trust, 30).await;
        (report, relayed.await.unwrap())
    });

    // Listener: STUN, connect through the relay with its candidate, start a
    // relayed session, then punch and run a fresh trusted session directly.
    let sock = bind_punch_socket("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let srflx = stun_binding(&sock, stun, WAIT).await.unwrap();
    assert_eq!(srflx, sock.local_addr().unwrap());
    let (relayed, peer) = connect_via_relay(&srv.relay(), mailbox, &[Candidate::Udp(srflx)])
        .await
        .unwrap();
    assert!(
        peer.candidates.iter().any(|c| c.udp().is_some()),
        "{peer:?}"
    );
    let relayed = tokio::spawn(run_listener(relayed, trusted_mode(), 10));
    let direct = punch_direct(sock, &peer.candidates, PunchRole::Listener)
        .await
        .expect("listener direct link");
    assert_eq!(direct.kind(), LinkKind::Stream);
    assert!(direct.info().peer.is_some());
    let l = tokio::time::timeout(WAIT, run_listener(direct, trusted_mode(), 25))
        .await
        .unwrap();
    assert!(l.connected, "{l:?}");
    assert!(l.frames >= 25, "{l:?}");
    let relayed = tokio::time::timeout(WAIT, relayed).await.unwrap().unwrap();
    assert!(relayed.connected && relayed.frames >= 10, "{relayed:?}");

    let (direct_station, relayed_station) =
        tokio::time::timeout(WAIT, station).await.unwrap().unwrap();
    assert!(direct_station.connected, "{direct_station:?}");
    assert!(direct_station.frames_sent >= 25, "{direct_station:?}");
    assert!(relayed_station.connected, "{relayed_station:?}");
    srv.stop().await;
}
