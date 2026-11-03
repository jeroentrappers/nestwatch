// SPDX-License-Identifier: AGPL-3.0-or-later
//! Audio, sound activation, mixing, talk-back and camera end to end over
//! loopback (product.md §3.4–§3.7, media.md).

mod common;

use std::time::Duration;

use common::*;
use lull_core::ListenMode;
use lull_core::listener::{
    Alert, ListenSettings, ListenerEvent, TalkView, VideoRequest, VideoView,
};
use lull_core::station::{CameraFrame, CameraRequest, PixelFormat, StationEvent};
use lull_core::{CameraFacing, VideoStateKind};

fn frames(l: &lull_core::listener::ListenerEngine, id: &str) -> (u64, u64) {
    let s = l.snapshot();
    let v = s.stations.iter().find(|v| v.id == id).unwrap();
    (v.audio_frames, v.preroll_frames)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_au_3_sound_activation_end_to_end() {
    let (station, _) = station("Nursery").await;
    let (cont, _) = listener("Continuous");
    let (act, _) = listener("Activated");
    let id = pair(&station, &cont).await;
    pair(&station, &act).await;
    cont.set_listen(&id, ListenSettings::continuous()).unwrap();
    // FR-AU-2: threshold, hold and pre-roll per listener.
    act.set_listen(&id, ListenSettings::activated(-30.0, 200, 300))
        .unwrap();
    let mut ss = station.state();
    wait_state(&mut ss, "listen modes applied", |s| {
        s.listeners.len() == 2
            && s.listeners
                .iter()
                .any(|l| l.mode == ListenMode::Activated && !l.open)
            && s.listeners.iter().any(|l| l.mode == ListenMode::Continuous)
    })
    .await;
    let mut aev = act.events();

    // A quiet room: the continuous listener hears it, the activated one
    // gets nothing (FR-AU-3).
    let mut tone = Tone::new(440.0, 48_000);
    feed(&station, &mut tone, -120.0, 1_000).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (c_quiet, _) = frames(&cont, &id);
    assert!(c_quiet >= 30, "continuous listener got {c_quiet} frames");
    assert_eq!(frames(&act, &id), (0, 0));
    assert!(!act.snapshot().stations[0].open);

    // A loud burst arrives with its pre-roll and raises one alert (FR-AL-1).
    feed(&station, &mut tone, -12.0, 400).await;
    let e = wait_event(&mut aev, "sound alert", |e| {
        matches!(e, ListenerEvent::Alert { .. })
    })
    .await;
    assert_eq!(
        e,
        ListenerEvent::Alert {
            station_id: id.clone(),
            alert: Alert::SoundStarted
        }
    );
    feed(&station, &mut tone, -120.0, 600).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (a_frames, a_pre) = frames(&act, &id);
    // 300 ms pre-roll = 15 frames, then the burst and the 200 ms hold.
    assert!(a_pre >= 14, "pre-roll frames: {a_pre}");
    assert!(a_frames >= a_pre + 20, "frames: {a_frames}");
    // The gate closed again after the hold time.
    let mut ls = act.state();
    wait_state(&mut ls, "gate closed", |s| !s.stations[0].open).await;
    let after_first = frames(&act, &id).0;
    feed(&station, &mut tone, -120.0, 300).await;
    assert_eq!(frames(&act, &id).0, after_first, "no audio while quiet");

    // A second burst within 30 s: audio again, but no second alert.
    feed(&station, &mut tone, -12.0, 300).await;
    feed(&station, &mut tone, -120.0, 300).await;
    assert!(frames(&act, &id).0 > after_first);
    let again = wait_event_for(&mut aev, Duration::from_millis(500), "second alert", |e| {
        matches!(
            e,
            ListenerEvent::Alert {
                alert: Alert::SoundStarted,
                ..
            }
        )
    })
    .await;
    assert_eq!(again, None, "sound alerts are rate limited");
    station.shutdown().await;
    cont.shutdown().await;
    act.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 6)]
async fn fr_ls_2_two_stations_mixed_into_one_output() {
    let (a, _) = station("Nursery").await;
    let (b, _) = station("Playroom").await;
    let (l, _) = listener("Parents");
    let ida = pair(&a, &l).await;
    let idb = pair(&b, &l).await;
    l.set_listen(&ida, ListenSettings::continuous()).unwrap();
    l.set_listen(&idb, ListenSettings::continuous()).unwrap();
    assert_eq!(l.snapshot().stations.len(), 2);

    let run = |solo: Option<String>| {
        let (a, b, l) = (a.clone(), b.clone(), l.clone());
        async move {
            l.set_solo(solo.as_deref()).unwrap();
            let feeder = tokio::spawn(async move {
                let (mut ta, mut tb) = (Tone::new(500.0, 48_000), Tone::new(1_250.0, 16_000));
                let mut tick = tokio::time::interval(Duration::from_millis(20));
                for _ in 0..90 {
                    tick.tick().await;
                    a.push_capture(&ta.samples(960, -20.0), 48_000, 1);
                    b.push_capture(&tb.samples(320, -20.0), 16_000, 1);
                }
            });
            let mut out = Vec::new();
            let mut tick = tokio::time::interval(Duration::from_millis(20));
            for _ in 0..90 {
                tick.tick().await;
                let mut buf = [0i16; 320];
                l.pull_playback(&mut buf);
                out.extend_from_slice(&buf);
            }
            feeder.await.unwrap();
            // The last second, after jitter buffers have settled.
            out.split_off(out.len() - 16_000)
        }
    };
    // FR-LS-2: both stations audible in one output.
    let mixed = run(None).await;
    let (pa, pb) = (tone_power(&mixed, 500.0), tone_power(&mixed, 1_250.0));
    let off = tone_power(&mixed, 3_000.0);
    assert!(
        pa > 100.0 * off && pb > 100.0 * off,
        "a {pa} b {pb} off {off}"
    );
    // Solo: only one station is heard; levels keep updating.
    let solo = run(Some(idb.clone())).await;
    let (pa2, pb2) = (tone_power(&solo, 500.0), tone_power(&solo, 1_250.0));
    assert!(pb2 > 100.0 * pa2, "solo a {pa2} b {pb2}");
    assert!(
        l.snapshot()
            .stations
            .iter()
            .all(|s| s.level_dbfs.unwrap_or(-90.0) > -40.0)
    );
    // Mute: muted station silent, alerts unaffected (FR-AU-6).
    l.set_solo(None).unwrap();
    l.set_muted(&ida, true).unwrap();
    let muted = run(None).await;
    let (pa3, pb3) = (tone_power(&muted, 500.0), tone_power(&muted, 1_250.0));
    assert!(pb3 > 100.0 * pa3, "muted a {pa3} b {pb3}");
    a.shutdown().await;
    b.shutdown().await;
    l.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_tb_1_talk_back_arbitration_and_playback() {
    let (station, _) = station("Nursery").await;
    let (mom, _) = listener("Mom");
    let (dad, _) = listener("Dad");
    let id = pair(&station, &mom).await;
    pair(&station, &dad).await;
    let mut sev = station.events();
    let mut mev = mom.events();
    let mut dev = dad.events();

    mom.talk(&id, true).unwrap();
    wait_event(&mut mev, "granted", |e| {
        matches!(
            e,
            ListenerEvent::Talk {
                state: TalkView::Granted,
                ..
            }
        )
    })
    .await;
    wait_event(&mut mev, "microphone wanted", |e| {
        matches!(e, ListenerEvent::MicrophoneWanted { .. })
    })
    .await;
    wait_event(&mut sev, "talk started", |e| {
        matches!(e, StationEvent::TalkStarted { .. })
    })
    .await;

    // Only one talker; the other one learns who talks (FR-TB-1).
    dad.talk(&id, true).unwrap();
    let e = wait_event(&mut dev, "busy", |e| {
        matches!(e, ListenerEvent::Talk { .. })
    })
    .await;
    assert_eq!(
        e,
        ListenerEvent::Talk {
            station_id: id.clone(),
            state: TalkView::Busy {
                by: Some("Mom".into())
            }
        }
    );
    let mut ss = station.state();
    wait_state(&mut ss, "talker shown", |s| {
        s.talker.as_deref() == Some("Mom")
    })
    .await;
    let mut ds = dad.state();
    wait_state(&mut ds, "dad sees talker", |s| {
        s.stations[0].talker.as_deref() == Some("Mom")
    })
    .await;

    // Mom's voice reaches the station speaker; Dad's frames would be dropped.
    let talker = {
        let mom = mom.clone();
        tokio::spawn(async move {
            let mut tone = Tone::new(700.0, 48_000);
            let mut tick = tokio::time::interval(Duration::from_millis(20));
            for _ in 0..60 {
                tick.tick().await;
                mom.push_talk_capture(&tone.samples(960, -15.0), 48_000, 1);
            }
        })
    };
    let mut heard = Vec::new();
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    for _ in 0..60 {
        tick.tick().await;
        let mut buf = [0i16; 320];
        assert!(station.pull_talkback(&mut buf));
        heard.extend_from_slice(&buf);
    }
    talker.await.unwrap();
    let p = tone_power(&heard, 700.0);
    assert!(p > 1e-3, "talk-back power {p}");
    // Volume applies (FR-TB-3).
    station.set_talkback_volume(0.0);
    let mut buf = [1i16; 640];
    station.pull_talkback(&mut buf);
    assert!(buf.iter().all(|&s| s == 0));

    // Release, then the other listener may talk.
    mom.talk(&id, false).unwrap();
    wait_event(&mut sev, "talk ended", |e| {
        matches!(e, StationEvent::TalkEnded)
    })
    .await;
    dad.talk(&id, true).unwrap();
    wait_event(&mut dev, "dad granted", |e| {
        matches!(
            e,
            ListenerEvent::Talk {
                state: TalkView::Granted,
                ..
            }
        )
    })
    .await;
    // Disconnecting releases talk-back.
    dad.shutdown().await;
    wait_event(&mut sev, "released on disconnect", |e| {
        matches!(e, StationEvent::TalkEnded)
    })
    .await;
    assert_eq!(station.snapshot().talker, None);

    // FR-ST-8: talk-back off entirely.
    station.set_talk_enabled(false);
    mom.talk(&id, true).unwrap();
    wait_event(&mut mev, "disabled", |e| {
        matches!(
            e,
            ListenerEvent::Talk {
                state: TalkView::Disabled,
                ..
            }
        )
    })
    .await;
    station.shutdown().await;
    mom.shutdown().await;
}

fn gradient(w: u32, h: u32) -> CameraFrame {
    let mut data = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            data.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 128]);
        }
    }
    CameraFrame {
        width: w,
        height: h,
        format: PixelFormat::Rgb,
        data,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_cam_1_camera_on_demand_with_jpeg_frames() {
    let (station, _) = station("Nursery").await;
    let (l, _) = listener("Parents");
    let id = pair(&station, &l).await;
    let mut sev = station.events();
    let mut frames_rx = l.video_frames();

    l.request_video(
        &id,
        VideoRequest {
            max_w: 320,
            max_h: 320,
            max_fps: 10,
        },
    )
    .unwrap();
    let e = wait_event(&mut sev, "camera wanted", |e| {
        matches!(e, StationEvent::CameraWanted(_))
    })
    .await;
    assert_eq!(
        e,
        StationEvent::CameraWanted(CameraRequest {
            max_w: 320,
            max_h: 320,
            max_fps: 10,
            facing: CameraFacing::Back,
            night: false
        })
    );
    let mut ss = station.state();
    wait_state(&mut ss, "camera indicator", |s| s.camera_on).await;

    let pusher = {
        let station = station.clone();
        tokio::spawn(async move {
            let frame = gradient(640, 480);
            let mut tick = tokio::time::interval(Duration::from_millis(33));
            for _ in 0..45 {
                tick.tick().await;
                station.push_camera_frame(&frame);
            }
        })
    };
    let f = tokio::time::timeout(WAIT, frames_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(f.station_id, id);
    // Aspect ratio kept within 320x320 (media.md §5).
    assert_eq!((f.width, f.height), (320, 240));
    assert_eq!(&f.jpeg[..2], &[0xFF, 0xD8]);
    let img = lull_media::video::decode_to_rgba(&f.jpeg).unwrap();
    assert_eq!((img.width, img.height), (320, 240));
    let mut ls = l.state();
    wait_state(&mut ls, "streaming", |s| {
        s.stations[0].video
            == VideoView::Streaming {
                w: 320,
                h: 240,
                fps: 10,
            }
    })
    .await;
    // Frames are paced to the listener's rate (30 fps offered, 10 fps max).
    let start = std::time::Instant::now();
    let mut n = 0;
    while start.elapsed() < Duration::from_millis(1000) {
        if tokio::time::timeout(Duration::from_millis(200), frames_rx.recv())
            .await
            .is_ok()
        {
            n += 1;
        }
    }
    assert!((5..=12).contains(&n), "{n} frames in 1 s");
    pusher.await.unwrap();

    // The camera stops right after the last viewer leaves (FR-CAM-1).
    l.stop_video(&id).unwrap();
    wait_event(&mut sev, "camera not wanted", |e| {
        matches!(e, StationEvent::CameraNotWanted)
    })
    .await;
    assert!(!station.snapshot().camera_on);

    // FR-ST-8: camera switched off entirely.
    station.set_camera_enabled(false);
    l.request_video(&id, VideoRequest::default()).unwrap();
    wait_state(&mut ls, "disabled", |s| {
        s.stations[0].video == VideoView::State(VideoStateKind::Disabled)
    })
    .await;
    station.shutdown().await;
    l.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_al_3_battery_alert() {
    let (station, _) = station("Nursery").await;
    let (l, _) = listener("Parents");
    let id = pair(&station, &l).await;
    let mut lev = l.events();
    station.set_battery(Some(40), Some(false));
    let mut ls = l.state();
    wait_state(&mut ls, "battery shown", |s| {
        s.stations[0].battery == Some(40)
    })
    .await;
    station.set_battery(Some(12), Some(false));
    let e = wait_event(&mut lev, "battery alert", |e| {
        matches!(e, ListenerEvent::Alert { .. })
    })
    .await;
    assert_eq!(
        e,
        ListenerEvent::Alert {
            station_id: id.clone(),
            alert: Alert::BatteryLow { percent: 12 }
        }
    );
    // Charging: no alert even when low.
    station.set_battery(Some(11), Some(true));
    let none = wait_event_for(&mut lev, Duration::from_millis(500), "no alert", |e| {
        matches!(e, ListenerEvent::Alert { .. })
    })
    .await;
    assert_eq!(none, None);
    station.shutdown().await;
    l.shutdown().await;
}

/// Feeds a tone to `station` and pulls `l`'s playback for `frames` × 20 ms;
/// returns the 500 Hz power of the second half.
async fn heard(
    station: &lull_core::station::StationEngine,
    l: &lull_core::listener::ListenerEngine,
    frames: usize,
) -> f32 {
    let s = station.clone();
    let feeder = tokio::spawn(async move {
        let mut t = Tone::new(500.0, 16_000);
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        for _ in 0..frames {
            tick.tick().await;
            s.push_capture(&t.samples(320, -20.0), 16_000, 1);
        }
    });
    let mut out = Vec::new();
    let mut tick = tokio::time::interval(Duration::from_millis(20));
    for _ in 0..frames {
        tick.tick().await;
        let mut buf = [0i16; 320];
        l.pull_playback(&mut buf);
        out.extend_from_slice(&buf);
    }
    feeder.await.unwrap();
    let half = out.len() / 2;
    tone_power(&out[half..], 500.0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fr_tb_2_own_playback_muted_while_talking() {
    let (station, _) = station("Nursery").await;
    let (l, _) = listener("Parents");
    let id = pair(&station, &l).await;
    l.set_listen(&id, ListenSettings::continuous()).unwrap();
    let before = heard(&station, &l, 60).await;
    assert!(before > 1e-3, "audible before talking: {before}");
    let mut lev = l.events();
    l.talk(&id, true).unwrap();
    wait_event(&mut lev, "granted", |e| {
        matches!(
            e,
            ListenerEvent::Talk {
                state: TalkView::Granted,
                ..
            }
        )
    })
    .await;
    let during = heard(&station, &l, 60).await;
    assert!(
        during < before / 1000.0,
        "muted while talking: {during} vs {before}"
    );
    l.talk(&id, false).unwrap();
    let after = heard(&station, &l, 60).await;
    assert!(after > 1e-3, "audible again: {after}");
    station.shutdown().await;
    l.shutdown().await;
}
