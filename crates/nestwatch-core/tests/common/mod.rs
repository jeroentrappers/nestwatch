// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared helpers for engine tests: in-memory storage, real loopback
//! sockets on OS-chosen (non-default) ports, fast timers.
#![allow(dead_code)]

use std::fmt::Debug;
use std::sync::Arc;
use std::time::Duration;

use nestwatch_core::listener::{ListenerConfig, ListenerEngine, ListenerEvent, ListenerState};
use nestwatch_core::station::{StationConfig, StationEngine, StationEvent, WebOptions};
use nestwatch_core::{MemoryStorage, Storage, SystemClock};
use nestwatch_net::web::{AssetSource, WebConfig};
use nestwatch_net::{BindScope, ListenConfig};
use tokio::sync::{broadcast, watch};

pub const WAIT: Duration = Duration::from_secs(15);

pub fn loopback(port: u16) -> ListenConfig {
    ListenConfig {
        port,
        attempts: 1,
        scope: BindScope::Loopback,
    }
}

/// A free loopback port (bound once, then released).
pub fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

pub fn station_config(name: &str) -> StationConfig {
    let mut cfg = StationConfig::loopback(name);
    cfg.status_interval = Duration::from_secs(5);
    cfg
}

pub fn with_web(mut cfg: StationConfig) -> StationConfig {
    cfg.web = Some(WebOptions {
        config: WebConfig {
            http: Some(loopback(0)),
            https: None,
            ..WebConfig::default()
        },
        assets: AssetSource::memory([(
            "index.html",
            &b"<!doctype html><title>Nestwatch</title>"[..],
        )]),
    });
    cfg
}

pub async fn station_with(cfg: StationConfig, storage: &MemoryStorage) -> StationEngine {
    StationEngine::start(cfg, Arc::new(storage.clone()), Arc::new(SystemClock::new()))
        .await
        .expect("station starts")
}

pub async fn station(name: &str) -> (StationEngine, MemoryStorage) {
    let storage = MemoryStorage::new();
    (station_with(station_config(name), &storage).await, storage)
}

pub fn listener_config(name: &str) -> ListenerConfig {
    let mut cfg = ListenerConfig::new(name);
    cfg.discovery = false;
    cfg.dial_timeout = Duration::from_secs(2);
    cfg.backoff_min = Duration::from_millis(100);
    cfg.backoff_max = Duration::from_millis(400);
    cfg
}

pub fn listener_with(cfg: ListenerConfig, storage: &MemoryStorage) -> ListenerEngine {
    ListenerEngine::start(
        cfg,
        Arc::new(storage.clone()) as Arc<dyn Storage>,
        Arc::new(SystemClock::new()),
    )
    .expect("listener starts")
}

pub fn listener(name: &str) -> (ListenerEngine, MemoryStorage) {
    let storage = MemoryStorage::new();
    (listener_with(listener_config(name), &storage), storage)
}

/// Waits until the watched value satisfies `f`.
pub async fn wait_state<T: Clone + Debug>(
    rx: &mut watch::Receiver<T>,
    what: &str,
    f: impl Fn(&T) -> bool,
) -> T {
    let r = tokio::time::timeout(WAIT, async {
        loop {
            {
                let v = rx.borrow_and_update();
                if f(&v) {
                    return v.clone();
                }
            }
            rx.changed().await.expect("engine alive");
        }
    })
    .await;
    match r {
        Ok(v) => v,
        Err(_) => panic!("timed out waiting for {what}; last: {:#?}", *rx.borrow()),
    }
}

/// Waits for an event matching `f`.
pub async fn wait_event<E: Clone + Debug>(
    rx: &mut broadcast::Receiver<E>,
    what: &str,
    f: impl Fn(&E) -> bool,
) -> E {
    wait_event_for(rx, WAIT, what, f)
        .await
        .unwrap_or_else(|| panic!("timed out waiting for {what}"))
}

/// Waits up to `d` for an event matching `f`.
pub async fn wait_event_for<E: Clone + Debug>(
    rx: &mut broadcast::Receiver<E>,
    d: Duration,
    _what: &str,
    f: impl Fn(&E) -> bool,
) -> Option<E> {
    tokio::time::timeout(d, async {
        loop {
            match rx.recv().await {
                Ok(e) if f(&e) => return e,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => panic!("event channel closed"),
            }
        }
    })
    .await
    .ok()
}

pub fn is_live(state: &ListenerState, station_id: &str) -> bool {
    state
        .stations
        .iter()
        .any(|s| s.id == station_id && s.status == nestwatch_core::listener::ConnStatus::Live)
}

/// Pairs `listener` with `station` by invitation and waits until live.
pub async fn pair(station: &StationEngine, listener: &ListenerEngine) -> String {
    let info = station.open_pairing().await.expect("pairing opens");
    let r = listener
        .pair_invitation(&info.invite_uri)
        .await
        .expect("pairing succeeds");
    let mut st = listener.state();
    wait_state(&mut st, "live after pairing", |s| is_live(s, &r.station_id)).await;
    r.station_id
}

/// A sine frame generator at `level_dbfs` RMS.
pub struct Tone {
    pub freq: f32,
    pub rate: u32,
    phase: f32,
}

impl Tone {
    pub fn new(freq: f32, rate: u32) -> Self {
        Self {
            freq,
            rate,
            phase: 0.0,
        }
    }

    pub fn samples(&mut self, n: usize, level_dbfs: f32) -> Vec<i16> {
        let amp = 32768.0 * 10f32.powf(level_dbfs / 20.0) * std::f32::consts::SQRT_2;
        (0..n)
            .map(|_| {
                let s = (amp * self.phase.sin()).round().clamp(-32768.0, 32767.0) as i16;
                self.phase += 2.0 * std::f32::consts::PI * self.freq / self.rate as f32;
                if self.phase > 2.0 * std::f32::consts::PI {
                    self.phase -= 2.0 * std::f32::consts::PI;
                }
                s
            })
            .collect()
    }
}

/// Pushes `ms` of audio at `level_dbfs` (silence below -100) in real time,
/// 20 ms at a time, at 48 kHz.
pub async fn feed(station: &StationEngine, tone: &mut Tone, level_dbfs: f32, ms: u64) {
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    for _ in 0..ms / 20 {
        tick.tick().await;
        let n = (tone.rate / 50) as usize;
        let s = if level_dbfs < -100.0 {
            vec![0i16; n]
        } else {
            tone.samples(n, level_dbfs)
        };
        station.push_capture(&s, tone.rate, 1);
    }
}

/// Goertzel power of `freq` in `x` (16 kHz), normalised.
pub fn tone_power(x: &[i16], freq: f32) -> f32 {
    let w = 2.0 * std::f32::consts::PI * freq / 16_000.0;
    let c = 2.0 * w.cos();
    let (mut s1, mut s2) = (0f32, 0f32);
    for &v in x {
        let s = f32::from(v) / 32768.0 + c * s1 - s2;
        s2 = s1;
        s1 = s;
    }
    (s1 * s1 + s2 * s2 - c * s1 * s2) / (x.len() as f32 * x.len() as f32)
}

pub fn events_station(s: &StationEngine) -> broadcast::Receiver<StationEvent> {
    s.events()
}

pub fn events_listener(l: &ListenerEngine) -> broadcast::Receiver<ListenerEvent> {
    l.events()
}
