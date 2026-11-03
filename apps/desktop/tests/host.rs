// SPDX-License-Identifier: AGPL-3.0-or-later
//! The desktop station host without devices: the engine starts, serves the
//! browser app, and a camera request without a camera reports
//! "unavailable" to the listener (FR-CAM-1, protocol.md §7.7).

use std::sync::Arc;
use std::time::Duration;

use lull_core::listener::{ListenerConfig, ListenerEngine, VideoRequest, VideoView};
use lull_core::{Clock, MemoryStorage, SystemClock, VideoStateKind};
use lull_desktop::station_host::{self, StationOptions};
use lull_platform::{DesktopOptions, DesktopPlatform};

#[tokio::test(flavor = "multi_thread")]
async fn station_host_without_devices() {
    // Force "no backend" for audio and camera: never touch real devices.
    let platform = DesktopPlatform::detect(&DesktopOptions {
        audio: Some("off".into()),
        camera: Some("off".into()),
        ..Default::default()
    });
    assert_eq!(platform.report.chosen("audio"), Some("none"));
    assert_eq!(platform.report.chosen("camera"), Some("none"));
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let running = station_host::start(
        platform,
        Arc::new(MemoryStorage::new()),
        clock.clone(),
        StationOptions {
            name: Some("Desk".into()),
            tcp_port: Some(0),
            http_port: Some(0),
            https_port: Some(0),
            loopback: true,
            no_mdns: true,
        },
    )
    .await
    .unwrap();
    let station = running.engine.clone();
    let snap = station.snapshot();
    assert_eq!(snap.name, "Desk");
    let http = snap.http_port.expect("web server runs");
    assert!(snap.https_fingerprint.is_some());

    // The browser app (or the placeholder) is served.
    let body = http_get(http).await;
    assert!(body.starts_with("HTTP/1.1 200"), "{body}");
    assert!(body.contains("<title>Lull</title>"));

    let info = station.open_pairing().await.unwrap();
    let mut cfg = ListenerConfig::new("Laptop");
    cfg.discovery = false;
    let listener = ListenerEngine::start(cfg, Arc::new(MemoryStorage::new()), clock).unwrap();
    let paired = listener.pair_invitation(&info.invite_uri).await.unwrap();
    for _ in 0..100 {
        if listener
            .snapshot()
            .stations
            .first()
            .is_some_and(|s| s.caps.iter().any(|c| c == "video"))
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    listener
        .request_video(&paired.station_id, VideoRequest::default())
        .unwrap();
    let mut state = None;
    for _ in 0..100 {
        state = listener
            .snapshot()
            .stations
            .first()
            .map(|s| s.video.clone());
        if state == Some(VideoView::State(VideoStateKind::Unavailable)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(state, Some(VideoView::State(VideoStateKind::Unavailable)));
    assert_eq!(
        station.snapshot().camera_availability,
        lull_core::station::CameraAvailability::Unavailable
    );
    listener.shutdown().await;
    station.shutdown().await;
    drop(running);
}

async fn http_get(port: u16) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    s.write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut buf = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(3), s.read_to_end(&mut buf)).await;
    String::from_utf8_lossy(&buf).into_owned()
}
