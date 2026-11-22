// SPDX-License-Identifier: AGPL-3.0-or-later
//! Listener home: station tiles (FR-LS-1..5) with the settings side panel.

use dioxus::prelude::*;

use super::app::{Nav, Screen};
use super::camera::CameraView;
use super::common::{Brand, Meter, Segmented, StatusPill, Toggle};
use super::settings_panel::SettingsPanel;
use crate::backend::{BackendHandle, Ui};
use crate::logic;
use crate::model::{Conn, Mode, Platform, Role, Room, StationRun, Talk, Tile};
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
            s::role_tab_listening().to_owned(),
            role == Some(Role::Listener),
        ),
        (
            s::role_tab_station().to_owned(),
            role == Some(Role::Station),
        ),
    ];
    rsx! {
        Segmented {
            label: s::role_group().to_owned(),
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
                Brand { title: s::app().to_owned() }
                div { class: "row wrap",
                    RoleSwitch {}
                    button {
                        class: "btn ghost",
                        r#type: "button",
                        onclick: move |_| nav.screen.set(Screen::AddStation),
                        "{s::add_station_button()}"
                    }
                    button {
                        class: "btn icon ghost",
                        r#type: "button",
                        aria_label: s::about(),
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
                    strong { "{s::tap_to_listen()}" }
                    span { class: "caption", "{s::tap_to_listen_hint()}" }
                }
            }
            if askable {
                button {
                    class: "btn ghost",
                    r#type: "button",
                    onclick: move |_| notif.request_notifications(),
                    "{s::enable_notifications()}"
                }
            }
            div { class: "{layout}",
                if model.tiles.is_empty() {
                    div { class: "card stack empty",
                        p { "{s::no_stations()}" }
                        button {
                            class: "btn primary",
                            r#type: "button",
                            onclick: move |_| nav.screen.set(Screen::AddStation),
                            "{s::add_station()}"
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
                    "{s::browser_keep_open()}"
                } else {
                    "{s::keeps_listening()}"
                }
            }
            if offer_station {
                button {
                    class: "btn ghost web-station-entry",
                    r#type: "button",
                    onclick: move |_| {
                        nav.update_prefs(&use_station, |p| p.role = Some(Role::Station));
                    },
                    "{s::station_use_this_device()}"
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
    let class = if tile.alarm.is_ringing() {
        "card tile alert ringing"
    } else if tile.alerting {
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
    let ui = use_context::<Ui>();
    let now = *ui.now_unix_ms.read();
    let room = logic::room_caption(&tile, now);
    let paused = logic::paused_caption(&tile, now);
    let banner = logic::banner(&tile);
    // "Not the baby" also while awake without a banner (FR-AL-7).
    let awake_offer = matches!(tile.room, Room::Awake { .. })
        && live
        && banner == logic::Banner::None
        && paused.is_none();

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
                span { class: "sr-only", "{s::sound_alert_sr()}" }
            }
            AlertBanner { id: tile.id.clone(), banner: banner.clone() }
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
                    p { class: "caption", "{s::needs_pairing_hint()}" }
                },
                Conn::Paused => rsx! {
                    div { class: "row-between",
                        p { class: "caption", "{s::paused_hint()}" }
                        button {
                            class: "btn",
                            r#type: "button",
                            onclick: move |_| { let _ = b4.set_follow(&id4, true); },
                            "{s::resume()}"
                        }
                    }
                },
                Conn::Live => rsx! {},
            }
            if room.is_some() || awake_offer {
                div { class: "row-between room-status",
                    if let Some(r) = room {
                        span { class: "caption", "{r}" }
                    }
                    if awake_offer {
                        AnswerButton { id: tile.id.clone(), ack: false, class: "btn ghost" }
                    }
                }
            }
            if let Some(p) = paused {
                div { class: "row-between paused-notice", role: "status",
                    span { class: "caption", "{p}" }
                    ResumeAlertsButton { id: tile.id.clone() }
                }
            }
            if live && cam_open {
                CameraView { tile: tile.clone() }
            }
            div { class: "actions",
                if live {
                    Toggle {
                        label: s::mute().to_owned(),
                        pressed: muted,
                        onclick: move |_| { let _ = b1.set_muted(&id1, !muted); },
                    }
                    Toggle {
                        label: s::camera().to_owned(),
                        pressed: cam_open,
                        disabled: !tile.can_video,
                        onclick: move |_| { let _ = b2.set_camera(&id2, !cam_open); },
                    }
                    Toggle {
                        label: s::solo().to_owned(),
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

/// "I'm on it" (`ack`) or "Not the baby" for station `id` (FR-AL-6, FR-AL-7).
#[component]
fn AnswerButton(id: String, ack: bool, class: String) -> Element {
    let backend = use_context::<BackendHandle>();
    let text = if ack {
        s::im_on_it()
    } else {
        s::not_the_baby()
    };
    rsx! {
        button {
            class: "{class}",
            r#type: "button",
            onclick: move |_| {
                let _ = if ack { backend.acknowledge(&id) } else { backend.not_the_baby(&id) };
            },
            "{text}"
        }
    }
}

#[component]
fn ResumeAlertsButton(id: String) -> Element {
    let backend = use_context::<BackendHandle>();
    rsx! {
        button {
            class: "btn",
            r#type: "button",
            onclick: move |_| { let _ = backend.resume_alerts(&id); },
            "{s::resume_alerts()}"
        }
    }
}

/// The banner of an unanswered sound alert or a ringing alarm (FR-AL-6):
/// icon and words, never colour alone. Seeing it does not acknowledge.
#[component]
fn AlertBanner(id: String, banner: logic::Banner) -> Element {
    let (class, title, hint) = match banner {
        logic::Banner::None => return rsx! {},
        logic::Banner::Pending(t) => ("alert-banner pending", t, None),
        logic::Banner::Ringing(t, h) => ("alert-banner ringing", t, Some(h)),
    };
    let ringing = hint.is_some();
    rsx! {
        div { class: "{class}", role: "alert",
            div { class: "row",
                span { class: "banner-icon", "aria-hidden": "true",
                    if ringing { "⏰" } else { "♪" }
                }
                div { class: "title",
                    strong { "{title}" }
                    if let Some(h) = hint {
                        span { class: "caption", "{h}" }
                    }
                }
            }
            div { class: "row",
                AnswerButton {
                    id: id.clone(),
                    ack: true,
                    class: if ringing { "btn alarm grow" } else { "btn primary grow" },
                }
                AnswerButton { id, ack: false, class: "btn ghost grow" }
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
        Talk::Granted => (s::talking().to_owned(), false),
        Talk::Requested => (s::talk_wait().to_owned(), false),
        Talk::Busy(by) => (s::talk_busy(by.as_deref()), true),
        Talk::Disabled => (s::talk_disabled().to_owned(), true),
        Talk::Idle => (s::hold_to_talk().to_owned(), false),
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
