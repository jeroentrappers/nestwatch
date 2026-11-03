// SPDX-License-Identifier: AGPL-3.0-or-later
//! Station snapshot → [`StationModel`] (FR-ST-1..8), native only.

use std::any::Any;
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use lull_core::station::{
    CameraAvailability, StationEngine, StationEvent, StationState, TrustedListener,
};
use lull_core::{Clock, ListenMode, Transport};
use lull_media::capture::{GainSetting, GainStep};

use super::Host;
use crate::backend::{Ui, race};
use crate::model::{
    CameraHere, Gain, ListenerRow, Mode, NoticeKind, PairingModel, StationModel, StationRun,
    StationSettingsModel, TrustedRow,
};

/// Pairings per window (product.md FR-PAIR-3).
pub const MAX_PAIRINGS: u32 = 8;

/// A started station and whatever keeps its devices running (audio and
/// camera streams, sleep inhibition); dropping the guard stops them.
pub struct RunningStation {
    /// The engine.
    pub engine: StationEngine,
    /// Device handles.
    pub guard: Box<dyn Any>,
}

impl std::fmt::Debug for RunningStation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunningStation")
            .field("engine", &self.engine)
            .finish_non_exhaustive()
    }
}

/// Engine gain setting for a UI choice.
pub fn gain_setting(g: Gain) -> GainSetting {
    match g {
        Gain::Auto => GainSetting::Auto,
        Gain::Db(0) => GainSetting::Manual(GainStep::Db0),
        Gain::Db(6) => GainSetting::Manual(GainStep::Db6),
        Gain::Db(12) => GainSetting::Manual(GainStep::Db12),
        Gain::Db(_) => GainSetting::Manual(GainStep::Db18),
    }
}

/// Maps a station snapshot at monotonic time `now_ms` (on the engine's
/// clock) with the current trust list.
pub fn map_station(s: &StationState, trusted: &[TrustedListener], now_ms: u64) -> StationModel {
    let http_url = match (s.endpoints.first(), s.http_port) {
        (Some(e), Some(p)) => Some(format!("http://{}:{p}/", e.host)),
        _ => None,
    };
    let https_url = match (s.endpoints.first(), s.https_port) {
        (Some(e), Some(p)) => Some(format!("https://{}:{p}/", e.host)),
        _ => None,
    };
    StationModel {
        name: s.name.clone(),
        device_id: s.device_id.clone(),
        level_dbfs: s.level_dbfs,
        listeners: s
            .listeners
            .iter()
            .map(|l| ListenerRow {
                session: l.session,
                id: l.id.clone(),
                name: l.name.clone(),
                mode: match l.mode {
                    ListenMode::Continuous => Mode::Continuous,
                    ListenMode::Activated => Mode::Activated,
                },
                threshold_dbfs: l.threshold_dbfs,
                open: l.open,
                talking: l.talking,
                watching: l.watching,
                web: l.transport == Transport::WebSocket,
            })
            .collect(),
        camera_on: s.camera_on,
        talker: s.talker.clone(),
        pairing: s.pairing.as_ref().map(|p| PairingModel {
            invite_uri: p.invite_uri.clone(),
            web_url: p.web_url.clone(),
            code: p.code.clone(),
            remaining_ms: p.closes_at_ms.saturating_sub(now_ms),
            pairings: p.pairings,
            max_pairings: MAX_PAIRINGS,
        }),
        trusted: trusted
            .iter()
            .map(|t| TrustedRow {
                id: t.id.clone(),
                name: t.name.clone(),
                paired_unix_ms: t.paired_unix_ms,
                last_seen_unix_ms: t.last_seen_unix_ms,
            })
            .collect(),
        settings: StationSettingsModel {
            camera_enabled: s.settings.camera_enabled,
            talk_enabled: s.settings.talk_enabled,
            talkback_volume: s.settings.talkback_volume,
            gain: match s.settings.gain_db {
                0 | 6 | 12 | 18 => Gain::Db(s.settings.gain_db),
                _ => Gain::Auto,
            },
        },
        fingerprint: s.https_fingerprint.clone(),
        http_url,
        https_url,
        battery: s.battery,
        charging: s.charging,
        camera_here: match s.camera_availability {
            CameraAvailability::Available => CameraHere::Available,
            CameraAvailability::Unavailable => CameraHere::Unavailable,
            CameraAvailability::Denied => CameraHere::Denied,
        },
    }
}

/// Feeds `ui.station` while `gen_` keeps the value it had at the start.
pub(super) fn spawn_station_pumps(
    engine: StationEngine,
    clock: Arc<dyn Clock>,
    host: Rc<dyn Host>,
    mut ui: Ui,
    gen_: Rc<Cell<u64>>,
) {
    let mine = gen_.get();
    {
        let engine = engine.clone();
        let host = host.clone();
        let gen_ = gen_.clone();
        spawn_forever(async move {
            let mut rx = engine.state();
            loop {
                if gen_.get() != mine {
                    break;
                }
                let state = rx.borrow_and_update().clone();
                let model = map_station(&state, &engine.trusted_listeners(), clock.now_ms());
                let run = StationRun::Running(Box::new(model));
                if *ui.station.peek() != run {
                    ui.station.set(run);
                }
                // Tick every second for the pairing countdown.
                if let Some(Err(_)) = race(rx.changed(), host.sleep(1000)).await {
                    break;
                }
            }
        });
    }
    spawn_forever(async move {
        let mut ev = engine.events();
        loop {
            if gen_.get() != mine {
                break;
            }
            match race(ev.recv(), host.sleep(1000)).await {
                Some(Ok(StationEvent::Paired {
                    name, verification, ..
                })) => {
                    ui.just_paired
                        .write()
                        .push((name.clone(), verification.clone()));
                    ui.notify(NoticeKind::ListenerPaired { name, verification });
                }
                Some(Ok(StationEvent::PairingOpened)) => ui.just_paired.write().clear(),
                Some(Ok(_)) | None => {}
                Some(Err(tokio::sync::broadcast::error::RecvError::Lagged(_))) => {}
                Some(Err(_)) => break,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use lull_core::station::StationConfig;
    use lull_core::{ManualClock, MemoryStorage};

    #[tokio::test]
    async fn maps_a_running_station_with_pairing() {
        let clock = Arc::new(ManualClock::new(1_000, 1_700_000_000_000));
        let station = StationEngine::start(
            StationConfig::loopback("Nursery"),
            Arc::new(MemoryStorage::new()),
            clock.clone(),
        )
        .await
        .unwrap();
        let info = station.open_pairing().await.unwrap();
        clock.advance(48_000);
        let m = map_station(
            &station.snapshot(),
            &station.trusted_listeners(),
            clock.now_ms(),
        );
        assert_eq!(m.name, "Nursery");
        let p = m.pairing.unwrap();
        assert_eq!(p.code, info.code);
        assert_eq!(p.remaining_ms, 600_000 - 48_000);
        assert_eq!(p.max_pairings, MAX_PAIRINGS);
        assert_eq!(m.settings.gain, Gain::Auto);
        assert!(m.listeners.is_empty() && m.trusted.is_empty());
        assert_eq!(m.http_url, None, "no web server in loopback config");
        station.shutdown().await;
    }

    #[test]
    fn gain_choices() {
        assert_eq!(gain_setting(Gain::Auto), GainSetting::Auto);
        assert_eq!(
            gain_setting(Gain::Db(12)),
            GainSetting::Manual(GainStep::Db12)
        );
    }
}
