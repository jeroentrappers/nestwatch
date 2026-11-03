// SPDX-License-Identifier: AGPL-3.0-or-later
//! The FFI surface end to end over loopback: a station and a listener
//! created through the exported API, paired by invitation, with audio,
//! levels, talk-back state and video moving through the observers.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nestwatch_ffi::*;

#[derive(Default)]
struct MemStore(Mutex<BTreeMap<String, Vec<u8>>>);

impl Storage for MemStore {
    fn get(&self, key: String) -> Result<Option<Vec<u8>>, StorageFailure> {
        Ok(self.0.lock().unwrap().get(&key).cloned())
    }
    fn put(&self, key: String, value: Vec<u8>) -> Result<(), StorageFailure> {
        self.0.lock().unwrap().insert(key, value);
        Ok(())
    }
    fn delete(&self, key: String) -> Result<(), StorageFailure> {
        self.0.lock().unwrap().remove(&key);
        Ok(())
    }
}

/// A store that always fails, to check errors cross the boundary.
struct BrokenStore;

impl Storage for BrokenStore {
    fn get(&self, _key: String) -> Result<Option<Vec<u8>>, StorageFailure> {
        Err(StorageFailure::Backend {
            reason: "locked".into(),
        })
    }
    fn put(&self, _key: String, _value: Vec<u8>) -> Result<(), StorageFailure> {
        Err(StorageFailure::Backend {
            reason: "locked".into(),
        })
    }
    fn delete(&self, _key: String) -> Result<(), StorageFailure> {
        Ok(())
    }
}

#[derive(Default)]
struct StationRec {
    states: Mutex<Vec<StationSnapshot>>,
    events: Mutex<Vec<StationEventInfo>>,
}

impl StationObserver for StationRec {
    fn on_state(&self, state: StationSnapshot) {
        self.states.lock().unwrap().push(state);
    }
    fn on_event(&self, event: StationEventInfo) {
        self.events.lock().unwrap().push(event);
    }
}

#[derive(Default)]
struct ListenerRec {
    states: Mutex<Vec<(Instant, ListenerSnapshot)>>,
    events: Mutex<Vec<ListenerEventInfo>>,
    frames: Mutex<Vec<VideoFrame>>,
}

impl ListenerObserver for ListenerRec {
    fn on_state(&self, state: ListenerSnapshot) {
        self.states.lock().unwrap().push((Instant::now(), state));
    }
    fn on_event(&self, event: ListenerEventInfo) {
        self.events.lock().unwrap().push(event);
    }
    fn on_video_frame(&self, frame: VideoFrame) {
        self.frames.lock().unwrap().push(frame);
    }
}

fn tone(n: usize, offset: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(n * 2);
    for i in 0..n {
        let t = (i + offset) as f32 / 16_000.0;
        let s = (0.3 * (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 32767.0) as i16;
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

async fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn loopback(name: &str) -> StationOptions {
    StationOptions {
        name: name.into(),
        port: 0,
        mdns: false,
        interfaces: Vec::new(),
        extra_endpoints: Vec::new(),
        loopback: true,
    }
}

#[tokio::test]
async fn station_and_listener_through_the_ffi() {
    let srec = Arc::new(StationRec::default());
    let station = start_station(
        loopback("Nursery"),
        Arc::new(MemStore::default()),
        srec.clone(),
    )
    .await
    .unwrap();
    assert_eq!(station.snapshot().name, "Nursery");
    assert!(!station.snapshot().mdns_txt.is_empty());

    let pairing = station.open_pairing().await.unwrap();
    assert!(pairing.invite_uri.starts_with("nestwatch://"));
    assert_eq!(pairing.code.len(), 11);
    let info = parse_invitation(pairing.invite_uri.clone()).unwrap();
    assert_eq!(info.station_id, station.device_id());
    assert!(qr_code(pairing.invite_uri.clone()).is_some());

    let lrec = Arc::new(ListenerRec::default());
    let lstore = Arc::new(MemStore::default());
    let listener = Listener::start(
        ListenerOptions {
            name: "Kitchen".into(),
            discovery: false,
        },
        lstore.clone(),
        lrec.clone(),
    )
    .unwrap();
    let paired = listener
        .pair_invitation(pairing.invite_uri.clone(), Vec::new())
        .await
        .unwrap();
    assert_eq!(paired.station_id, station.device_id());
    assert_eq!(paired.verification.len(), 7);
    let sid = paired.station_id.clone();

    // Continuous audio so the tone flows regardless of the threshold.
    listener
        .set_listen(
            sid.clone(),
            ListenSettingsInfo {
                mode: ListenMode::Continuous,
                threshold_dbfs: -50.0,
                hold_ms: 4000,
                preroll_ms: 500,
            },
        )
        .unwrap();
    wait_for("live tile", || {
        listener
            .snapshot()
            .stations
            .iter()
            .any(|t| t.id == sid && t.connection == Connection::Live)
    })
    .await;
    wait_for("paired event", || {
        srec.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, StationEventInfo::Paired { name, .. } if name == "Kitchen"))
    })
    .await;
    assert_eq!(station.trusted_listeners().len(), 1);

    // Feed 20 ms blocks of tone and play back on the listener.
    let mut heard = false;
    let mut offset = 0;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        station.push_capture_i16(tone(320, offset), 16_000, 1);
        offset += 320;
        let out = listener.pull_playback(320);
        assert_eq!(out.len(), 640);
        if out
            .chunks_exact(2)
            .any(|c| i16::from_le_bytes([c[0], c[1]]).abs() > 1000)
        {
            heard = true;
        }
        let level = listener.snapshot().stations[0].level_dbfs.unwrap_or(-90.0);
        if heard && level > -30.0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(heard, "tone reached the listener's playback");
    let level = listener.snapshot().stations[0].level_dbfs.unwrap();
    assert!(level > -30.0, "level {level}");
    assert!(station.snapshot().level_dbfs > -30.0);

    // Snapshots are rate limited to about 10 per second.
    {
        let states = lrec.states.lock().unwrap();
        for w in states.windows(2) {
            assert!(w[1].0 - w[0].0 >= Duration::from_millis(90));
        }
        assert!(states.iter().any(|(_, s)| !s.stations.is_empty()));
    }

    // Camera: request, frames pushed with rotation, JPEG out.
    listener.request_video(sid.clone(), 320, 240, 10).unwrap();
    wait_for("camera wanted", || {
        srec.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, StationEventInfo::CameraWanted { .. }))
    })
    .await;
    let deadline = Instant::now() + Duration::from_secs(10);
    while lrec.frames.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "no video frame");
        let (w, h) = (64u32, 48u32);
        let ok = station.push_camera_frame(CameraImage {
            width: w,
            height: h,
            format: ImageFormat::I420,
            rotation: 90,
            data: vec![100; (w * h + 2 * (w / 2) * (h / 2)) as usize],
        });
        assert!(ok);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    {
        let f = &lrec.frames.lock().unwrap()[0];
        assert_eq!(f.station_id, sid);
        assert_eq!((f.width, f.height), (48, 64), "rotated");
        assert_eq!(&f.jpeg[..2], &[0xff, 0xd8]);
    }
    assert!(station.snapshot().camera_on);
    listener.stop_video(sid.clone()).unwrap();
    wait_for("camera not wanted", || {
        srec.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, StationEventInfo::CameraNotWanted))
    })
    .await;

    // Talk-back.
    listener.talk(sid.clone(), true).unwrap();
    wait_for("talk granted", || {
        listener.snapshot().stations[0].talk == TalkState::Granted
    })
    .await;
    assert!(
        lrec.events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e, ListenerEventInfo::MicrophoneWanted { .. }))
    );
    listener.talk(sid.clone(), false).unwrap();

    // Settings round-trip through snapshots.
    station.set_gain(Gain::Db12);
    station.set_battery(Some(80), Some(true));
    station.rename("Nursery 2".into()).unwrap();
    let s = station.snapshot();
    assert_eq!(s.settings.gain, Gain::Db12);
    assert_eq!(s.battery, Some(80));
    assert_eq!(s.name, "Nursery 2");
    wait_for("battery on tile", || {
        listener.snapshot().stations[0].battery == Some(80)
    })
    .await;

    // Revoke: the listener is told to pair again.
    assert!(station.revoke(listener.device_id()));
    wait_for("needs pairing", || {
        listener.snapshot().stations[0].connection == Connection::NeedsPairing
    })
    .await;
    assert_eq!(listener.known_stations().len(), 1);
    listener.forget(sid.clone()).unwrap();
    assert!(listener.known_stations().is_empty());

    listener.stop().await;
    station.stop().await;
    station.stop().await;
    assert!(lstore.0.lock().unwrap().contains_key("device-key"));
}

#[tokio::test]
async fn errors_cross_the_boundary() {
    let r = Listener::start(
        ListenerOptions {
            name: "x".into(),
            discovery: false,
        },
        Arc::new(BrokenStore),
        Arc::new(ListenerRec::default()),
    );
    assert!(matches!(r, Err(NestwatchError::Storage { .. })));

    let listener = Listener::start(
        ListenerOptions {
            name: "x".into(),
            discovery: false,
        },
        Arc::new(MemStore::default()),
        Arc::new(ListenerRec::default()),
    )
    .unwrap();
    assert!(matches!(
        listener
            .pair_invitation("nonsense".into(), Vec::new())
            .await,
        Err(NestwatchError::Invalid { .. })
    ));
    assert!(matches!(
        listener.follow("aaaaaaaaaaaaaaaa".into()),
        Err(NestwatchError::UnknownStation | NestwatchError::Invalid { .. })
    ));
    assert!(matches!(
        listener.add_discovered(DiscoveredService {
            fullname: "a._nestwatch._tcp.local.".into(),
            name: "a".into(),
            host: String::new(),
            addrs: vec!["192.0.2.1".into()],
            port: 47100,
            txt: vec![TxtEntry {
                key: "v".into(),
                value: "2".into()
            }],
        }),
        Err(NestwatchError::Invalid { .. })
    ));
    listener.stop().await;
}

#[tokio::test]
async fn platform_discovery_feeds_nearby_stations() {
    let srec = Arc::new(StationRec::default());
    let station = start_station(loopback("Nursery"), Arc::new(MemStore::default()), srec)
        .await
        .unwrap();
    let snap = station.snapshot();
    let listener = Listener::start(
        ListenerOptions {
            name: "x".into(),
            discovery: false,
        },
        Arc::new(MemStore::default()),
        Arc::new(ListenerRec::default()),
    )
    .unwrap();
    listener
        .add_discovered(DiscoveredService {
            fullname: "Nursery._nestwatch._tcp.local.".into(),
            name: "Nursery".into(),
            host: "nestwatch.local.".into(),
            addrs: vec!["127.0.0.1".into(), "fe80::1".into()],
            port: snap.tcp_port,
            txt: snap.mdns_txt.clone(),
        })
        .unwrap();
    let nearby = listener.snapshot().nearby;
    assert_eq!(nearby.len(), 1);
    assert_eq!(nearby[0].id, snap.device_id);
    assert_eq!(
        nearby[0].endpoints,
        vec![format!("127.0.0.1:{}", snap.tcp_port)]
    );

    // Code pairing with the discovered address and no explicit route.
    let p = station.open_pairing().await.unwrap();
    let r = listener
        .pair_code(p.code.clone(), snap.device_id.clone(), Vec::new())
        .await
        .unwrap();
    assert_eq!(r.station_id, snap.device_id);
    listener.remove_discovered("Nursery._nestwatch._tcp.local.".into());
    assert!(listener.snapshot().nearby.is_empty());
    listener.stop().await;
    station.stop().await;
}

#[test]
fn helpers() {
    assert_eq!(grouped_id("mfrggzdfmztwq2lk".into()), "mfrg-gzdf-mztw-q2lk");
    assert_eq!(
        parse_device_id("MFRG-GZDF mztw-q2lk".into()).unwrap(),
        "mfrggzdfmztwq2lk"
    );
    assert!(core_version().starts_with("nestwatch-core/"));
}

#[tokio::test]
async fn interfaces_build_invitation_endpoints() {
    let station = start_station(
        StationOptions {
            name: "Nursery".into(),
            port: 47_350,
            mdns: false,
            interfaces: vec![
                InterfaceAddress {
                    name: "wlan0".into(),
                    ip: "192.168.1.20".into(),
                },
                InterfaceAddress {
                    name: "lo".into(),
                    ip: "127.0.0.1".into(),
                },
            ],
            extra_endpoints: vec!["nursery.example.ts.net:47350".into()],
            loopback: false,
        },
        Arc::new(MemStore::default()),
        Arc::new(StationRec::default()),
    )
    .await
    .unwrap();
    let s = station.snapshot();
    assert!(s.tcp_port >= 47_350 && s.tcp_port < 47_360);
    assert_eq!(s.endpoints[0], format!("192.168.1.20:{}", s.tcp_port));
    assert!(!s.endpoints.iter().any(|e| e.starts_with("127.")));
    station.stop().await;
}

/// rendezvous.md FR-RV-1, FR-RV-6, §3.2 through the FFI: off by default,
/// a custom server, online against an in-process server on loopback, the
/// relay in invitations, a kept mailbox and "reset relay identity".
#[tokio::test]
async fn relay_setting_and_status_through_the_ffi() {
    let state = tempfile::tempdir().unwrap();
    let config = nestwatch_server::Config {
        http: "127.0.0.1:0".parse().unwrap(),
        stun: None,
        state_dir: state.path().to_owned(),
        log_interval: 0.0,
        ..nestwatch_server::Config::default()
    };
    let server = nestwatch_server::Server::start(config).await.unwrap();
    let origin = format!("http://{}", server.http_addr());

    let srec = Arc::new(StationRec::default());
    let station = start_station(
        loopback("Nursery"),
        Arc::new(MemStore::default()),
        srec.clone(),
    )
    .await
    .unwrap();
    let s = station.snapshot();
    assert_eq!(s.relay, RelayStatus::Off);
    assert!(!s.settings.relay_enabled);
    assert_eq!(s.settings.relay_server, "https://nestwatch.appmire.be");
    assert_eq!(station.relay_mailbox(), None);
    assert!(matches!(
        station.set_relay_server("not a server".into()),
        Err(NestwatchError::Invalid { .. })
    ));

    station.set_relay_server(format!("{origin}/")).unwrap();
    assert_eq!(station.snapshot().settings.relay_server, origin);
    station.set_relay_enabled(true).unwrap();
    wait_for("relay online", || {
        matches!(station.snapshot().relay, RelayStatus::Online { .. })
    })
    .await;
    let m1 = station.relay_mailbox().expect("mailbox created");
    let p = station.open_pairing().await.unwrap();
    assert!(
        p.invite_uri.contains(&format!("m={m1}")),
        "{}",
        p.invite_uri
    );
    assert!(
        p.relay_web_url
            .unwrap()
            .starts_with(&format!("{origin}/app/#")),
        "browser listener on the relay"
    );

    station.set_relay_enabled(false).unwrap();
    assert_eq!(station.snapshot().relay, RelayStatus::Off);
    assert_eq!(station.relay_mailbox().as_deref(), Some(m1.as_str()));
    station.reset_relay_identity().unwrap();
    assert_ne!(station.relay_mailbox().unwrap(), m1);
    station.stop().await;
    server.shutdown().await;
}
