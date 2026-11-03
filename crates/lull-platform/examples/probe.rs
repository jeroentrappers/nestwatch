// SPDX-License-Identifier: AGPL-3.0-or-later
//! Hardware probe: exercises every platform backend once.
//!
//! ```text
//! cargo run -p lull-platform --example probe -- [--audio NAME] [--camera NAME]
//!     [--key-store NAME] [--skip-camera] [--skip-notify] [--skip-tone]
//! ```
//!
//! Audio is measured and discarded; the camera delivers one frame whose size
//! is printed; nothing is written to disk except a test secret that is
//! deleted again.

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use lull_media::capture::{CaptureChain, GainSetting, GainStep};
use lull_media::level::level_dbfs;
use lull_platform::{
    CameraRequest, CaptureRequest, DesktopOptions, DesktopPlatform, Notification, PixelData,
    PlaybackRequest, PlaybackSource, Urgency,
};

const TEST_SECRET: &str = "probe-test";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |flag: &str| {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let options = DesktopOptions {
        audio: value("--audio"),
        camera: value("--camera"),
        key_store: value("--key-store"),
        key_dir: None,
    };

    let t = Instant::now();
    let platform = DesktopPlatform::detect(&options);
    println!("== backends (detected in {} ms)", t.elapsed().as_millis());
    print!("{}", platform.report);

    devices(&platform);
    record(&platform);
    if !has("--skip-tone") {
        tone(&platform);
    }
    if !has("--skip-camera") {
        camera(&platform);
    }
    if !has("--skip-notify") {
        notify(&platform);
    }
    battery(&platform);
    power(&platform);
    secrets(&platform);
}

fn devices(p: &DesktopPlatform) {
    println!("== audio inputs ({})", p.audio_input.backend());
    match p.audio_input.devices() {
        Ok(d) => d.iter().for_each(|d| {
            println!(
                "  {}{} [{}]",
                if d.is_default { "* " } else { "  " },
                d.name,
                d.id
            )
        }),
        Err(e) => println!("  error: {e}"),
    }
    println!("== audio outputs ({})", p.audio_output.backend());
    match p.audio_output.devices() {
        Ok(d) => d.iter().for_each(|d| {
            println!(
                "  {}{} [{}]",
                if d.is_default { "* " } else { "  " },
                d.name,
                d.id
            )
        }),
        Err(e) => println!("  error: {e}"),
    }
    println!("== cameras ({})", p.camera.backend());
    match p.camera.cameras() {
        Ok(c) if c.is_empty() => println!("  none"),
        Ok(c) => c
            .iter()
            .for_each(|c| println!("  {} [{}] facing {:?}", c.name, c.id, c.facing)),
        Err(e) => println!("  error: {e}"),
    }
}

fn record(p: &DesktopPlatform) {
    println!("== record 2 s from the default input");
    #[derive(Default)]
    struct Meter {
        chain: Option<CaptureChain>,
        levels: Vec<f32>,
        pcm: Vec<i16>,
        format: Option<lull_platform::StreamFormat>,
    }
    let meter = Arc::new(Mutex::new(Meter::default()));
    let m = Arc::clone(&meter);
    let started = p.audio_input.start(
        &CaptureRequest::default(),
        Box::new(move |format, samples| {
            let mut m = m.lock().unwrap();
            if m.chain.is_none() {
                m.format = Some(format);
                m.chain = Some(
                    CaptureChain::new(
                        format.sample_rate,
                        format.channels,
                        GainSetting::Manual(GainStep::Db0),
                    )
                    .unwrap(),
                );
            }
            let Meter {
                chain, levels, pcm, ..
            } = &mut *m;
            let chain = chain.as_mut().unwrap();
            samples.push_into(chain);
            while let Some(frame) = chain.pop_frame() {
                levels.push(frame.level_dbfs);
                pcm.extend_from_slice(&frame.pcm);
            }
        }),
    );
    let mut stream = match started {
        Ok(s) => s,
        Err(e) => {
            println!("  error: {e}");
            return;
        }
    };
    std::thread::sleep(Duration::from_secs(2));
    stream.stop();
    let stats = stream.stats();
    let m = meter.lock().unwrap();
    if let Some(f) = m.format {
        println!(
            "  device format: {} Hz, {} ch, {:?}",
            f.sample_rate, f.channels, f.sample_type
        );
    }
    let peak = m.levels.iter().copied().fold(-90.0f32, f32::max);
    println!(
        "  {} frames of 20 ms; level {:.1} dBFS overall, loudest frame {:.1} dBFS",
        m.levels.len(),
        level_dbfs(&m.pcm),
        peak
    );
    println!(
        "  overruns {}, error {:?}",
        stats.overruns,
        stats.error.as_deref().unwrap_or("none")
    );
}

fn tone(p: &DesktopPlatform) {
    println!("== play a 1 s 440 Hz tone at -18 dBFS");
    let mut phase = 0f64;
    let amp = 32768.0 * 10f64.powf(-18.0 / 20.0);
    let mut frames_left = 50u32;
    let (done_tx, done_rx) = mpsc::channel();
    let source = PlaybackSource::Pcm16k(Box::new(move |frame| {
        if frames_left == 0 {
            let _ = done_tx.send(());
            return false;
        }
        frames_left -= 1;
        for s in frame.iter_mut() {
            *s = (phase.sin() * amp) as i16;
            phase += 2.0 * std::f64::consts::PI * 440.0 / 16_000.0;
        }
        true
    }));
    let t = Instant::now();
    let mut stream = match p.audio_output.start(&PlaybackRequest::default(), source) {
        Ok(s) => s,
        Err(e) => {
            println!("  error: {e}");
            return;
        }
    };
    let _ = done_rx.recv_timeout(Duration::from_secs(3));
    // Let the last buffered audio drain.
    std::thread::sleep(Duration::from_millis(150));
    let wall = t.elapsed();
    let stats = stream.stats();
    stream.stop();
    let f = stream.format();
    println!(
        "  stream {} Hz {} ch; source drained after {} ms (1000 ms of audio)",
        f.sample_rate,
        f.channels,
        wall.as_millis().saturating_sub(150)
    );
    println!(
        "  underruns {}, error {:?}",
        stats.underruns,
        stats.error.as_deref().unwrap_or("none")
    );
}

fn camera(p: &DesktopPlatform) {
    println!("== capture one camera frame ({})", p.camera.backend());
    let (tx, rx) = mpsc::sync_channel(1);
    let t = Instant::now();
    let started = p.camera.start(
        &CameraRequest::default(),
        Box::new(move |frame| {
            let _ = tx.try_send(frame);
        }),
    );
    let mut stream = match started {
        Ok(s) => s,
        Err(e) => {
            println!("  error: {e} (video-state {})", e.video_state());
            return;
        }
    };
    let frame = rx.recv_timeout(Duration::from_secs(10));
    stream.stop();
    match frame {
        Ok(Ok(f)) => {
            let kind = match f.data {
                PixelData::Rgb(_) => "RGB",
                PixelData::Rgba(_) => "RGBA",
                PixelData::I420 { .. } => "I420",
            };
            let mode = stream.mode();
            println!(
                "  frame {}x{} {kind} after {} ms (camera mode {}x{} @ {} fps); not stored",
                f.width,
                f.height,
                t.elapsed().as_millis(),
                mode.width,
                mode.height,
                mode.fps
            );
        }
        Ok(Err(e)) => println!("  error: {e} (video-state {})", e.video_state()),
        Err(_) => println!("  no frame within 10 s"),
    }
}

fn notify(p: &DesktopPlatform) {
    println!("== notification ({})", p.notifier.backend());
    let n = Notification {
        title: "Lull probe".into(),
        body: "Platform probe test notification.".into(),
        urgency: Urgency::Normal,
    };
    match p.notifier.notify(&n) {
        Ok(()) => println!("  shown"),
        Err(e) => println!("  error: {e}"),
    }
}

fn battery(p: &DesktopPlatform) {
    println!("== battery ({})", p.battery.backend());
    match p.battery.status() {
        Ok(Some(b)) => println!("  {} %, charging {}", b.percent, b.charging),
        Ok(None) => println!("  no battery"),
        Err(e) => println!("  error: {e}"),
    }
}

fn power(p: &DesktopPlatform) {
    println!("== sleep inhibition ({})", p.power.backend());
    match p.power.inhibit_sleep("Lull platform probe") {
        Ok(i) => {
            println!("  held by {}", i.backend());
            drop(i);
            println!("  released");
        }
        Err(e) => println!("  error: {e}"),
    }
}

fn secrets(p: &DesktopPlatform) {
    println!("== key store ({})", p.key_store.backend());
    let mut value = [0u8; 32];
    for (i, b) in value.iter_mut().enumerate() {
        *b = (i as u8).wrapping_mul(37) ^ 0x5A;
    }
    let result = (|| -> Result<bool, lull_platform::PlatformError> {
        p.key_store.put(TEST_SECRET, &value)?;
        let back = p.key_store.get(TEST_SECRET)?;
        let same = back.as_ref().map(|s| s.expose()) == Some(&value[..]);
        let deleted = p.key_store.delete(TEST_SECRET)?;
        let gone = p.key_store.get(TEST_SECRET)?.is_none();
        Ok(same && deleted && gone)
    })();
    match result {
        Ok(true) => println!("  round trip of a 32-byte test secret ok; deleted"),
        Ok(false) => println!("  round trip mismatch"),
        Err(e) => {
            let _ = p.key_store.delete(TEST_SECRET);
            println!("  error: {e}");
        }
    }
}
