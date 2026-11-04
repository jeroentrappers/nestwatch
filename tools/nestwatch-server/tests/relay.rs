// SPDX-License-Identifier: AGPL-3.0-or-later
//! End-to-end rendezvous and relay tests (rendezvous.md §4): the server runs
//! in-process on loopback; stations register with `StationRegistration`,
//! listeners connect with `connect_via_relay`, and real NWP/1 sessions run
//! through the pipes.

mod common;

use std::time::Duration;

use common::*;
use nestwatch_net::LinkKind;
use nestwatch_relay::{
    Candidate, ErrorCode, MailboxId, MailboxIdentity, RegistrationStatus, RelayError,
    connect_via_relay,
};

/// Serves every accepted listener with a station session that sends
/// `frames` audio frames; reports come back on the channel.
fn serve_station(
    mut reg: nestwatch_relay::StationRegistration,
    policy: Trust,
    frames: u32,
) -> tokio::sync::mpsc::UnboundedReceiver<StationReport> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(incoming) = reg.next().await {
            let tx = tx.clone();
            let policy = policy.clone();
            tokio::spawn(async move {
                let r = run_station(incoming.link, policy, frames).await;
                let _ = tx.send(r);
            });
        }
    });
    rx
}

#[tokio::test]
async fn pairing_by_invite_then_trusted_reconnect_through_the_relay() {
    let srv = TestServer::start(|_| {}).await;
    let identity = MailboxIdentity::generate().unwrap();
    let mailbox = identity.id();
    let reg = srv.registration(identity);
    let candidates = vec![Candidate::parse("udp:192.0.2.10:40000").unwrap()];
    reg.set_candidates(candidates.clone());
    let observed = registered(&reg).await;
    assert!(observed.starts_with("127.0.0.1:"), "{observed}");

    let trust = Trust {
        invite: Some(token()),
        ..Trust::default()
    };
    let mut reports = serve_station(reg, trust.clone(), 25);

    // First pairing with the invitation (IKpsk2) over the relay pipe.
    let my_candidates = vec![Candidate::parse("udp:198.51.100.7:5000").unwrap()];
    let (link, peer) = connect_via_relay(&srv.relay(), mailbox, &my_candidates)
        .await
        .expect("relayed connect");
    assert_eq!(link.kind(), LinkKind::Messages);
    assert_eq!(
        peer.candidates, candidates,
        "station candidates reach the listener"
    );
    assert!(peer.observed.is_some());
    let l = tokio::time::timeout(WAIT, run_listener(link, invite_mode(), 20))
        .await
        .unwrap();
    assert!(l.paired && l.connected, "{l:?}");
    assert!(l.frames >= 20, "{l:?}");
    assert_eq!(l.first_seq, Some(0));
    let s = tokio::time::timeout(WAIT, reports.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(s.connected && s.paired, "{s:?}");
    assert_eq!(trust.trusted.lock().unwrap().len(), 1);

    // Reconnect as a trusted listener (IK), still relayed.
    let (link, _) = connect_via_relay(&srv.relay(), mailbox, &[]).await.unwrap();
    let l = tokio::time::timeout(WAIT, run_listener(link, trusted_mode(), 20))
        .await
        .unwrap();
    assert!(l.connected && !l.paired, "{l:?}");
    assert!(l.frames >= 20, "{l:?}");
    let s = tokio::time::timeout(WAIT, reports.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(s.connected && !s.paired, "{s:?}");

    let m = srv.server.metrics();
    assert_eq!(m.pipes_total, 2);
    assert!(m.bytes_relayed_total > 40 * 80, "{m:?}");
    assert_eq!(m.registrations_total, 1);
    assert_eq!(m.mailboxes_online, 1);
    srv.stop().await;
}

#[tokio::test]
async fn an_unknown_listener_is_rejected_end_to_end() {
    // The relay forwards; NWP/1 inside decides. A listener the station does
    // not trust fails the IK handshake even though the pipe works.
    let srv = TestServer::start(|_| {}).await;
    let identity = MailboxIdentity::generate().unwrap();
    let mailbox = identity.id();
    let reg = srv.registration(identity);
    registered(&reg).await;
    let mut reports = serve_station(reg, Trust::default(), 5);
    let (link, _) = connect_via_relay(&srv.relay(), mailbox, &[]).await.unwrap();
    let l = tokio::time::timeout(WAIT, run_listener(link, trusted_mode(), 5))
        .await
        .unwrap();
    assert!(!l.connected && l.failed.is_some(), "{l:?}");
    let s = tokio::time::timeout(WAIT, reports.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(!s.connected, "{s:?}");
    srv.stop().await;
}

#[tokio::test]
async fn takeover_without_the_secret_is_rejected() {
    let srv = TestServer::start(|_| {}).await;
    let owner = MailboxIdentity::generate().unwrap();
    let reg = srv.registration(owner.clone());
    registered(&reg).await;

    let thief = MailboxIdentity::from_parts(owner.id(), [0xEE; 32], 0);
    let bad = srv.registration(thief.clone());
    let s = wait_status(&bad, |s| matches!(s, RegistrationStatus::Rejected(_))).await;
    assert_eq!(s, RegistrationStatus::Rejected(ErrorCode::MailboxTaken));
    // The owner stays registered.
    assert!(matches!(
        *reg.status().borrow(),
        RegistrationStatus::Registered { .. }
    ));

    // Still refused while the owner is offline.
    reg.shutdown().await;
    let bad = srv.registration(thief);
    let s = wait_status(&bad, |s| matches!(s, RegistrationStatus::Rejected(_))).await;
    assert_eq!(s, RegistrationStatus::Rejected(ErrorCode::MailboxTaken));
    // The owner can come back.
    let reg = srv.registration(owner);
    registered(&reg).await;
    srv.stop().await;
}

#[tokio::test]
async fn a_new_registration_with_the_secret_replaces_the_old_one() {
    let srv = TestServer::start(|_| {}).await;
    let owner = MailboxIdentity::generate().unwrap();
    let first = srv.registration(owner.clone());
    registered(&first).await;
    let second = srv.registration(owner.clone());
    registered(&second).await;
    let s = wait_status(&first, |s| matches!(s, RegistrationStatus::Retrying { .. })).await;
    assert!(
        matches!(&s, RegistrationStatus::Retrying { error, .. } if error == "replaced"),
        "{s:?}"
    );
    // Connects reach the newest registration.
    let mut second = second;
    let accept = tokio::spawn(async move { second.next_request().await.is_some() });
    let relay = srv.relay();
    let connect = tokio::spawn(async move { connect_via_relay(&relay, owner.id(), &[]).await });
    assert!(tokio::time::timeout(WAIT, accept).await.unwrap().unwrap());
    connect.abort();
    drop(first);
    srv.stop().await;
}

#[tokio::test]
async fn offline_mailbox() {
    let srv = TestServer::start(|_| {}).await;
    let r = connect_via_relay(&srv.relay(), MailboxId([3; 16]), &[]).await;
    assert!(
        matches!(r, Err(RelayError::Server(ErrorCode::Offline))),
        "{r:?}"
    );
    assert_eq!(srv.server.metrics().connects_refused_total, 1);
    srv.stop().await;
}

#[tokio::test]
async fn a_station_that_never_accepts_times_out_as_offline() {
    let srv = TestServer::start(|c| c.limits.session_timeout = 0.5).await;
    let identity = MailboxIdentity::generate().unwrap();
    let mailbox = identity.id();
    let mut reg = srv.registration(identity);
    registered(&reg).await;
    let relay = srv.relay();
    let connect = tokio::spawn(async move { connect_via_relay(&relay, mailbox, &[]).await });
    let req = tokio::time::timeout(WAIT, reg.next_request())
        .await
        .unwrap()
        .unwrap();
    let r = connect.await.unwrap();
    assert!(
        matches!(r, Err(RelayError::Server(ErrorCode::Offline))),
        "{r:?}"
    );
    // A late accept is refused by the server (the pipe never forms).
    // A late accept is refused by the server: `error session`, then close.
    let mut link = req.accept(&[]).await.unwrap().link;
    let first = tokio::time::timeout(WAIT, link.recv())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(
        nestwatch_relay::Message::decode(&first).unwrap(),
        nestwatch_relay::Message::Error {
            code: ErrorCode::Session
        }
    );
    assert!(srv.server.metrics().sessions_expired_total >= 1);
    srv.stop().await;
}

#[tokio::test]
async fn connects_wait_for_a_station_that_reconnects_within_the_grace_period() {
    let srv = TestServer::start(|_| {}).await;
    let identity = MailboxIdentity::generate().unwrap();
    let mailbox = identity.id();
    let reg = srv.registration(identity.clone());
    registered(&reg).await;
    reg.shutdown().await;
    // Offline now, but within the 60 s grace: the connect is parked...
    let relay = srv.relay();
    let connect = tokio::spawn(async move { connect_via_relay(&relay, mailbox, &[]).await });
    tokio::time::sleep(Duration::from_millis(200)).await;
    // ...and delivered when the station registers again.
    let mut reg = srv.registration(identity);
    let req = tokio::time::timeout(WAIT, reg.next_request())
        .await
        .unwrap()
        .unwrap();
    let incoming = req.accept(&[]).await.unwrap();
    let (mut l, _) = connect.await.unwrap().unwrap();
    let mut s = incoming.link;
    l.send(b"NEST\x01\x01").await.unwrap();
    assert_eq!(s.recv().await.unwrap().unwrap(), b"NEST\x01\x01");
    srv.stop().await;
}

#[tokio::test]
async fn pipe_limit_per_mailbox_and_per_address() {
    for (per_mailbox, per_ip) in [(2, 16), (16, 2)] {
        let srv = TestServer::start(|c| {
            c.limits.pipes_per_mailbox = per_mailbox;
            c.limits.pipes_per_ip = per_ip;
        })
        .await;
        let identity = MailboxIdentity::generate().unwrap();
        let mailbox = identity.id();
        let mut reg = srv.registration(identity);
        registered(&reg).await;
        let held = tokio::spawn(async move {
            let mut links = Vec::new();
            while let Some(i) = reg.next().await {
                links.push(i.link);
            }
        });
        let a = connect_via_relay(&srv.relay(), mailbox, &[]).await.unwrap();
        let b = connect_via_relay(&srv.relay(), mailbox, &[]).await.unwrap();
        let c = connect_via_relay(&srv.relay(), mailbox, &[]).await;
        assert!(
            matches!(c, Err(RelayError::Server(ErrorCode::Limit))),
            "{per_mailbox}/{per_ip}: {c:?}"
        );
        // Closing a pipe frees its slot.
        drop(a);
        let mut ok = false;
        for _ in 0..50 {
            if connect_via_relay(&srv.relay(), mailbox, &[]).await.is_ok() {
                ok = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(ok, "slot freed");
        drop(b);
        held.abort();
        srv.stop().await;
    }
}

#[tokio::test]
async fn connect_rate_limit() {
    let srv = TestServer::start(|c| c.limits.connect_per_minute = 3).await;
    for _ in 0..3 {
        let r = connect_via_relay(&srv.relay(), MailboxId([9; 16]), &[]).await;
        assert!(
            matches!(r, Err(RelayError::Server(ErrorCode::Offline))),
            "{r:?}"
        );
    }
    let r = connect_via_relay(&srv.relay(), MailboxId([9; 16]), &[]).await;
    assert!(
        matches!(r, Err(RelayError::Server(ErrorCode::RateLimited))),
        "{r:?}"
    );
    assert_eq!(srv.server.metrics().rate_limited_total, 1);
    srv.stop().await;
}

#[tokio::test]
async fn register_rate_limit() {
    let srv = TestServer::start(|c| c.limits.register_per_minute = 2).await;
    for _ in 0..2 {
        let reg = srv.registration(MailboxIdentity::generate().unwrap());
        registered(&reg).await;
        reg.shutdown().await;
    }
    let reg = srv.registration(MailboxIdentity::generate().unwrap());
    let s = wait_status(&reg, |s| matches!(s, RegistrationStatus::Retrying { .. })).await;
    assert!(
        matches!(&s, RegistrationStatus::Retrying { error, .. } if error == "rate-limited"),
        "{s:?}"
    );
    srv.stop().await;
}

#[tokio::test]
async fn idle_pipes_are_closed() {
    let srv = TestServer::start(|c| c.limits.idle_timeout = 0.5).await;
    let identity = MailboxIdentity::generate().unwrap();
    let mailbox = identity.id();
    let mut reg = srv.registration(identity);
    registered(&reg).await;
    let station = tokio::spawn(async move { reg.next().await.unwrap().link });
    let (mut l, _) = connect_via_relay(&srv.relay(), mailbox, &[]).await.unwrap();
    let mut s = station.await.unwrap();
    // Traffic keeps it open...
    let mut t = tokio::time::Instant::now();
    for i in 0..4 {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        l.send(b"ping").await.unwrap();
        assert_eq!(s.recv().await.unwrap().unwrap(), b"ping");
        t = tokio::time::Instant::now();
    }
    // ...silence closes both ends.
    let end = tokio::time::timeout(Duration::from_secs(5), l.recv())
        .await
        .unwrap();
    assert!(matches!(end, Ok(None) | Err(_)), "{end:?}");
    assert!(
        t.elapsed() >= Duration::from_millis(400),
        "{:?}",
        t.elapsed()
    );
    let end = tokio::time::timeout(Duration::from_secs(5), s.recv())
        .await
        .unwrap();
    assert!(matches!(end, Ok(None) | Err(_)), "{end:?}");
    srv.stop().await;
}

#[tokio::test]
async fn pipe_lifetime_is_enforced() {
    let srv = TestServer::start(|c| c.limits.pipe_lifetime = 0.6).await;
    let identity = MailboxIdentity::generate().unwrap();
    let mailbox = identity.id();
    let mut reg = srv.registration(identity);
    registered(&reg).await;
    let station = tokio::spawn(async move { reg.next().await.unwrap().link });
    let (mut l, _) = connect_via_relay(&srv.relay(), mailbox, &[]).await.unwrap();
    let mut s = station.await.unwrap();
    let pinger = tokio::spawn(async move {
        loop {
            if s.send(b"x").await.is_err() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });
    let t = tokio::time::Instant::now();
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_secs(5), l.recv())
        .await
        .unwrap()
    {}
    assert!(
        t.elapsed() >= Duration::from_millis(400),
        "{:?}",
        t.elapsed()
    );
    pinger.abort();
    srv.stop().await;
}

#[tokio::test]
async fn relay_rate_is_limited() {
    // 400 kbit/s = 50 kB/s; 150 kB need about 2 s after the 50 kB burst.
    let srv = TestServer::start(|c| c.limits.pipe_rate_bps = 400_000).await;
    let identity = MailboxIdentity::generate().unwrap();
    let mailbox = identity.id();
    let mut reg = srv.registration(identity);
    registered(&reg).await;
    let station = tokio::spawn(async move { reg.next().await.unwrap().link });
    let (mut l, _) = connect_via_relay(&srv.relay(), mailbox, &[]).await.unwrap();
    let mut s = station.await.unwrap();
    let t = tokio::time::Instant::now();
    let sender = tokio::spawn(async move {
        for _ in 0..150 {
            l.send(&[0u8; 1000]).await.unwrap();
        }
        l
    });
    let mut got = 0;
    while got < 150_000 {
        got += s.recv().await.unwrap().unwrap().len();
    }
    let took = t.elapsed();
    assert!(took >= Duration::from_millis(1500), "{took:?}");
    assert!(took < Duration::from_secs(10), "{took:?}");
    drop(sender.await.unwrap());
    srv.stop().await;
}

#[tokio::test]
async fn monthly_budget_stops_relaying() {
    let srv = TestServer::start(|c| c.limits.monthly_bytes_per_mailbox = 10_000).await;
    let identity = MailboxIdentity::generate().unwrap();
    let mailbox = identity.id();
    let mut reg = srv.registration(identity);
    registered(&reg).await;
    let station = tokio::spawn(async move {
        let mut links = Vec::new();
        while let Some(i) = reg.next().await {
            links.push(i.link);
            if links.len() == 1 {
                break;
            }
        }
        (reg, links.pop().unwrap())
    });
    let (mut l, _) = connect_via_relay(&srv.relay(), mailbox, &[]).await.unwrap();
    let (_reg, mut s) = station.await.unwrap();
    let mut delivered = 0;
    for _ in 0..30 {
        if l.send(&[1u8; 1000]).await.is_err() {
            break;
        }
    }
    while let Ok(Ok(Some(b))) = tokio::time::timeout(Duration::from_secs(5), s.recv()).await {
        delivered += b.len();
    }
    assert!(delivered <= 10_000, "{delivered}");
    let r = connect_via_relay(&srv.relay(), mailbox, &[]).await;
    assert!(
        matches!(r, Err(RelayError::Server(ErrorCode::Quota))),
        "{r:?}"
    );
    srv.stop().await;
}

#[tokio::test]
async fn restart_keeps_the_secret_hash() {
    let srv = TestServer::start(|_| {}).await;
    let owner = MailboxIdentity::generate().unwrap();
    let reg = srv.registration(owner.clone());
    registered(&reg).await;
    reg.shutdown().await;
    let state = srv.stop().await;

    let srv = TestServer::start_in(state, |_| {}).await;
    let thief = srv.registration(MailboxIdentity::from_parts(owner.id(), [1; 32], 0));
    let s = wait_status(&thief, |s| matches!(s, RegistrationStatus::Rejected(_))).await;
    assert_eq!(s, RegistrationStatus::Rejected(ErrorCode::MailboxTaken));
    let reg = srv.registration(owner);
    registered(&reg).await;
    srv.stop().await;
}

#[tokio::test]
async fn version_and_garbage_are_refused() {
    use futures_util::{SinkExt, StreamExt};
    use minicbor::Encoder;
    use tokio_tungstenite::tungstenite::Message as Ws;

    let srv = TestServer::start(|_| {}).await;
    let url = srv.relay().server.ws_url().to_owned();
    let ask = |payload: Vec<u8>| {
        let url = url.clone();
        async move {
            let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
            ws.send(Ws::Binary(payload.into())).await.unwrap();
            loop {
                match ws.next().await {
                    Some(Ok(Ws::Binary(b))) => {
                        return Some(nestwatch_relay::Message::decode(&b).unwrap());
                    }
                    Some(Ok(_)) => continue,
                    _ => return None,
                }
            }
        }
    };
    let mut e = Encoder::new(Vec::new());
    e.map(4).unwrap();
    e.str("t").unwrap().str("register").unwrap();
    e.str("v").unwrap().u8(2).unwrap();
    e.str("mailbox").unwrap().bytes(&[0; 16]).unwrap();
    e.str("secret").unwrap().bytes(&[0; 32]).unwrap();
    assert_eq!(
        ask(e.into_writer()).await,
        Some(nestwatch_relay::Message::Error {
            code: ErrorCode::Version
        })
    );
    assert_eq!(
        ask(b"NEST\x01\x01".to_vec()).await,
        Some(nestwatch_relay::Message::Error {
            code: ErrorCode::Protocol
        })
    );
    // An accept for a session that does not exist.
    let accept = nestwatch_relay::Message::Accept {
        session: [7; 16],
        candidates: vec![],
    };
    assert_eq!(
        ask(accept.encode().unwrap()).await,
        Some(nestwatch_relay::Message::Error {
            code: ErrorCode::Session
        })
    );
    srv.stop().await;
}

#[tokio::test]
async fn oversized_messages_end_the_pipe() {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message as Ws;
    use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

    let srv = TestServer::start(|_| {}).await;
    let identity = MailboxIdentity::generate().unwrap();
    let mailbox = identity.id();
    let mut reg = srv.registration(identity);
    registered(&reg).await;
    let station = tokio::spawn(async move { reg.next().await.unwrap().link });
    // A raw listener that ignores the 64 KiB client-side limit.
    let config = WebSocketConfig::default()
        .max_message_size(None)
        .max_frame_size(None);
    let (mut ws, _) = tokio_tungstenite::connect_async_with_config(
        srv.relay().server.ws_url(),
        Some(config),
        false,
    )
    .await
    .unwrap();
    let connect = nestwatch_relay::Message::Connect {
        mailbox,
        nonce: [1; 16],
        candidates: vec![],
    };
    ws.send(Ws::Binary(connect.encode().unwrap().into()))
        .await
        .unwrap();
    let accepted = ws.next().await.unwrap().unwrap();
    assert!(matches!(
        nestwatch_relay::Message::decode(&accepted.into_data()).unwrap(),
        nestwatch_relay::Message::Accepted { .. }
    ));
    let mut s = station.await.unwrap();
    // Exactly 64 KiB passes...
    ws.send(Ws::Binary(vec![7u8; 64 * 1024].into()))
        .await
        .unwrap();
    let got = s.recv().await;
    // (the station's own Link limit is the Noise maximum of 65535 bytes,
    // so it may refuse it locally; the server forwarded it unchanged)
    assert!(matches!(got, Ok(Some(ref b)) if b.len() == 64 * 1024) || got.is_err());
    // ...one byte more ends the pipe.
    let _ = ws.send(Ws::Binary(vec![7u8; 64 * 1024 + 1].into())).await;
    let end = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                Some(Ok(Ws::Binary(_) | Ws::Ping(_) | Ws::Pong(_))) => continue,
                _ => return,
            }
        }
    })
    .await;
    assert!(end.is_ok(), "pipe closed");
    srv.stop().await;
}

#[tokio::test]
async fn stun_binding_answers_with_the_source_address() {
    let srv = TestServer::start(|_| {}).await;
    let stun = srv.server.stun_addr().unwrap();
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mapped = nestwatch_relay::stun::stun_binding(&socket, stun, Duration::from_secs(3))
        .await
        .unwrap();
    assert_eq!(mapped, socket.local_addr().unwrap());
    assert_eq!(srv.server.metrics().stun_requests_total, 1);
    srv.stop().await;
}

#[tokio::test]
async fn stun_is_rate_limited() {
    let srv = TestServer::start(|c| c.limits.stun_per_second = 3).await;
    let stun = srv.server.stun_addr().unwrap();
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    let mut answered = 0;
    for _ in 0..10 {
        let txid = nestwatch_relay::stun::new_transaction_id();
        socket
            .send_to(&nestwatch_relay::stun::binding_request(&txid), stun)
            .await
            .unwrap();
    }
    let mut buf = [0u8; 1500];
    while let Ok(Ok(_)) =
        tokio::time::timeout(Duration::from_millis(300), socket.recv_from(&mut buf)).await
    {
        answered += 1;
    }
    assert_eq!(answered, 3);
    srv.stop().await;
}

#[tokio::test]
async fn keep_alive_holds_the_control_connection() {
    let srv = TestServer::start(|c| c.limits.control_idle_timeout = 0.6).await;
    // Pinging every 200 ms keeps the registration.
    let reg = srv.registration_with(MailboxIdentity::generate().unwrap(), |c| {
        c.ping_interval = Duration::from_millis(200);
    });
    registered(&reg).await;
    let mut status = reg.status();
    status.borrow_and_update();
    let changed = tokio::time::timeout(Duration::from_millis(1500), status.changed()).await;
    assert!(changed.is_err(), "still registered: {:?}", *status.borrow());
    // Without pings the server drops it and the station reconnects.
    let quiet = srv.registration_with(MailboxIdentity::generate().unwrap(), |c| {
        c.ping_interval = Duration::from_secs(30);
    });
    registered(&quiet).await;
    let s = wait_status(&quiet, |s| matches!(s, RegistrationStatus::Retrying { .. })).await;
    assert!(matches!(s, RegistrationStatus::Retrying { .. }));
    registered(&quiet).await;
    srv.stop().await;
}

#[tokio::test]
async fn the_station_notices_a_dead_server() {
    let srv = TestServer::start(|_| {}).await;
    let reg = srv.registration_with(MailboxIdentity::generate().unwrap(), |c| {
        c.ping_interval = Duration::from_millis(100);
        c.dead_after = Duration::from_millis(500);
    });
    registered(&reg).await;
    srv.stop().await;
    let s = wait_status(&reg, |s| matches!(s, RegistrationStatus::Retrying { .. })).await;
    assert!(matches!(s, RegistrationStatus::Retrying { .. }));
}

/// OPEN-QUESTIONS WEBST-3: a client without WebSocket pings (a browser
/// station) keeps its control connection with `ping` messages; each is
/// answered with `pong`.
#[tokio::test]
async fn ping_messages_hold_the_control_connection() {
    use futures_util::{SinkExt, StreamExt};
    use nestwatch_relay::Message;
    use tokio_tungstenite::tungstenite::Message as Ws;

    let srv = TestServer::start(|c| c.limits.control_idle_timeout = 0.6).await;
    let url = srv.relay().server.ws_url().to_owned();
    let (mut ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
    let identity = MailboxIdentity::generate().unwrap();
    let register = Message::Register {
        mailbox: identity.id(),
        secret: identity.secret(),
    };
    ws.send(Ws::Binary(register.encode().unwrap().into()))
        .await
        .unwrap();
    async fn next<S>(ws: &mut S) -> Option<Message>
    where
        S: futures_util::Stream<Item = Result<Ws, tokio_tungstenite::tungstenite::Error>> + Unpin,
    {
        loop {
            match tokio::time::timeout(Duration::from_secs(2), ws.next()).await {
                Ok(Some(Ok(Ws::Binary(b)))) => return Some(Message::decode(&b).unwrap()),
                Ok(Some(Ok(Ws::Ping(_) | Ws::Pong(_)))) => continue,
                _ => return None,
            }
        }
    }
    assert!(matches!(
        next(&mut ws).await,
        Some(Message::Registered { .. })
    ));
    // Five pings 300 ms apart outlive the 0.6 s idle timeout twice over.
    for _ in 0..5 {
        tokio::time::sleep(Duration::from_millis(300)).await;
        ws.send(Ws::Binary(Message::Ping.encode().unwrap().into()))
            .await
            .unwrap();
        assert_eq!(next(&mut ws).await, Some(Message::Pong));
    }
    assert_eq!(srv.server.metrics().mailboxes_online, 1);
    srv.stop().await;
}
