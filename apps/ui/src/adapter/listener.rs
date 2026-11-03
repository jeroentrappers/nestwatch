// SPDX-License-Identifier: AGPL-3.0-or-later
//! Listener snapshot → [`ListenerModel`] (FR-LS-3).

use std::collections::HashMap;

use nestwatch_core::ListenMode;
use nestwatch_core::VideoStateKind;
use nestwatch_core::listener::{
    ConnStatus, ListenerState, NetPath as CoreNetPath, StationView, TalkView, VideoView,
};

use crate::logic;
use crate::model::{
    Conn, ListenerModel, Mode, Nearby, NetPath, Talk, Tile, TileSettings, Video, VideoProblem,
};

/// Remembers what snapshots do not carry: when a station went down.
#[derive(Debug, Default)]
pub struct Tracker {
    down_since: HashMap<String, u64>,
}

/// Maps a listener snapshot at monotonic time `now_ms`. `talk_possible` is
/// false where this platform cannot capture the microphone (a browser
/// without a secure context).
pub fn map_listener(
    state: &ListenerState,
    tracker: &mut Tracker,
    now_ms: u64,
    talk_possible: bool,
) -> ListenerModel {
    tracker
        .down_since
        .retain(|id, _| state.stations.iter().any(|s| &s.id == id));
    let tiles = state
        .stations
        .iter()
        .map(|v| map_tile(v, tracker, now_ms, talk_possible))
        .collect();
    ListenerModel {
        name: state.name.clone(),
        device_id: state.device_id.clone(),
        tiles,
        solo: state.solo.clone(),
        nearby: state
            .discovered
            .iter()
            .map(|d| Nearby {
                id: d.id.clone(),
                name: d.name.clone(),
                pairing: d.pairing,
                known: d.known,
                endpoints: d.endpoints.iter().map(ToString::to_string).collect(),
            })
            .collect(),
    }
}

fn map_conn(s: &ConnStatus) -> Conn {
    match s {
        ConnStatus::Idle => Conn::Paused,
        ConnStatus::Connecting { attempt, tried } => Conn::Connecting {
            attempt: *attempt,
            tried: tried.len(),
        },
        ConnStatus::Live => Conn::Live,
        ConnStatus::Reconnecting { attempt, tried } => Conn::Reconnecting {
            attempt: *attempt,
            tried: tried.len(),
        },
        ConnStatus::NeedsPairing => Conn::NeedsPairing,
    }
}

fn map_tile(v: &StationView, tracker: &mut Tracker, now_ms: u64, talk_possible: bool) -> Tile {
    let conn = map_conn(&v.status);
    let down_for_ms = match conn {
        Conn::Connecting { .. } | Conn::Reconnecting { .. } => {
            let since = *tracker.down_since.entry(v.id.clone()).or_insert(now_ms);
            Some(now_ms.saturating_sub(since))
        }
        _ => {
            tracker.down_since.remove(&v.id);
            None
        }
    };
    let s = &v.settings;
    let settings = TileSettings {
        mode: match s.listen.mode {
            ListenMode::Continuous => Mode::Continuous,
            ListenMode::Activated => Mode::Activated,
        },
        threshold_dbfs: s.listen.threshold_dbfs,
        hold_ms: s.listen.hold_ms,
        preroll_ms: s.listen.preroll_ms,
        volume: s.volume,
        muted: s.muted,
        alert_sound: s.alerts.sound,
        alert_unreachable: s.alerts.unreachable,
        alert_battery: s.alerts.battery,
    };
    let has = |c: &str| v.caps.iter().any(|x| x == c);
    let live = conn == Conn::Live;
    let mut tile = Tile {
        id: v.id.clone(),
        name: v.name.clone(),
        conn,
        path: v.path.map(|p| match p {
            CoreNetPath::Lan => NetPath::Lan,
            CoreNetPath::Vpn => NetPath::Vpn,
        }),
        route: v.route.clone(),
        level_dbfs: v.level_dbfs,
        open: v.open,
        battery: v.battery,
        charging: v.charging,
        station_listeners: v.station_listeners,
        camera_on: v.camera_on,
        talker: v.talker.clone(),
        talk: match &v.talk {
            TalkView::Idle => Talk::Idle,
            TalkView::Requested => Talk::Requested,
            TalkView::Granted => Talk::Granted,
            TalkView::Busy { by } => Talk::Busy(by.clone()),
            TalkView::Disabled => Talk::Disabled,
        },
        video: match &v.video {
            VideoView::Off => Video::Off,
            VideoView::Requested => Video::Starting,
            VideoView::Streaming { w, h, fps } => Video::Streaming {
                w: *w,
                h: *h,
                fps: *fps,
            },
            VideoView::State(k) => match k {
                VideoStateKind::Streaming => Video::Starting,
                VideoStateKind::Stopped => Video::Problem(VideoProblem::Stopped),
                VideoStateKind::Unavailable => Video::Problem(VideoProblem::Unavailable),
                VideoStateKind::Denied => Video::Problem(VideoProblem::Denied),
                VideoStateKind::Disabled => Video::Problem(VideoProblem::Disabled),
            },
        },
        // Capabilities are only known from a live session's `hello`.
        can_talk: talk_possible && (!live || has("talk")),
        can_video: !live || has("video"),
        settings,
        down_for_ms,
        routes: 0,
        alerting: false,
    };
    // The talker in `status` is ourselves while granted.
    if tile.talk == Talk::Granted {
        tile.talker = None;
    }
    tile.alerting = logic::has_sound(&tile);
    tile
}

#[cfg(test)]
mod tests {
    use super::*;
    use nestwatch_core::listener::StationSettings;

    fn view(status: ConnStatus) -> StationView {
        StationView {
            id: "aaaabbbbccccdddd".into(),
            name: "Nursery".into(),
            status,
            path: Some(CoreNetPath::Vpn),
            route: Some("100.64.0.2:47100".into()),
            level_dbfs: Some(-30.0),
            open: true,
            battery: Some(50),
            charging: None,
            station_listeners: Some(2),
            camera_on: false,
            talker: Some("Me".into()),
            talk: TalkView::Granted,
            video: VideoView::State(VideoStateKind::Disabled),
            rtt_ms: Some(4),
            caps: vec!["audio".into(), "talk".into()],
            settings: StationSettings::default(),
            audio_frames: 0,
            preroll_frames: 0,
        }
    }

    fn state(v: StationView) -> ListenerState {
        ListenerState {
            name: "Laptop".into(),
            device_id: "zzzz".into(),
            stations: vec![v],
            solo: None,
            discovered: Vec::new(),
        }
    }

    #[test]
    fn live_station_maps_to_a_tile() {
        let mut tr = Tracker::default();
        let m = map_listener(&state(view(ConnStatus::Live)), &mut tr, 1_000, true);
        let t = &m.tiles[0];
        assert_eq!(t.conn, Conn::Live);
        assert_eq!(t.path, Some(NetPath::Vpn));
        assert_eq!(t.talk, Talk::Granted);
        assert_eq!(t.talker, None, "we are the talker");
        assert!(t.can_talk);
        assert!(!t.can_video, "no video cap");
        assert_eq!(t.video, Video::Problem(VideoProblem::Disabled));
        assert!(t.alerting);
        assert_eq!(t.down_for_ms, None);
        assert_eq!(t.settings.threshold_dbfs, -50.0);
    }

    #[test]
    fn outage_duration_is_tracked_until_live() {
        let mut tr = Tracker::default();
        let st = state(view(ConnStatus::Reconnecting {
            attempt: 2,
            tried: vec!["a".into(), "b".into()],
        }));
        let m = map_listener(&st, &mut tr, 10_000, true);
        assert_eq!(m.tiles[0].down_for_ms, Some(0));
        assert_eq!(
            m.tiles[0].conn,
            Conn::Reconnecting {
                attempt: 2,
                tried: 2
            }
        );
        assert!(!m.tiles[0].alerting);
        let m = map_listener(&st, &mut tr, 52_000, true);
        assert_eq!(m.tiles[0].down_for_ms, Some(42_000));
        let m = map_listener(&state(view(ConnStatus::Live)), &mut tr, 53_000, true);
        assert_eq!(m.tiles[0].down_for_ms, None);
        let m = map_listener(&st, &mut tr, 60_000, true);
        assert_eq!(m.tiles[0].down_for_ms, Some(0));
    }

    #[test]
    fn talk_needs_platform_support() {
        let mut tr = Tracker::default();
        let m = map_listener(&state(view(ConnStatus::Live)), &mut tr, 0, false);
        assert!(!m.tiles[0].can_talk);
    }
}
