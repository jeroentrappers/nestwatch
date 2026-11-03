// SPDX-License-Identifier: AGPL-3.0-or-later
//! Listener devices on desktop: speaker, talk-back microphone, alert
//! notifications and the alert tone (product.md §3.2, §3.5, §3.6).

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use nestwatch_core::listener::{Alert, ListenerEngine, ListenerEvent};
use nestwatch_platform::{
    AudioStream, CaptureRequest, DesktopPlatform, Notification, PlaybackRequest, PlaybackSource,
    Samples, StreamFormat, Urgency,
};
use nestwatch_ui::strings as s;

/// Samples of the alert tone at 16 kHz (two 200 ms tones).
const TONE_SAMPLES: u32 = 6_720;

/// Adds the alert tone to a 16 kHz frame; `left` counts down.
pub fn mix_tone(frame: &mut [i16], left: &AtomicU32) {
    let mut n = left.load(Ordering::Relaxed);
    if n == 0 {
        return;
    }
    for x in frame.iter_mut() {
        if n == 0 {
            break;
        }
        let i = TONE_SAMPLES - n;
        // 880 Hz then 660 Hz, 20 ms gap, soft attack and release.
        let (freq, pos) = if i < 3_200 {
            (880.0, i)
        } else if i < 3_520 {
            (0.0, 0)
        } else {
            (660.0, i - 3_520)
        };
        if freq > 0.0 {
            let t = pos as f32 / 16_000.0;
            let env = (pos.min(3_199 - pos) as f32 / 160.0).min(1.0);
            let v = (2.0 * std::f32::consts::PI * freq * t).sin() * 0.25 * env * 32767.0;
            *x = (f32::from(*x) + v).clamp(-32768.0, 32767.0) as i16;
        }
        n -= 1;
    }
    left.store(n, Ordering::Relaxed);
}

/// The listener's running devices; dropping it stops them.
pub struct ListenerDevices {
    _speaker: Option<Box<dyn AudioStream>>,
    mic: Arc<Mutex<Option<Box<dyn AudioStream>>>>,
    task: tokio::task::JoinHandle<()>,
}

impl std::fmt::Debug for ListenerDevices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ListenerDevices").finish_non_exhaustive()
    }
}

impl Drop for ListenerDevices {
    fn drop(&mut self) {
        self.task.abort();
        if let Ok(mut m) = self.mic.lock() {
            m.take();
        }
    }
}

/// Starts playback and the event handling of `engine` (call inside the
/// tokio runtime).
pub fn start(platform: &DesktopPlatform, engine: &ListenerEngine) -> ListenerDevices {
    let tone = Arc::new(AtomicU32::new(0));
    let speaker = {
        let e = engine.clone();
        let tone = tone.clone();
        let src = PlaybackSource::Pcm16k(Box::new(move |frame| {
            e.pull_playback(frame);
            mix_tone(frame, &tone);
            true
        }));
        match platform
            .audio_output
            .start(&PlaybackRequest::default(), src)
        {
            Ok(s) => Some(s),
            Err(e) => {
                log::warn!("no speaker ({e}); alerts are shown only");
                None
            }
        }
    };
    let mic = Arc::new(Mutex::new(None));
    let task = tokio::spawn(events(platform.clone(), engine.clone(), mic.clone(), tone));
    ListenerDevices {
        _speaker: speaker,
        mic,
        task,
    }
}

/// The notification for an alert (FR-AL-1..3).
pub fn alert_notification(station: &str, alert: Alert) -> Notification {
    let (title, urgency) = match alert {
        Alert::SoundStarted => (s::alert_sound(station), Urgency::Critical),
        Alert::Unreachable => (s::alert_unreachable(station), Urgency::Critical),
        Alert::Reachable => (s::alert_reachable(station), Urgency::Low),
        Alert::BatteryLow { percent } => (s::alert_battery(station, percent), Urgency::Critical),
    };
    Notification {
        title,
        body: s::ALERT_BODY.to_owned(),
        urgency,
    }
}

async fn events(
    platform: DesktopPlatform,
    engine: ListenerEngine,
    mic: Arc<Mutex<Option<Box<dyn AudioStream>>>>,
    tone: Arc<AtomicU32>,
) {
    let mut ev = engine.events();
    loop {
        let e = match ev.recv().await {
            Ok(e) => e,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(_) => break,
        };
        match e {
            ListenerEvent::MicrophoneWanted { .. } => {
                let e = engine.clone();
                let cb = Box::new(move |f: StreamFormat, s: Samples<'_>| match s {
                    Samples::I16(x) => e.push_talk_capture(x, f.sample_rate, f.channels),
                    Samples::F32(x) => {
                        let v: Vec<i16> = x
                            .iter()
                            .map(|s| (s.clamp(-1.0, 1.0) * 32767.0) as i16)
                            .collect();
                        e.push_talk_capture(&v, f.sample_rate, f.channels);
                    }
                });
                let req = CaptureRequest {
                    device: None,
                    sample_rate: Some(48_000),
                    channels: Some(1),
                };
                match platform.audio_input.start(&req, cb) {
                    Ok(s) => {
                        if let Ok(mut m) = mic.lock() {
                            *m = Some(s);
                        }
                    }
                    Err(e) => log::warn!("no microphone for talk-back ({e})"),
                }
            }
            ListenerEvent::MicrophoneNotWanted => {
                if let Ok(mut m) = mic.lock() {
                    m.take();
                }
            }
            ListenerEvent::Alert { station_id, alert } => {
                let name = engine
                    .snapshot()
                    .stations
                    .iter()
                    .find(|v| v.id == station_id)
                    .map(|v| v.name.clone())
                    .unwrap_or_default();
                if !matches!(alert, Alert::Reachable) {
                    tone.store(TONE_SAMPLES, Ordering::Relaxed);
                }
                if let Err(e) = platform.notifier.notify(&alert_notification(&name, alert)) {
                    log::warn!("notification failed: {e}");
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tone_is_mixed_once_and_counts_down() {
        let left = AtomicU32::new(TONE_SAMPLES);
        let mut total = 0;
        let mut loud = false;
        while left.load(Ordering::Relaxed) > 0 {
            let mut f = [0i16; 320];
            mix_tone(&mut f, &left);
            loud |= f.iter().any(|s| s.abs() > 4_000);
            total += 1;
        }
        assert_eq!(total, TONE_SAMPLES.div_ceil(320));
        assert!(loud);
        let mut f = [7i16; 320];
        mix_tone(&mut f, &left);
        assert!(f.iter().all(|&s| s == 7), "no tone without an alert");
    }

    #[test]
    fn alert_texts_come_from_the_strings_module() {
        let n = alert_notification("Nursery", Alert::SoundStarted);
        assert_eq!(n.title, s::alert_sound("Nursery"));
        assert_eq!(n.urgency, Urgency::Critical);
        assert_eq!(
            alert_notification("Nursery", Alert::Reachable).urgency,
            Urgency::Low
        );
    }
}
