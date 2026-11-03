// SPDX-License-Identifier: AGPL-3.0-or-later
//! Both engines with an in-process `nestwatch-server` on loopback
//! (rendezvous.md FR-RV-1..4, §3.2, §4, §5): relay-only invitations, the
//! switch from relayed to direct without an audio gap, the relay setting
//! and its mailbox, and the relay as the last route.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use common::*;
use nestwatch_core::listener::{
    ConnStatus, ListenSettings, ListenerConfig, ListenerEngine, NetPath, StationView,
};
use nestwatch_core::station::{RelayStatus, StationConfig, StationEngine, StationState};
use nestwatch_core::{MemoryStorage, Transport};
use nestwatch_server::{Config, Server};

/// One 20 ms playback pull: its RMS and the station's path at that time.
type Pulled = (f32, Option<NetPath>);

/// An in-process relay server on loopback ports chosen by the OS.
async fn server() -> (Server, tempfile::TempDir) {
    let state = tempfile::tempdir().unwrap();
    let mut c = Config {
        http: "127.0.0.1:0".parse().unwrap(),
        stun: Some("127.0.0.1:0".parse().unwrap()),
        state_dir: state.path().to_owned(),
        log_interval: 0.0,
        ..Config::default()
    };
    c.limits.connect_per_minute = 1000;
    c.limits.register_per_minute = 1000;
    (Server::start(c).await.expect("server starts"), state)
}

fn origin(s: &Server) -> String {
    format!("http://{}", s.http_addr())
}

fn relay_station_config(
    s: &Server,
    endpoints: Option<Vec<nestwatch_core::Endpoint>>,
) -> StationConfig {
    let mut cfg = station_config("Nursery");
    cfg.endpoints = endpoints;
    cfg.relay.stun_port = s.stun_addr().map(|a| a.port());
    cfg.relay.backoff_min = Duration::from_millis(50);
    cfg.relay.backoff_max = Duration::from_millis(200);
    cfg.relay.timeout = Duration::from_secs(5);
    cfg
}

fn relay_listener_config(s: &Server) -> ListenerConfig {
    let mut cfg = listener_config("Parents");
    cfg.relay.stun_port = s.stun_addr().map(|a| a.port());
    cfg.relay.punch_bind = "127.0.0.1:0".parse().unwrap();
    cfg.relay.timeout = Duration::from_secs(5);
    cfg
}

async fn online(station: &StationEngine) -> StationState {
    let mut st = station.state();
    wait_state(&mut st, "relay online", |s| {
        matches!(s.relay, RelayStatus::Online { .. })
    })
    .await
}

async fn relay_station(
    s: &Server,
    storage: &MemoryStorage,
    endpoints: Option<Vec<nestwatch_core::Endpoint>>,
) -> StationEngine {
    let station = station_with(relay_station_config(s, endpoints), storage).await;
    station.set_relay_server(&origin(s)).unwrap();
    station.set_relay_enabled(true).unwrap();
    online(&station).await;
    station
}

fn view(l: &ListenerEngine, id: &str) -> StationView {
    l.snapshot()
        .stations
        .into_iter()
        .find(|v| v.id == id)
        .expect("known station")
}

async fn wait_path(l: &ListenerEngine, id: &str, path: NetPath) -> StationView {
    let mut st = l.state();
    let s = wait_state(&mut st, &format!("path {path:?}"), |s| {
        s.stations
            .iter()
            .any(|v| v.id == id && v.status == ConnStatus::Live && v.path == Some(path))
    })
    .await;
    s.stations.into_iter().find(|v| v.id == id).unwrap()
}

/// FR-RV-2, FR-RV-3, §5.2: an invitation with only `r`/`m` (no endpoints)
/// pairs through the relay, audio flows relayed, then the session moves to
/// the punched QUIC path; the playback gap across the switch is measured.
#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn relay_only_invitation_then_direct_without_audio_gap() {
    let (srv, _state) = server().await;
    let station = relay_station(&srv, &MemoryStorage::new(), Some(Vec::new())).await;
    let info = station.open_pairing().await.unwrap();
    let inv = nestwatch_proto::Invitation::parse(&info.invite_uri).unwrap();
    assert!(inv.endpoints.is_empty(), "{}", info.invite_uri);
    let relay = inv.relay.expect("relay parameters");
    assert_eq!(relay.server, origin(&srv));
    assert_eq!(
        Some(relay.mailbox.to_vec()),
        station.relay_mailbox().map(|m| {
            nestwatch_relay::MailboxId::parse(&m)
                .unwrap()
                .as_bytes()
                .to_vec()
        })
    );
    assert!(
        info.relay_web_url
            .as_deref()
            .unwrap()
            .starts_with(&format!("{}/app/#", origin(&srv)))
    );

    let mut cfg = relay_listener_config(&srv);
    cfg.relay.first_punch_after = Duration::from_millis(1500);
    let listener = listener_with(cfg, &MemoryStorage::new());

    // The station feeds a tone; the listener pulls playback in real time.
    let feeding = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let feeder = {
        let (station, feeding) = (station.clone(), feeding.clone());
        tokio::spawn(async move {
            let mut tone = Tone::new(440.0, 48_000);
            let mut tick = tokio::time::interval(Duration::from_millis(20));
            while feeding.load(std::sync::atomic::Ordering::Relaxed) {
                tick.tick().await;
                station.push_capture(&tone.samples(960, -20.0), 48_000, 1);
            }
        })
    };
    let mut events = listener.events();
    let r = listener.pair_invitation(&info.invite_uri).await.unwrap();
    let id = r.station_id.clone();
    listener
        .set_listen(&id, ListenSettings::continuous())
        .unwrap();
    let relayed = wait_path(&listener, &id, NetPath::Relayed).await;
    assert!(
        relayed.route.as_deref().unwrap().starts_with("relay "),
        "{relayed:?}"
    );
    let ss = station.snapshot();
    assert_eq!(ss.listeners.len(), 1);
    assert_eq!(ss.listeners[0].transport, Transport::Relayed);

    // Playback: one RMS value per 20 ms pull, with the path at that time.
    let samples: Arc<Mutex<Vec<Pulled>>> = Arc::default();
    let pulling = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let puller = {
        let (l, samples, pulling, id) = (
            listener.clone(),
            samples.clone(),
            pulling.clone(),
            id.clone(),
        );
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(20));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
            let mut buf = [0i16; 320];
            while pulling.load(std::sync::atomic::Ordering::Relaxed) {
                tick.tick().await;
                l.pull_playback(&mut buf);
                let rms = (buf.iter().map(|&v| f64::from(v).powi(2)).sum::<f64>() / 320.0).sqrt();
                let path = l
                    .snapshot()
                    .stations
                    .iter()
                    .find(|v| v.id == id)
                    .and_then(|v| v.path);
                samples.lock().unwrap().push((rms as f32, path));
            }
        })
    };

    // Relayed audio flows.
    tokio::time::sleep(Duration::from_millis(800)).await;
    let before = view(&listener, &id).audio_frames;
    assert!(before >= 20, "relayed frames: {before}");

    // FR-RV-3: the direct path forms and replaces the relayed session.
    let direct = wait_path(&listener, &id, NetPath::Direct).await;
    assert!(
        direct.route.as_deref().unwrap().starts_with("direct "),
        "{direct:?}"
    );
    let mut ss = station.state();
    wait_state(&mut ss, "only the direct session left", |s| {
        s.listeners.len() == 1 && s.listeners[0].transport == Transport::Direct
    })
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    pulling.store(false, std::sync::atomic::Ordering::Relaxed);
    puller.await.unwrap();
    feeding.store(false, std::sync::atomic::Ordering::Relaxed);
    feeder.await.unwrap();

    // The switch is seamless: no disconnection on the listener.
    while let Ok(e) = events.try_recv() {
        assert!(
            !matches!(e, nestwatch_core::listener::ListenerEvent::Disconnected { .. }),
            "{e:?}"
        );
    }
    let samples = samples.lock().unwrap().clone();
    // -20 dBFS RMS is ~3277; silence (and concealment fades) far below.
    let audible = |rms: f32| rms > 500.0;
    let first = samples
        .iter()
        .position(|s| audible(s.0))
        .expect("audio played");
    let switch = samples
        .iter()
        .position(|s| s.1 == Some(NetPath::Direct))
        .expect("switch seen in playback");
    assert!(first < switch, "audio before the switch");
    assert!(
        samples[switch..].iter().filter(|s| audible(s.0)).count() >= 50,
        "direct audio plays"
    );
    // Longest silent run after audio started.
    let (mut longest, mut run) = (0usize, 0usize);
    for s in &samples[first..] {
        if audible(s.0) {
            run = 0;
        } else {
            run += 1;
            longest = longest.max(run);
        }
    }
    let gap_ms = longest * 20;
    eprintln!("relay->direct switch: longest playback gap {gap_ms} ms");
    assert!(gap_ms <= 500, "playback gap {gap_ms} ms");

    listener.shutdown().await;
    station.shutdown().await;
    srv.shutdown().await;
}

/// FR-RV-1: the relay is off by default: no registration, no `r`/`m`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_off_means_no_registration_and_no_relay_in_invitations() {
    let (srv, _state) = server().await;
    let station = station_with(relay_station_config(&srv, None), &MemoryStorage::new()).await;
    // Only the server address is set; the setting stays off.
    station.set_relay_server(&origin(&srv)).unwrap();
    let s = station.snapshot();
    assert_eq!(s.relay, RelayStatus::Off);
    assert!(!s.settings.relay_enabled);
    let info = station.open_pairing().await.unwrap();
    assert!(!info.invite_uri.contains("r="), "{}", info.invite_uri);
    assert!(!info.invite_uri.contains("m="), "{}", info.invite_uri);
    assert_eq!(info.relay_web_url, None);
    assert_eq!(station.relay_mailbox(), None);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(srv.metrics().registrations_total, 0);
    assert_eq!(srv.metrics().mailboxes_online, 0);

    // Turning it on updates the open window's invitation; off again removes it.
    station.set_relay_enabled(true).unwrap();
    online(&station).await;
    let with = station.snapshot().pairing.unwrap().invite_uri;
    assert!(with.contains("&r=") && with.contains("&m="), "{with}");
    station.set_relay_enabled(false).unwrap();
    let mut st = station.state();
    let s = wait_state(&mut st, "relay off", |s| s.relay == RelayStatus::Off).await;
    assert!(!s.pairing.unwrap().invite_uri.contains("m="));
    station.shutdown().await;
    srv.shutdown().await;
}

/// §3.2: the mailbox survives restarts and off/on; "reset relay identity"
/// makes a new one.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn mailbox_persists_across_restarts() {
    let (srv, _state) = server().await;
    let storage = MemoryStorage::new();
    let station = relay_station(&srv, &storage, None).await;
    let m1 = station.relay_mailbox().unwrap();
    station.shutdown().await;

    let station = station_with(relay_station_config(&srv, None), &storage).await;
    assert!(
        station.snapshot().settings.relay_enabled,
        "setting persisted"
    );
    assert_eq!(station.snapshot().settings.relay_server, origin(&srv));
    online(&station).await;
    assert_eq!(station.relay_mailbox().as_deref(), Some(m1.as_str()));

    station.set_relay_enabled(false).unwrap();
    station.set_relay_enabled(true).unwrap();
    online(&station).await;
    assert_eq!(station.relay_mailbox().as_deref(), Some(m1.as_str()));

    station.reset_relay_identity().unwrap();
    online(&station).await;
    let m2 = station.relay_mailbox().unwrap();
    assert_ne!(m1, m2);
    station.shutdown().await;
    let station = station_with(relay_station_config(&srv, None), &storage).await;
    assert_eq!(station.relay_mailbox().as_deref(), Some(m2.as_str()));
    station.shutdown().await;
    srv.shutdown().await;
}

/// FR-NET-3 / FR-RV-2: stored endpoints first, the relay when they are dead.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn listener_falls_back_to_relay_when_stored_endpoints_are_dead() {
    let (srv, _state) = server().await;
    let storage = MemoryStorage::new();
    let station = relay_station(&srv, &storage, None).await;
    let mut cfg = relay_listener_config(&srv);
    cfg.relay.direct = false;
    let listener = listener_with(cfg, &MemoryStorage::new());
    let id = pair(&station, &listener).await;
    let v = view(&listener, &id);
    assert_eq!(v.path, Some(NetPath::HomeNetwork), "{v:?}");
    let known = &listener.known_stations()[0];
    assert!(known.relay.is_some(), "relay stored per station");

    // The station comes back on another port: the stored endpoint is dead.
    station.shutdown().await;
    let station = station_with(relay_station_config(&srv, None), &storage).await;
    online(&station).await;
    let v = wait_path(&listener, &id, NetPath::Relayed).await;
    assert!(v.route.unwrap().starts_with("relay "));
    let mut ss = station.state();
    wait_state(&mut ss, "relayed listener", |s| {
        s.listeners
            .iter()
            .any(|l| l.transport == Transport::Relayed)
    })
    .await;
    listener.shutdown().await;
    station.shutdown().await;
    srv.shutdown().await;
}
