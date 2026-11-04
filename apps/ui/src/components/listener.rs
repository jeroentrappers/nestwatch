// SPDX-License-Identifier: AGPL-3.0-or-later
//! Listener home: station tiles (FR-LS-1..5) with the settings side panel.

use dioxus::prelude::*;

use super::app::{Nav, Screen};
use super::camera::CameraView;
use super::common::{Brand, Meter, Segmented, StatusPill, Toggle};
use super::settings_panel::SettingsPanel;
use crate::backend::{BackendHandle, Ui};
use crate::logic;
use crate::model::{Conn, Mode, Platform, Role, StationRun, Talk, Tile};
use crate::strings as s;

/// Role switch (design README §5.1): always on desktop, in the browser
/// while the station is in use.
#[component]
pub fn RoleSwitch() -> Element {
    let backend = use_context::<BackendHandle>();
    let nav = use_context::<Nav>();
    let ui = use_context::<Ui>();
    let caps = backend.capabilities();
    if !caps.station {
        return rsx! {};
    }
    let role = nav.prefs.read().role;
    // In the browser the switch appears once the station is in use; until
    // then the listener home offers "Use this device as a station".
    let station_in_use = role == Some(Role::Station)
        || !matches!(
            *ui.station.read(),
            StationRun::Stopped | StationRun::Failed(_)
        );
    if caps.platform == Platform::Web && !station_in_use {
        return rsx! {};
    }
    let options = vec![
        (
            s::ROLE_TAB_LISTENING.to_owned(),
            role == Some(Role::Listener),
        ),
        (s::ROLE_TAB_STATION.to_owned(), role == Some(Role::Station)),
    ];
    rsx! {
        Segmented {
            label: s::ROLE_GROUP.to_owned(),
            options,
            onselect: move |i: usize| {
                let r = if i == 0 { Role::Listener } else { Role::Station };
                nav.update_prefs(&backend, |p| p.role = Some(r));
            },
        }
    }
}

/// The listener's home screen.
#[component]
pub fn ListenerHome() -> Element {
    let backend = use_context::<BackendHandle>();
    let ui = use_context::<Ui>();
    let mut nav = use_context::<Nav>();
    let caps = backend.capabilities();
    let model = ui.listener.read().clone();
    let panel = nav.panel.read().clone();
    let panel_tile = panel.as_deref().and_then(|id| model.tile(id)).cloned();
    let locked = *ui.audio_locked.read();
    let askable = *ui.notifications_askable.read();
    let layout = if panel_tile.is_some() {
        "layout with-panel"
    } else {
        "layout"
    };
    let unlock = backend.clone();
    let notif = backend.clone();
    let use_station = backend.clone();
    // FR-WEBST-1: the browser's entry to the station role.
    let offer_station = caps.platform == Platform::Web
        && caps.station
        && matches!(
            *ui.station.read(),
            StationRun::Stopped | StationRun::Failed(_)
        );
    rsx! {
        main { class: "page stack",
            header { class: "app-top",
                Brand { title: s::APP.to_owned() }
                div { class: "row wrap",
                    RoleSwitch {}
                    button {
                        class: "btn ghost",
                        r#type: "button",
                        onclick: move |_| nav.screen.set(Screen::AddStation),
                        "{s::ADD_STATION_BUTTON}"
                    }
                    button {
                        class: "btn icon ghost",
                        r#type: "button",
                        aria_label: s::ABOUT,
                        onclick: move |_| nav.screen.set(Screen::About),
                        "ⓘ"
                    }
                }
            }
            if locked {
                button {
                    class: "card audio-prompt",
                    r#type: "button",
                    onclick: move |_| unlock.unlock_audio(),
                    strong { "{s::TAP_TO_LISTEN}" }
                    span { class: "caption", "{s::TAP_TO_LISTEN_HINT}" }
                }
            }
            if askable {
                button {
                    class: "btn ghost",
                    r#type: "button",
                    onclick: move |_| notif.request_notifications(),
                    "{s::ENABLE_NOTIFICATIONS}"
                }
            }
            div { class: "{layout}",
                if model.tiles.is_empty() {
                    div { class: "card stack empty",
                        p { "{s::NO_STATIONS}" }
                        button {
                            class: "btn primary",
                            r#type: "button",
                            onclick: move |_| nav.screen.set(Screen::AddStation),
                            "{s::ADD_STATION}"
                        }
                    }
                } else {
                    div { class: "grid tiles",
                        for t in model.tiles.iter() {
                            TileCard {
                                key: "{t.id}",
                                tile: t.clone(),
                                solo: model.solo.as_deref() == Some(t.id.as_str()),
                            }
                        }
                    }
                }
                if let Some(t) = panel_tile {
                    SettingsPanel { key: "{t.id}", tile: t }
                }
            }
            p { class: "caption footnote",
                if caps.platform == Platform::Web {
                    "{s::BROWSER_KEEP_OPEN}"
                } else {
                    "{s::KEEPS_LISTENING}"
                }
            }
            if offer_station {
                button {
                    class: "btn ghost web-station-entry",
                    r#type: "button",
                    onclick: move |_| {
                        nav.update_prefs(&use_station, |p| p.role = Some(Role::Station));
                    },
                    "{s::STATION_USE_THIS_DEVICE}"
                }
            }
        }
    }
}

/// One station tile.
#[component]
pub fn TileCard(tile: Tile, solo: bool) -> Element {
    let backend = use_context::<BackendHandle>();
    let mut nav = use_context::<Nav>();
    let pill = logic::tile_pill(&tile);
    let subtitle = logic::tile_subtitle(&tile);
    let live = tile.conn == Conn::Live;
    let class = if tile.alerting {
        "card tile alert"
    } else {
        "card tile"
    };
    let label = s::tile_label(&tile.name, &pill.text);
    let threshold = (tile.settings.mode == Mode::Activated).then_some(tile.settings.threshold_dbfs);
    let battery = logic::battery_caption(tile.battery, tile.charging);
    let battery_class = if logic::battery_low(tile.battery, tile.charging) {
        "battery low"
    } else {
        "battery"
    };
    let id = tile.id.clone();

    let b1 = backend.clone();
    let id1 = id.clone();
    let muted = tile.settings.muted;
    let b2 = backend.clone();
    let id2 = id.clone();
    let cam_open = tile.video.is_open();
    let b3 = backend.clone();
    let id3 = id.clone();
    let b4 = backend.clone();
    let id4 = id.clone();
    let id5 = id.clone();
    let settings_label = s::station_settings_of(&tile.name);

    rsx! {
        article { class: "{class}", aria_label: "{label}",
            div { class: "row-between",
                div { class: "title",
                    h2 { "{tile.name}" }
                    if !subtitle.is_empty() {
                        span { class: "caption", "{subtitle}" }
                    }
                }
                StatusPill { pill }
            }
            if tile.alerting {
                span { class: "sr-only", "{s::SOUND_ALERT_SR}" }
            }
            Meter { level: if live { tile.level_dbfs } else { None }, threshold }
            if live {
                div { class: "row-between caption",
                    span { "{logic::level_caption(&tile)}" }
                    if let Some(b) = battery {
                        span { class: "{battery_class}", "{b}" }
                    }
                }
            }
            match &tile.conn {
                Conn::Reconnecting { attempt, .. } => rsx! {
                    p { class: "caption",
                        if let Some(d) = tile.down_for_ms {
                            "{s::unreachable_for(&logic::duration(d))} "
                        }
                        span { class: "sr-only", "{s::attempt(*attempt)}" }
                    }
                },
                Conn::Connecting { attempt, .. } => rsx! {
                    p { class: "caption", "{s::attempt(*attempt)}" }
                },
                Conn::NeedsPairing => rsx! {
                    p { class: "caption", "{s::NEEDS_PAIRING_HINT}" }
                },
                Conn::Paused => rsx! {
                    div { class: "row-between",
                        p { class: "caption", "{s::PAUSED_HINT}" }
                        button {
                            class: "btn",
                            r#type: "button",
                            onclick: move |_| { let _ = b4.set_follow(&id4, true); },
                            "{s::RESUME}"
                        }
                    }
                },
                Conn::Live => rsx! {},
            }
            if live && cam_open {
                CameraView { tile: tile.clone() }
            }
            div { class: "actions",
                if live {
                    Toggle {
                        label: s::MUTE.to_owned(),
                        pressed: muted,
                        onclick: move |_| { let _ = b1.set_muted(&id1, !muted); },
                    }
                    Toggle {
                        label: s::CAMERA.to_owned(),
                        pressed: cam_open,
                        disabled: !tile.can_video,
                        onclick: move |_| { let _ = b2.set_camera(&id2, !cam_open); },
                    }
                    Toggle {
                        label: s::SOLO.to_owned(),
                        pressed: solo,
                        onclick: move |_| {
                            let _ = b3.set_solo(if solo { None } else { Some(id3.as_str()) });
                        },
                    }
                }
                button {
                    class: "btn icon settings-btn",
                    r#type: "button",
                    aria_label: "{settings_label}",
                    aria_expanded: "{nav.panel.read().as_deref() == Some(id5.as_str())}",
                    onclick: move |_| {
                        let open = nav.panel.peek().as_deref() == Some(id5.as_str());
                        nav.panel.set(if open { None } else { Some(id5.clone()) });
                    },
                    "⚙︎"
                }
            }
            if live && tile.can_talk {
                TalkButton { id: tile.id.clone(), talk: tile.talk.clone() }
            }
        }
    }
}

/// Hold to talk (FR-TB-1): pointer down/up, or Space/Enter held.
#[component]
fn TalkButton(id: String, talk: Talk) -> Element {
    let backend = use_context::<BackendHandle>();
    let mut held = use_signal(|| false);
    let (text, disabled) = match &talk {
        Talk::Granted => (s::TALKING.to_owned(), false),
        Talk::Requested => (s::TALK_WAIT.to_owned(), false),
        Talk::Busy(by) => (s::talk_busy(by.as_deref()), true),
        Talk::Disabled => (s::TALK_DISABLED.to_owned(), true),
        Talk::Idle => (s::HOLD_TO_TALK.to_owned(), false),
    };
    let active = *held.read() || talk == Talk::Granted;
    let class = if active {
        "btn talk active"
    } else {
        "btn talk"
    };
    let press = {
        let backend = backend.clone();
        let id = id.clone();
        move || {
            if !*held.peek() {
                held.set(true);
                let _ = backend.talk(&id, true);
            }
        }
    };
    let release = {
        let backend = backend.clone();
        let id = id.clone();
        move || {
            if *held.peek() {
                held.set(false);
                let _ = backend.talk(&id, false);
            }
        }
    };
    let (mut p1, mut p2) = (press.clone(), press);
    let (mut r1, mut r2, mut r3, mut r4, mut r5) = (
        release.clone(),
        release.clone(),
        release.clone(),
        release.clone(),
        release,
    );
    rsx! {
        button {
            class: "{class}",
            r#type: "button",
            aria_pressed: "{active}",
            disabled,
            onpointerdown: move |_| p1(),
            onpointerup: move |_| r1(),
            onpointercancel: move |_| r2(),
            onpointerleave: move |_| r3(),
            onkeydown: move |e: KeyboardEvent| {
                if is_talk_key(&e.key()) {
                    e.prevent_default();
                    if !e.is_auto_repeating() {
                        p2();
                    }
                }
            },
            onkeyup: move |e: KeyboardEvent| {
                if is_talk_key(&e.key()) {
                    e.prevent_default();
                    r4();
                }
            },
            onblur: move |_| r5(),
            "{text}"
        }
    }
}

fn is_talk_key(k: &Key) -> bool {
    match k {
        Key::Enter => true,
        Key::Character(c) => c == " ",
        _ => false,
    }
}
