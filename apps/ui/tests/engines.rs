// SPDX-License-Identifier: AGPL-3.0-or-later
//! The listener and station adapters against real engines over loopback.
#![cfg(feature = "native")]

use std::sync::Arc;
use std::time::Duration;

use nestwatch_core::listener::{ListenerConfig, ListenerEngine};
use nestwatch_core::station::{StationConfig, StationEngine};
use nestwatch_core::{Clock, MemoryStorage, SystemClock};
use nestwatch_ui::adapter::listener::{Tracker, map_listener};
use nestwatch_ui::adapter::station::map_station;
use nestwatch_ui::logic;
use nestwatch_ui::model::{Conn, Mode, NetPath};

#[tokio::test(flavor = "multi_thread")]
async fn paired_listener_maps_live_and_station_lists_it() {
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let station = StationEngine::start(
        StationConfig::loopback("Nursery"),
        Arc::new(MemoryStorage::new()),
        clock.clone(),
    )
    .await
    .unwrap();
    let info = station.open_pairing().await.unwrap();
    let mut cfg = ListenerConfig::new("Laptop");
    cfg.discovery = false;
    let listener =
        ListenerEngine::start(cfg, Arc::new(MemoryStorage::new()), clock.clone()).unwrap();
    let paired = listener.pair_invitation(&info.invite_uri).await.unwrap();
    assert_eq!(paired.verification.len(), 7);

    // Feed a loud tone so levels and sound activation move.
    let feeder = {
        let station = station.clone();
        tokio::spawn(async move {
            let mut phase = 0f32;
            loop {
                let s: Vec<i16> = (0..960)
                    .map(|_| {
                        phase += 2.0 * std::f32::consts::PI * 440.0 / 48_000.0;
                        (phase.sin() * 8_000.0) as i16
                    })
                    .collect();
                station.push_capture(&s, 48_000, 1);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
    };
    let mut tracker = Tracker::default();
    let mut live = None;
    for _ in 0..100 {
        let m = map_listener(&listener.snapshot(), &mut tracker, clock.now_ms(), true);
        if let Some(t) = m.tiles.first()
            && t.conn == Conn::Live
            && t.level_dbfs.is_some_and(|l| l > -40.0)
        {
            live = Some(t.clone());
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let tile = live.expect("tile goes live with a level");
    assert_eq!(tile.name, "Nursery");
    assert_eq!(tile.path, Some(NetPath::Lan));
    assert_eq!(tile.settings.mode, Mode::Activated);
    assert!(logic::has_sound(&tile) || !tile.open);

    let sm = map_station(
        &station.snapshot(),
        &station.trusted_listeners(),
        clock.now_ms(),
    );
    assert_eq!(sm.listeners.len(), 1);
    assert_eq!(sm.listeners[0].name, "Laptop");
    assert_eq!(sm.listeners[0].mode, Mode::Activated);
    assert_eq!(sm.listeners[0].threshold_dbfs, -50.0);
    assert_eq!(logic::strictest_threshold(&sm.listeners), Some(-50.0));
    assert_eq!(sm.trusted.len(), 1);
    assert_eq!(sm.pairing.as_ref().map(|p| p.pairings), Some(1));

    feeder.abort();
    listener.shutdown().await;
    station.shutdown().await;
}
