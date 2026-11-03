// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reconnect, alerts on reachability and address memory (FR-LS-5,
//! FR-AL-2, FR-NET-2, FR-NET-3) over loopback.

mod common;

use std::time::Duration;

use common::*;
use nestwatch_core::listener::{Alert, ConnStatus, ListenSettings, ListenerEvent, Route};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_al_2_unreachable_and_reachable_again() {
    let storage = nestwatch_core::MemoryStorage::new();
    let port = free_port();
    let mut cfg = station_config("Nursery");
    cfg.tcp = loopback(port);
    let station = station_with(cfg.clone(), &storage).await;
    let mut lcfg = listener_config("Parents");
    lcfg.unreachable_alert_after = Duration::from_millis(1_000);
    let (l, _) = (
        listener_with(lcfg, &nestwatch_core::MemoryStorage::new()),
        (),
    );
    let id = pair(&station, &l).await;
    let mut lev = l.events();

    station.shutdown().await;
    drop(station);
    let t0 = std::time::Instant::now();
    let e = wait_event(&mut lev, "unreachable", |e| {
        matches!(e, ListenerEvent::Alert { .. })
    })
    .await;
    assert_eq!(
        e,
        ListenerEvent::Alert {
            station_id: id.clone(),
            alert: Alert::Unreachable
        }
    );
    assert!(
        t0.elapsed() >= Duration::from_millis(900),
        "too early: {:?}",
        t0.elapsed()
    );
    // FR-LS-5: reconnect progress with the addresses tried.
    let mut ls = l.state();
    let s = wait_state(&mut ls, "reconnect progress", |s| {
        matches!(&s.stations[0].status, ConnStatus::Reconnecting { attempt, tried } if *attempt >= 2 && !tried.is_empty())
    })
    .await;
    let ConnStatus::Reconnecting { tried, .. } = &s.stations[0].status else {
        unreachable!()
    };
    assert_eq!(tried[0], format!("127.0.0.1:{port}"));

    let station = station_with(cfg, &storage).await;
    let e = wait_event(&mut lev, "reachable", |e| {
        matches!(e, ListenerEvent::Alert { .. })
    })
    .await;
    assert_eq!(
        e,
        ListenerEvent::Alert {
            station_id: id.clone(),
            alert: Alert::Reachable
        }
    );
    wait_state(&mut ls, "live", |s| is_live(s, &id)).await;
    station.shutdown().await;
    l.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_al_4_alerts_while_muted() {
    let (station, _) = station("Nursery").await;
    let (l, _) = listener("Parents");
    let id = pair(&station, &l).await;
    l.set_muted(&id, true).unwrap();
    l.set_listen(&id, ListenSettings::activated(-30.0, 100, 100))
        .unwrap();
    let mut ss = station.state();
    wait_state(&mut ss, "activated", |s| {
        s.listeners
            .first()
            .is_some_and(|x| x.mode == nestwatch_core::ListenMode::Activated)
    })
    .await;
    let mut lev = l.events();
    let mut tone = Tone::new(440.0, 16_000);
    feed(&station, &mut tone, -10.0, 200).await;
    let e = wait_event(&mut lev, "alert", |e| {
        matches!(e, ListenerEvent::Alert { .. })
    })
    .await;
    assert!(matches!(
        e,
        ListenerEvent::Alert {
            alert: Alert::SoundStarted,
            ..
        }
    ));
    station.shutdown().await;
    l.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_net_3_remembers_working_addresses_in_order_of_success() {
    let (station, _) = station("Nursery").await;
    let (l, lstore) = listener("Parents");
    let id = pair(&station, &l).await;
    let good = format!("127.0.0.1:{}", station.snapshot().tcp_port);
    // FR-NET-2: a manually entered address that does not work.
    let dead = Route::parse(&format!("127.0.0.1:{}", free_port())).unwrap();
    l.add_route(&id, dead.clone()).unwrap();
    let known = &l.known_stations()[0];
    let ok = known
        .endpoints
        .iter()
        .find(|e| e.route.to_string() == good)
        .unwrap();
    assert!(ok.last_success_unix_ms.is_some());
    let bad = known.endpoints.iter().find(|e| e.route == dead).unwrap();
    assert!(bad.last_success_unix_ms.is_none());
    assert_eq!(known.routes_by_success()[0].to_string(), good);

    // After a restart of the listener the working address is tried first.
    l.shutdown().await;
    drop(l);
    let l = listener_with(listener_config("Parents"), &lstore);
    let mut ls = l.state();
    let s = wait_state(&mut ls, "live", |s| is_live(s, &id)).await;
    assert_eq!(s.stations[0].route.as_deref(), Some(good.as_str()));
    assert_eq!(
        s.stations[0].path,
        Some(nestwatch_core::listener::NetPath::Lan)
    );
    station.shutdown().await;
    l.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_ls_1_unfollow_and_follow_again() {
    let (station, _) = station("Nursery").await;
    let (l, _) = listener("Parents");
    let id = pair(&station, &l).await;
    l.unfollow(&id).unwrap();
    let mut ss = station.state();
    wait_state(&mut ss, "listener gone", |s| s.listeners.is_empty()).await;
    assert_eq!(l.snapshot().stations[0].status, ConnStatus::Idle);
    l.follow(&id).unwrap();
    wait_state(&mut ss, "listener back", |s| s.listeners.len() == 1).await;
    station.shutdown().await;
    l.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_al_2_unreachable_after_20_s_on_a_manual_clock() {
    use std::sync::Arc;
    let (station, _) = station("Nursery").await;
    let clock = Arc::new(nestwatch_core::ManualClock::new(1_000, 1_700_000_000_000));
    let l = nestwatch_core::listener::ListenerEngine::start(
        listener_config("Parents"),
        Arc::new(nestwatch_core::MemoryStorage::new()),
        clock.clone(),
    )
    .unwrap();
    let id = pair(&station, &l).await;
    let mut lev = l.events();
    station.shutdown().await;
    wait_event(&mut lev, "disconnected", |e| {
        matches!(e, ListenerEvent::Disconnected { .. })
    })
    .await;
    // Spec value (20 s): nothing at 19 s, the alert just after 20 s.
    clock.advance(19_000);
    let early = wait_event_for(&mut lev, Duration::from_millis(400), "early", |e| {
        matches!(e, ListenerEvent::Alert { .. })
    })
    .await;
    assert_eq!(early, None);
    clock.advance(1_500);
    let e = wait_event(&mut lev, "unreachable", |e| {
        matches!(e, ListenerEvent::Alert { .. })
    })
    .await;
    assert_eq!(
        e,
        ListenerEvent::Alert {
            station_id: id,
            alert: Alert::Unreachable
        }
    );
    l.shutdown().await;
}
