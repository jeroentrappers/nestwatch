// SPDX-License-Identifier: AGPL-3.0-or-later
//! Headless station on this computer's real devices: microphone, talk-back
//! speaker, on-demand camera, sleep inhibition and battery, without the
//! desktop window. Settings come from the same environment variables as the
//! desktop app (`NESTWATCH_STATION_NAME`, `NESTWATCH_TCP_PORT`,
//! `NESTWATCH_HTTP_PORT`, `NESTWATCH_HTTPS_PORT`, `NESTWATCH_LOOPBACK=1`,
//! `NESTWATCH_NO_MDNS=1`, `NESTWATCH_DATA_DIR`, `NESTWATCH_WEB_DIR`).
//!
//! `--pair` opens pairing mode at start and prints the invitation, the web
//! address and the pairing code.

use std::sync::Arc;

use nestwatch_core::SystemClock;
use nestwatch_core::station::StationEvent;
use nestwatch_desktop::{engine_storage, station_host};
use nestwatch_platform::{DesktopOptions, DesktopPlatform};

#[tokio::main]
async fn main() {
    let pair = std::env::args().any(|a| a == "--pair");
    let platform = DesktopPlatform::detect(&DesktopOptions::default());
    eprintln!("devices: {}", platform.report);
    let storage = engine_storage(&platform);
    let running = match station_host::start(
        platform,
        storage,
        Arc::new(SystemClock::new()),
        station_host::StationOptions::from_env(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("could not start the station: {e}");
            std::process::exit(1);
        }
    };
    let engine = running.engine.clone();
    let state = engine.state().borrow().clone();
    println!("station {} id {}", state.name, state.device_id);
    if pair {
        match engine.open_pairing().await {
            Ok(info) => {
                println!("invite: {}", info.invite_uri);
                if let Some(web) = &info.web_url {
                    println!("web: {web}");
                }
                println!("code: {}", info.code);
            }
            Err(e) => eprintln!("could not open pairing: {e}"),
        }
    }
    let mut events = engine.events();
    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            event = events.recv() => match event {
                Ok(StationEvent::Paired { verification, .. }) => println!("paired, verification {verification}"),
                Ok(StationEvent::CameraWanted(r)) => println!("camera on ({}x{} at {} fps)", r.max_w, r.max_h, r.max_fps),
                Ok(StationEvent::CameraNotWanted) => println!("camera off"),
                Ok(StationEvent::ListenerConnected { .. }) => println!("listener connected"),
                Ok(StationEvent::ListenerDisconnected { .. }) => println!("listener disconnected"),
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            },
        }
    }
    engine.shutdown().await;
    drop(running);
}
