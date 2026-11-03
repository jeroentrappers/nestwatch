// SPDX-License-Identifier: AGPL-3.0-or-later
//! Pairing, trust and session limits over real loopback sockets
//! (product.md §3.3, protocol.md §4, §5, §7.10, §7.11, §9).

mod common;

use std::time::Duration;

use common::*;
use lull_core::Transport;
use lull_core::listener::{ConnStatus, ListenerError, ListenerEvent, Route};
use lull_core::station::{PairingEnd, StationEvent};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_pair_1_pair_by_invitation_over_tcp() {
    let (station, _) = station("Nursery").await;
    let (listener, _) = listener("Parents");
    let mut sev = station.events();
    let info = station.open_pairing().await.unwrap();
    assert!(info.invite_uri.starts_with("lull://invite?v=1&s="));
    assert_eq!(info.code.len(), 11);

    let r = listener.pair_invitation(&info.invite_uri).await.unwrap();
    assert_eq!(r.station_id, station.device_id());
    assert_eq!(r.name, "Nursery");
    // FR-PAIR-5: both sides show the same verification number.
    let paired = wait_event(&mut sev, "station Paired", |e| {
        matches!(e, StationEvent::Paired { .. })
    })
    .await;
    let StationEvent::Paired {
        verification,
        name,
        listener_id,
    } = paired
    else {
        unreachable!()
    };
    assert_eq!(verification, r.verification);
    assert_eq!(name, "Parents");
    assert_eq!(listener_id, listener.device_id());
    assert_eq!(r.verification.len(), 7);

    let mut ls = listener.state();
    wait_state(&mut ls, "live", |s| is_live(s, &r.station_id)).await;
    let mut ss = station.state();
    let st = wait_state(&mut ss, "one listener", |s| s.listeners.len() == 1).await;
    assert_eq!(st.listeners[0].transport, Transport::Tcp);
    let trusted = station.trusted_listeners();
    assert_eq!(trusted.len(), 1);
    assert_eq!(trusted[0].name, "Parents");
    assert_eq!(listener.known_stations()[0].name, "Nursery");
    station.shutdown().await;
    listener.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_pair_1_pair_by_web_address_over_websocket() {
    let storage = lull_core::MemoryStorage::new();
    let station = station_with(with_web(station_config("Nursery")), &storage).await;
    let (listener, _) = listener("Browser");
    let info = station.open_pairing().await.unwrap();
    // protocol.md §3.5: the browser receives the invitation in the fragment.
    let web = info.web_url.clone().expect("web url");
    assert!(web.starts_with("http://127.0.0.1:") && web.contains("/#lull%3A%2F%2Finvite"));
    let r = listener.pair_invitation(&web).await.unwrap();
    assert_eq!(r.station_id, station.device_id());
    let mut ss = station.state();
    let st = wait_state(&mut ss, "websocket listener", |s| s.listeners.len() == 1).await;
    assert_eq!(st.listeners[0].transport, Transport::WebSocket);
    let ls = listener.snapshot();
    assert!(
        ls.stations[0]
            .route
            .as_deref()
            .unwrap()
            .starts_with("ws://127.0.0.1:")
    );
    station.shutdown().await;
    listener.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_pair_2_pair_by_code_with_injected_station_id() {
    let (station, _) = station("Nursery").await;
    let (listener, _) = listener("Tablet");
    let info = station.open_pairing().await.unwrap();
    let port = station.snapshot().tcp_port;
    // No mDNS: the station ID that discovery would provide is passed in.
    let route = Route::parse(&format!("127.0.0.1:{port}")).unwrap();
    let r = listener
        .pair_code(&info.code.to_lowercase(), station.device_id(), vec![route])
        .await
        .unwrap();
    assert_eq!(r.station_id, station.device_id());
    let mut ls = listener.state();
    wait_state(&mut ls, "live", |s| is_live(s, &r.station_id)).await;
    assert_eq!(station.trusted_listeners().len(), 1);
    station.shutdown().await;
    listener.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn protocol_5_3_wrong_codes_close_the_window() {
    let storage = lull_core::MemoryStorage::new();
    let mut cfg = station_config("Nursery");
    cfg.max_code_failures = 2;
    let station = station_with(cfg, &storage).await;
    let (listener, _) = listener("Guesser");
    let mut sev = station.events();
    let info = station.open_pairing().await.unwrap();
    let wrong = if info.code.starts_with('0') {
        "11111-11111"
    } else {
        "00000-00000"
    };
    let route = Route::parse(&format!("127.0.0.1:{}", station.snapshot().tcp_port)).unwrap();
    for _ in 0..2 {
        let e = listener
            .pair_code(wrong, station.device_id(), vec![route.clone()])
            .await
            .unwrap_err();
        assert!(matches!(e, ListenerError::Rejected(_)), "{e:?}");
    }
    let e = wait_event(&mut sev, "window closed", |e| {
        matches!(e, StationEvent::PairingClosed { .. })
    })
    .await;
    assert_eq!(
        e,
        StationEvent::PairingClosed {
            reason: PairingEnd::CodeFailures
        }
    );
    // The right code no longer works either (FR-PAIR-3).
    let e = listener
        .pair_code(&info.code, station.device_id(), vec![route])
        .await
        .unwrap_err();
    assert_eq!(e, ListenerError::Refused("pairing-closed".into()));
    assert!(station.trusted_listeners().is_empty());
    station.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_pair_3_invitation_only_while_pairing_is_open() {
    let storage = lull_core::MemoryStorage::new();
    let mut cfg = station_config("Nursery");
    cfg.max_pairings = 1;
    let station = station_with(cfg, &storage).await;
    let mut sev = station.events();
    let info = station.open_pairing().await.unwrap();
    let (first, _) = listener("First");
    first.pair_invitation(&info.invite_uri).await.unwrap();
    // The window is full after max_pairings pairings.
    let e = wait_event(&mut sev, "closed", |e| {
        matches!(e, StationEvent::PairingClosed { .. })
    })
    .await;
    assert_eq!(
        e,
        StationEvent::PairingClosed {
            reason: PairingEnd::Full
        }
    );
    assert!(station.snapshot().pairing.is_none());
    let (second, _) = listener("Second");
    let e = second.pair_invitation(&info.invite_uri).await.unwrap_err();
    assert_eq!(e, ListenerError::Refused("pairing-closed".into()));

    // A new window with a new token; the old invitation stays invalid.
    let info2 = station.open_pairing().await.unwrap();
    assert_ne!(info2.invite_uri, info.invite_uri);
    station.close_pairing();
    let e = second.pair_invitation(&info2.invite_uri).await.unwrap_err();
    assert_eq!(e, ListenerError::Refused("pairing-closed".into()));
    station.shutdown().await;
    first.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_pair_3_window_expires() {
    let storage = lull_core::MemoryStorage::new();
    let mut cfg = station_config("Nursery");
    cfg.pairing_window = Duration::from_millis(600);
    let station = station_with(cfg, &storage).await;
    let mut sev = station.events();
    let info = station.open_pairing().await.unwrap();
    let e = wait_event(&mut sev, "expired", |e| {
        matches!(e, StationEvent::PairingClosed { .. })
    })
    .await;
    assert_eq!(
        e,
        StationEvent::PairingClosed {
            reason: PairingEnd::Expired
        }
    );
    let (l, _) = listener("Late");
    let e = l.pair_invitation(&info.invite_uri).await.unwrap_err();
    assert_eq!(e, ListenerError::Refused("pairing-closed".into()));
    station.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_pair_6_revocation_ends_sessions_and_rejects_reconnects() {
    let (station, _) = station("Nursery").await;
    let (listener, _) = listener("Lost phone");
    let id = pair(&station, &listener).await;
    let mut lev = listener.events();
    let mut sev = station.events();
    assert!(station.revoke(listener.device_id()));
    wait_event(&mut lev, "needs pairing", |e| {
        matches!(e, ListenerEvent::NeedsPairing { .. })
    })
    .await;
    wait_event(&mut sev, "revoked", |e| {
        matches!(e, StationEvent::ListenerRevoked { .. })
    })
    .await;
    let s = listener.snapshot();
    assert_eq!(s.stations[0].status, ConnStatus::NeedsPairing);
    assert!(station.trusted_listeners().is_empty());
    let mut ss = station.state();
    wait_state(&mut ss, "no listeners", |s| s.listeners.is_empty()).await;

    // Following again: trusted mode is rejected silently (protocol.md §5.1).
    listener.follow(&id).unwrap();
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(!is_live(&listener.snapshot(), &id));
    assert!(station.snapshot().listeners.is_empty());
    station.shutdown().await;
    listener.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_pair_6_forget_sends_unpair() {
    let (station, _) = station("Nursery").await;
    let (listener, _) = listener("Old tablet");
    let id = pair(&station, &listener).await;
    let mut sev = station.events();
    listener.forget(&id).unwrap();
    let e = wait_event(&mut sev, "unpaired", |e| {
        matches!(e, StationEvent::ListenerUnpaired { .. })
    })
    .await;
    assert_eq!(
        e,
        StationEvent::ListenerUnpaired {
            listener_id: listener.device_id().to_owned()
        }
    );
    assert!(station.trusted_listeners().is_empty());
    assert!(listener.known_stations().is_empty());
    station.shutdown().await;
    listener.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_pair_4_reconnects_in_trusted_mode_after_station_restart() {
    let storage = lull_core::MemoryStorage::new();
    let port = free_port();
    let mut cfg = station_config("Nursery");
    cfg.tcp = loopback(port);
    let station = station_with(cfg.clone(), &storage).await;
    let (listener, lstore) = listener("Parents");
    let id = pair(&station, &listener).await;
    let mut lev = listener.events();
    station.shutdown().await;
    drop(station);
    wait_event(&mut lev, "disconnected", |e| {
        matches!(e, ListenerEvent::Disconnected { .. })
    })
    .await;
    let mut ls = listener.state();
    wait_state(&mut ls, "reconnecting", |s| {
        matches!(s.stations[0].status, ConnStatus::Reconnecting { .. })
    })
    .await;

    // Same storage, same port, pairing closed (FR-PAIR-4).
    let station = station_with(cfg, &storage).await;
    assert!(station.snapshot().pairing.is_none());
    wait_event(&mut lev, "connected again", |e| {
        matches!(e, ListenerEvent::Connected { .. })
    })
    .await;
    wait_state(&mut ls, "live", |s| is_live(s, &id)).await;

    // A restarted listener reconnects from storage too.
    listener.shutdown().await;
    drop(listener);
    let listener = listener_with(listener_config("Parents"), &lstore);
    let mut ls = listener.state();
    wait_state(&mut ls, "live after listener restart", |s| is_live(s, &id)).await;
    station.shutdown().await;
    listener.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn protocol_9_busy_beyond_the_listener_limit() {
    let storage = lull_core::MemoryStorage::new();
    let mut cfg = station_config("Nursery");
    cfg.max_listeners = 2;
    let station = station_with(cfg, &storage).await;
    let (a, _) = listener("A");
    let (b, _) = listener("B");
    let (c, _) = listener("C");
    pair(&station, &a).await;
    pair(&station, &b).await;
    let info = station.open_pairing().await.unwrap();
    let e = c.pair_invitation(&info.invite_uri).await.unwrap_err();
    assert_eq!(e, ListenerError::Refused("busy".into()));
    assert_eq!(station.snapshot().listeners.len(), 2);
    for l in [&a, &b, &c] {
        l.shutdown().await;
    }
    station.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn protocol_9_same_listener_again_replaces_the_old_session() {
    let (station, _) = station("Nursery").await;
    let (first, lstore) = listener("Phone");
    let id = pair(&station, &first).await;
    let mut sev = station.events();
    let mut fev = first.events();
    // A second engine on the same storage is the same listener key.
    let second = listener_with(listener_config("Phone"), &lstore);
    let e = wait_event(&mut sev, "replaced", |e| {
        matches!(e, StationEvent::SessionReplaced { .. })
    })
    .await;
    let StationEvent::SessionReplaced { listener_id, .. } = e else {
        unreachable!()
    };
    assert_eq!(listener_id, first.device_id());
    let e = wait_event(&mut fev, "first disconnected", |e| {
        matches!(e, ListenerEvent::Disconnected { .. })
    })
    .await;
    let ListenerEvent::Disconnected { reason, .. } = e else {
        unreachable!()
    };
    assert!(reason.contains("replaced"), "{reason}");
    second.shutdown().await;
    let mut ls = first.state();
    wait_state(&mut ls, "first live again", |s| is_live(s, &id)).await;
    first.shutdown().await;
    station.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_pair_3_ten_minute_window_on_a_manual_clock() {
    use std::sync::Arc;
    let clock = Arc::new(lull_core::ManualClock::new(5_000, 1_700_000_000_000));
    let station = lull_core::station::StationEngine::start(
        station_config("Nursery"),
        Arc::new(lull_core::MemoryStorage::new()),
        clock.clone(),
    )
    .await
    .unwrap();
    let mut sev = station.events();
    let info = station.open_pairing().await.unwrap();
    assert_eq!(info.closes_at_ms, 5_000 + 600_000);
    clock.advance(599_000);
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(station.snapshot().pairing.is_some());
    clock.advance(1_000);
    let e = wait_event(&mut sev, "expired", |e| {
        matches!(e, StationEvent::PairingClosed { .. })
    })
    .await;
    assert_eq!(
        e,
        StationEvent::PairingClosed {
            reason: PairingEnd::Expired
        }
    );
    station.shutdown().await;
}
