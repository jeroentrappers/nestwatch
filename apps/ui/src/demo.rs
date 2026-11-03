// SPDX-License-Identifier: AGPL-3.0-or-later
//! A backend with made-up stations for design previews and UI checks. It
//! never touches the network or devices.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use base64::Engine as _;
use dioxus::core::spawn_forever;
use dioxus::prelude::*;

use crate::backend::{Backend, LocalFuture, Ui};
use crate::model::{
    CameraHere, Capabilities, Conn, Frame, Gain, LevelHistory, ListenerModel, ListenerRow, Mode,
    Nearby, NetPath, PairOutcome, PairingModel, Platform, Prefs, RelayView, StationModel,
    StationRun, StationSettingsModel, Talk, Theme, Tile, TileSettings, TrustedRow, UiError, Video,
};

/// Timer of the hosting platform.
pub type Sleeper = Rc<dyn Fn(u64) -> LocalFuture<()>>;

/// Demo data backend.
pub struct DemoBackend {
    platform: Platform,
    sleep: Sleeper,
    theme: Rc<dyn Fn(Theme)>,
    prefs: RefCell<Prefs>,
    ui: Cell<Option<Ui>>,
}

impl std::fmt::Debug for DemoBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DemoBackend").finish_non_exhaustive()
    }
}

const NURSERY: &str = "nurserynurseryaa";
const LEO: &str = "leosroomleosrooo";
const GRANDMA: &str = "grandmagrandmaaa";

impl DemoBackend {
    /// A demo backend. `apply_theme` sets the root's `data-theme`.
    pub fn new(
        platform: Platform,
        prefs: Prefs,
        sleep: Sleeper,
        apply_theme: Rc<dyn Fn(Theme)>,
    ) -> Self {
        Self {
            platform,
            sleep,
            theme: apply_theme,
            prefs: RefCell::new(prefs),
            ui: Cell::new(None),
        }
    }

    fn edit(&self, id: &str, f: impl FnOnce(&mut Tile)) -> Result<(), UiError> {
        let Some(mut ui) = self.ui.get() else {
            return Err(UiError::Unsupported);
        };
        let mut l = ui.listener.write();
        let t = l
            .tiles
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or(UiError::Other("unknown".into()))?;
        f(t);
        t.alerting = crate::logic::has_sound(t);
        Ok(())
    }
}

fn tile(id: &str, name: &str) -> Tile {
    Tile {
        id: id.into(),
        name: name.into(),
        conn: Conn::Live,
        path: Some(NetPath::Lan),
        route: Some("192.168.1.20:47100".into()),
        level_dbfs: Some(-58.0),
        open: false,
        battery: None,
        charging: None,
        station_listeners: Some(2),
        camera_on: false,
        talker: None,
        talk: Talk::Idle,
        video: Video::Off,
        can_talk: true,
        can_video: true,
        settings: TileSettings::default(),
        down_for_ms: None,
        routes: 3,
        alerting: false,
    }
}

/// The demo listener: three stations as in the design mockups.
pub fn demo_listener() -> ListenerModel {
    let mut nursery = tile(NURSERY, "Nursery");
    nursery.path = Some(NetPath::Vpn);
    nursery.level_dbfs = Some(-18.0);
    nursery.open = true;
    nursery.battery = Some(82);
    nursery.charging = Some(false);
    nursery.camera_on = true;
    nursery.video = Video::Streaming {
        w: 640,
        h: 480,
        fps: 10,
    };
    nursery.alerting = true;
    let mut leo = tile(LEO, "Leo's room");
    leo.charging = Some(true);
    leo.path = Some(NetPath::Relayed);
    let mut grandma = tile(GRANDMA, "Grandma's");
    grandma.conn = Conn::Reconnecting {
        attempt: 3,
        tried: 3,
    };
    grandma.path = None;
    grandma.level_dbfs = None;
    grandma.down_for_ms = Some(42_000);
    ListenerModel {
        name: "Living room laptop".into(),
        device_id: "k3m9q2x7a4b8c5d6".into(),
        tiles: vec![nursery, leo, grandma],
        solo: None,
        nearby: vec![
            Nearby {
                id: "nurserynewnewaaa".into(),
                name: "Nursery".into(),
                pairing: true,
                known: false,
                endpoints: vec!["192.168.1.20:47100".into()],
            },
            Nearby {
                id: LEO.into(),
                name: "Leo's room".into(),
                pairing: false,
                known: true,
                endpoints: Vec::new(),
            },
        ],
    }
}

/// The demo station.
pub fn demo_station() -> StationModel {
    StationModel {
        name: "Nursery".into(),
        device_id: "nurserynurseryaa".into(),
        level_dbfs: -56.0,
        listeners: vec![
            ListenerRow {
                session: 1,
                id: "mum".into(),
                name: "Mum's phone".into(),
                mode: Mode::Activated,
                threshold_dbfs: -50.0,
                open: false,
                talking: false,
                watching: true,
                web: false,
                relayed: true,
            },
            ListenerRow {
                session: 2,
                id: "laptop".into(),
                name: "Living room laptop".into(),
                mode: Mode::Continuous,
                threshold_dbfs: -50.0,
                open: true,
                talking: false,
                watching: false,
                web: true,
                relayed: false,
            },
        ],
        camera_on: true,
        talker: None,
        pairing: None,
        trusted: vec![
            TrustedRow {
                id: "mum".into(),
                name: "Mum's phone".into(),
                paired_unix_ms: 1_700_000_000_000,
                last_seen_unix_ms: 1_700_000_000_000,
            },
            TrustedRow {
                id: "laptop".into(),
                name: "Living room laptop".into(),
                paired_unix_ms: 1_700_000_000_000,
                last_seen_unix_ms: 1_700_000_000_000,
            },
        ],
        settings: StationSettingsModel {
            camera_enabled: true,
            talk_enabled: true,
            talkback_volume: 0.8,
            gain: Gain::Auto,
            relay_enabled: true,
            relay_server: "https://nestwatch.appmire.be".into(),
        },
        fingerprint: Some("3A:9F:12:C4:7E:08:B1:55:2D:6A:90:EE:41:7C:D3:28:5B:0F:A6:19:84:C2:7D:E3:11:4B:9A:60:F5:2E:8C:D7".into()),
        http_url: Some("http://192.168.1.20:47180/".into()),
        https_url: Some("https://192.168.1.20:47181/".into()),
        battery: Some(82),
        charging: Some(false),
        camera_here: CameraHere::Available,
        relay: RelayView::Online,
    }
}

/// A placeholder "camera" picture (an SVG crib), as a frame.
pub fn demo_frame() -> Frame {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 640 480"><defs><radialGradient id="g" cx="60%" cy="40%" r="70%"><stop offset="0" stop-color="#3b3446"/><stop offset="1" stop-color="#121019"/></radialGradient></defs><rect width="640" height="480" fill="url(#g)"/><rect x="115" y="106" width="410" height="288" rx="18" fill="none" stroke="#ffffff" stroke-opacity=".18" stroke-width="6"/><ellipse cx="320" cy="250" rx="74" ry="44" fill="#ffffff" fill-opacity=".12"/></svg>"##;
    Frame {
        frame_id: 1,
        src: format!(
            "data:image/svg+xml;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(svg)
        ),
        width: 640,
        height: 480,
    }
}

fn demo_level(t: u64, base: f32, spike_at: u64) -> f32 {
    // A quiet room with a little noise and one cry every minute.
    let noise = ((t as f32 / 900.0).sin() * 2.5) + ((t as f32 / 370.0).cos() * 1.5);
    let phase = (t + 60_000 - spike_at) % 60_000;
    let cry = if (0..6_000).contains(&phase) {
        let x = phase as f32 / 6_000.0;
        40.0 * (x * std::f32::consts::PI).sin()
    } else {
        0.0
    };
    (base + noise + cry).min(-6.0)
}

impl Backend for DemoBackend {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            platform: self.platform,
            station: self.platform == Platform::Desktop,
            discovery: self.platform == Platform::Desktop,
            talk: true,
        }
    }

    fn attach(&self, mut ui: Ui) {
        self.ui.set(Some(ui));
        ui.listener.set(demo_listener());
        ui.frames.write().insert(NURSERY.into(), demo_frame());
        // One minute of history so the chart is not empty.
        {
            let mut h = ui.history.write();
            for (id, base, spike) in [(NURSERY, -60.0, 48_000), (LEO, -62.0, 20_000)] {
                let mut hist = LevelHistory::default();
                for i in 0..240u64 {
                    let t = i * 250;
                    hist.push(t, demo_level(t, base, spike));
                }
                h.insert(id.into(), hist);
            }
        }
        let sleep = self.sleep.clone();
        spawn_forever(async move {
            let mut t = 60_000u64;
            loop {
                sleep(250).await;
                t += 250;
                let n = demo_level(t, -60.0, 48_000);
                let l = demo_level(t, -62.0, 20_000);
                {
                    let mut h = ui.history.write();
                    h.entry(NURSERY.into()).or_default().push(t, n);
                    h.entry(LEO.into()).or_default().push(t, l);
                }
                let mut lm = ui.listener.write();
                for tile in lm.tiles.iter_mut() {
                    let v = match tile.id.as_str() {
                        NURSERY => n,
                        LEO => l,
                        _ => continue,
                    };
                    tile.level_dbfs = Some(v);
                    tile.open = tile.settings.mode == Mode::Activated
                        && v >= tile.settings.threshold_dbfs
                        || tile.id == NURSERY && tile.open && v > -45.0;
                    tile.alerting = crate::logic::has_sound(tile);
                }
                drop(lm);
                if let StationRun::Running(m) = &mut *ui.station.write() {
                    m.level_dbfs = demo_level(t, -60.0, 48_000);
                    if let Some(p) = &mut m.pairing {
                        p.remaining_ms = p.remaining_ms.saturating_sub(250);
                    }
                }
            }
        });
    }

    fn load_prefs(&self) -> Prefs {
        self.prefs.borrow().clone()
    }

    fn save_prefs(&self, prefs: &Prefs) {
        *self.prefs.borrow_mut() = prefs.clone();
    }

    fn apply_theme(&self, theme: Theme) {
        (self.theme)(theme);
    }

    fn sleep(&self, ms: u64) -> LocalFuture<()> {
        (self.sleep)(ms)
    }

    fn unix_ms(&self) -> u64 {
        1_700_000_600_000
    }

    fn pair_invitation(&self, _input: String) -> LocalFuture<Result<PairOutcome, UiError>> {
        let sleep = (self.sleep)(800);
        Box::pin(async move {
            sleep.await;
            Ok(PairOutcome {
                station_id: "nurserynewnewaaa".into(),
                name: "Nursery".into(),
                verification: "042 917".into(),
            })
        })
    }

    fn pair_code(
        &self,
        code: String,
        _station_id: String,
        _address: Option<String>,
    ) -> LocalFuture<Result<PairOutcome, UiError>> {
        let f = self.pair_invitation(code);
        Box::pin(f)
    }

    fn forget(&self, station: &str) -> Result<(), UiError> {
        if let Some(mut ui) = self.ui.get() {
            ui.listener.write().tiles.retain(|t| t.id != station);
        }
        Ok(())
    }

    fn set_follow(&self, station: &str, follow: bool) -> Result<(), UiError> {
        self.edit(station, |t| {
            t.conn = if follow { Conn::Live } else { Conn::Paused };
        })
    }

    fn add_address(&self, _station: &str, address: &str) -> Result<(), UiError> {
        if address.contains(':') {
            Ok(())
        } else {
            Err(UiError::InvalidAddress)
        }
    }

    fn set_listen(
        &self,
        station: &str,
        mode: Mode,
        threshold_dbfs: f32,
        hold_ms: u32,
    ) -> Result<(), UiError> {
        self.edit(station, |t| {
            t.settings.mode = mode;
            t.settings.threshold_dbfs = threshold_dbfs;
            t.settings.hold_ms = hold_ms;
        })
    }

    fn set_volume(&self, station: &str, volume: f32) -> Result<(), UiError> {
        self.edit(station, |t| t.settings.volume = volume)
    }

    fn set_muted(&self, station: &str, muted: bool) -> Result<(), UiError> {
        self.edit(station, |t| t.settings.muted = muted)
    }

    fn set_alerts(
        &self,
        station: &str,
        sound: bool,
        unreachable: bool,
        battery: bool,
    ) -> Result<(), UiError> {
        self.edit(station, |t| {
            t.settings.alert_sound = sound;
            t.settings.alert_unreachable = unreachable;
            t.settings.alert_battery = battery;
        })
    }

    fn set_solo(&self, station: Option<&str>) -> Result<(), UiError> {
        if let Some(mut ui) = self.ui.get() {
            ui.listener.write().solo = station.map(str::to_owned);
        }
        Ok(())
    }

    fn talk(&self, station: &str, on: bool) -> Result<(), UiError> {
        self.edit(station, |t| {
            t.talk = if on { Talk::Granted } else { Talk::Idle }
        })
    }

    fn set_camera(&self, station: &str, on: bool) -> Result<(), UiError> {
        if let Some(mut ui) = self.ui.get() {
            if on {
                ui.frames.write().insert(station.into(), demo_frame());
            } else {
                ui.frames.write().remove(station);
            }
        }
        self.edit(station, |t| {
            t.video = if on {
                Video::Streaming {
                    w: 640,
                    h: 480,
                    fps: 10,
                }
            } else {
                Video::Off
            };
            t.camera_on = on;
        })
    }

    fn set_video_options(&self, _station: &str, _front: bool, _night: bool) -> Result<(), UiError> {
        Ok(())
    }

    fn start_station(&self) -> LocalFuture<Result<(), UiError>> {
        if let Some(mut ui) = self.ui.get() {
            ui.station
                .set(StationRun::Running(Box::new(demo_station())));
        }
        Box::pin(async { Ok(()) })
    }

    fn stop_station(&self) -> LocalFuture<()> {
        if let Some(mut ui) = self.ui.get() {
            ui.station.set(StationRun::Stopped);
        }
        Box::pin(async {})
    }

    fn open_pairing(&self) -> LocalFuture<Result<(), UiError>> {
        if let Some(mut ui) = self.ui.get()
            && let StationRun::Running(m) = &mut *ui.station.write()
        {
            m.pairing = Some(PairingModel {
                invite_uri: "nestwatch://invite?v=1&s=q2X8Yb5Nw1Kc7Rt3Vm9Lp4Hd6Fg0Js2Az8Ce5Ui1Oy3&t=Z9x8W7v6U5t4S3r2Q1p0Ng&n=Nursery&a=192.168.1.20%3A47100&a=100.101.102.103%3A47100".into(),
                web_url: Some("http://192.168.1.20:47180/#nestwatch%3A%2F%2Finvite%3Fv%3D1".into()),
                relay_web_url: Some("https://nestwatch.appmire.be/app/#nestwatch%3A%2F%2Finvite%3Fv%3D1".into()),
                code: "7KQ2M-X9RTA".into(),
                remaining_ms: 552_000,
                pairings: 0,
                max_pairings: 8,
            });
        }
        if let Some(mut ui) = self.ui.get() {
            ui.just_paired
                .write()
                .push(("Mum's phone".into(), "042 917".into()));
        }
        Box::pin(async { Ok(()) })
    }

    fn close_pairing(&self) {
        if let Some(mut ui) = self.ui.get()
            && let StationRun::Running(m) = &mut *ui.station.write()
        {
            m.pairing = None;
        }
    }

    fn rename_station(&self, name: &str) -> Result<(), UiError> {
        if let Some(mut ui) = self.ui.get()
            && let StationRun::Running(m) = &mut *ui.station.write()
        {
            m.name = name.to_owned();
        }
        Ok(())
    }

    fn set_relay_enabled(&self, on: bool) -> Result<(), UiError> {
        if let Some(mut ui) = self.ui.get()
            && let StationRun::Running(m) = &mut *ui.station.write()
        {
            m.settings.relay_enabled = on;
            m.relay = if on {
                RelayView::Online
            } else {
                RelayView::Off
            };
        }
        Ok(())
    }

    fn set_relay_server(&self, origin: &str) -> Result<(), UiError> {
        if !origin.starts_with("https://") && !origin.starts_with("http://") {
            return Err(UiError::Other(crate::strings::RELAY_SERVER_INVALID.into()));
        }
        if let Some(mut ui) = self.ui.get()
            && let StationRun::Running(m) = &mut *ui.station.write()
        {
            m.settings.relay_server = origin.trim_end_matches('/').to_owned();
        }
        Ok(())
    }

    fn reset_relay_identity(&self) -> Result<(), UiError> {
        Ok(())
    }
}
