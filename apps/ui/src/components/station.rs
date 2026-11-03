// SPDX-License-Identifier: AGPL-3.0-or-later
//! Station screens (design README §5.2–5.4): hero ring, who is listening,
//! camera and talk indicators, pairing sheet, dimmed mode and settings.

use dioxus::core::spawn_forever;
use dioxus::prelude::*;

use super::about::ThemePicker;
use super::app::{Nav, Screen};
use super::common::{Brand, Ring, Segmented, SwitchRow};
use super::listener::RoleSwitch;
use crate::backend::{BackendHandle, Ui};
use crate::logic;
use crate::model::{CameraHere, Gain, PairingModel, StationModel, StationRun};
use crate::qr;
use crate::strings as s;

/// The station screen.
#[component]
pub fn StationScreen() -> Element {
    let backend = use_context::<BackendHandle>();
    let ui = use_context::<Ui>();
    let run = ui.station.read().clone();
    // Start the station when this screen shows and it is not running.
    {
        let backend = backend.clone();
        use_hook(move || {
            if matches!(*ui.station.peek(), StationRun::Stopped) {
                let f = backend.start_station();
                spawn_forever(async move {
                    let _ = f.await;
                });
            }
        });
    }
    let retry = backend.clone();
    match run {
        StationRun::Running(m) => rsx! { RunningStation { model: *m } },
        StationRun::Starting => rsx! {
            main { class: "page stack",
                header { class: "app-top", Brand { title: s::APP.to_owned() } RoleSwitch {} }
                p { class: "muted", role: "status", "{s::STATION_STARTING}" }
            }
        },
        StationRun::Stopped | StationRun::Failed(_) => {
            let failed = match &run {
                StationRun::Failed(e) => Some(e.clone()),
                _ => None,
            };
            rsx! {
                main { class: "page stack",
                    header { class: "app-top", Brand { title: s::APP.to_owned() } RoleSwitch {} }
                    div { class: "card stack",
                        if let Some(e) = failed {
                            p { role: "alert", strong { "{s::STATION_FAILED}" } " ({e})" }
                        } else {
                            p { "{s::STATION_STOPPED}" }
                        }
                        button {
                            class: "btn primary",
                            r#type: "button",
                            onclick: move |_| {
                                let f = retry.start_station();
                                spawn_forever(async move {
                                    let _ = f.await;
                                });
                            },
                            "{s::STATION_START}"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn RunningStation(model: StationModel) -> Element {
    let backend = use_context::<BackendHandle>();
    let mut nav = use_context::<Nav>();
    let mut dimmed = use_signal(|| false);
    let mut sheet = use_signal(|| false);
    let mut settings = use_signal(|| false);
    let threshold = logic::strictest_threshold(&model.listeners);
    let hearing = logic::anyone_hearing(&model.listeners);
    let count = model.listeners.len();
    let count_pill = if count == 0 {
        "pill plain"
    } else {
        "pill calm"
    };

    if *dimmed.read() {
        return rsx! {
            div {
                class: "dim-screen dimmed",
                role: "button",
                tabindex: "0",
                aria_label: s::DIMMED_LABEL,
                onclick: move |_| dimmed.set(false),
                onkeydown: move |_| dimmed.set(false),
                Ring { level: model.level_dbfs, threshold, sound: hearing, small: true }
                span { class: "{count_pill}", "{logic::listening_count(count)}" }
                p { class: "caption", "{s::TAP_TO_BRIGHTEN}" }
            }
        };
    }

    let open_pairing = backend.clone();
    rsx! {
        main { class: "page stack station",
            header { class: "app-top",
                Brand { title: model.name.clone() }
                div { class: "row",
                    RoleSwitch {}
                    button {
                        class: "btn icon ghost",
                        r#type: "button",
                        aria_label: s::SETTINGS,
                        aria_expanded: "{settings.read()}",
                        onclick: move |_| settings.set(true),
                        "⚙︎"
                    }
                }
            }
            // FR-ST-3: listeners, camera and talk, always visible.
            div { class: "row wrap indicators", role: "status",
                span { class: "{count_pill}", "{logic::listening_count(count)}" }
                if model.camera_on {
                    span { class: "pill cam", "{logic::camera_pill(&model.listeners)}" }
                } else {
                    span { class: "pill plain", "{s::CAMERA_OFF}" }
                }
                if let Some(t) = &model.talker {
                    span { class: "pill voice", "{s::talking_through(t)}" }
                } else if !model.settings.talk_enabled {
                    span { class: "pill plain", "{s::TALK_OFF}" }
                }
                if let Some(b) = logic::battery_caption(model.battery, model.charging) {
                    span { class: "chip", "{b}" }
                }
            }
            div { class: "hero",
                Ring { level: model.level_dbfs, threshold, sound: hearing }
            }
            section { class: "card stack-s listeners", aria_label: s::LISTENERS_HEADING,
                if model.listeners.is_empty() {
                    p { class: "muted", "{s::NO_LISTENERS}" }
                }
                for l in model.listeners.iter() {
                    div { key: "{l.session}", class: "row-between",
                        span { "{l.name}" }
                        span { class: "caption", "{logic::listener_row_caption(l)}" }
                    }
                }
            }
            div { class: "stack-s bottom-actions",
                button {
                    class: "btn primary block",
                    r#type: "button",
                    onclick: move |_| {
                        sheet.set(true);
                        let f = open_pairing.open_pairing();
                        spawn(async move {
                            let _ = f.await;
                        });
                    },
                    "{s::PAIR_LISTENER}"
                }
                button {
                    class: "btn ghost block",
                    r#type: "button",
                    onclick: move |_| dimmed.set(true),
                    "{s::DIM_SCREEN}"
                }
                p { class: "caption footnote", "{s::STATION_KEEPS_RUNNING}" }
                button {
                    class: "btn ghost",
                    r#type: "button",
                    onclick: move |_| nav.screen.set(Screen::About),
                    "{s::ABOUT}"
                }
            }
        }
        if *sheet.read() {
            PairingSheet {
                pairing: model.pairing.clone(),
                device_id: model.device_id.clone(),
                onclose: move |_| sheet.set(false),
            }
        }
        if *settings.read() {
            StationSettings { model: model.clone(), onclose: move |_| settings.set(false) }
        }
    }
}

/// The pairing sheet (FR-PAIR-1..5).
#[component]
fn PairingSheet(
    pairing: Option<PairingModel>,
    device_id: String,
    onclose: EventHandler<()>,
) -> Element {
    let backend = use_context::<BackendHandle>();
    let ui = use_context::<Ui>();
    let mut copied = use_signal(|| false);
    let paired = ui.just_paired.read().clone();
    let close = backend.clone();
    let copy = backend.clone();
    rsx! {
        div { class: "overlay", role: "dialog", aria_modal: "true", aria_label: s::PAIR_LISTENER,
            div { class: "sheet stack pairing",
                h2 { "{s::PAIR_LISTENER}" }
                match pairing {
                    None => rsx! {
                        p { class: "muted", role: "status",
                            if paired.is_empty() { "{s::PAIR_OPENING}" } else { "{s::PAIR_CLOSED}" }
                        }
                    },
                    Some(p) => {
                        let qr = qr::qr_svg(&p.invite_uri);
                        let left = p.max_pairings.saturating_sub(p.pairings);
                        let link = p.web_url.clone().unwrap_or_else(|| p.invite_uri.clone());
                        let link2 = link.clone();
                        rsx! {
                            p { class: "muted", "{s::PAIR_INTRO}" }
                            if let Some(q) = qr {
                                div { class: "qr",
                                    svg {
                                        view_box: "0 0 {q.size} {q.size}",
                                        width: "190",
                                        height: "190",
                                        shape_rendering: "crispEdges",
                                        role: "img",
                                        "aria-label": s::QR_LABEL,
                                        rect { width: "{q.size}", height: "{q.size}", fill: "#fff" }
                                        path { fill: "#17161d", d: "{q.path}" }
                                    }
                                }
                            }
                            div { class: "stack-s centered",
                                span { class: "caption", "{s::PAIR_CODE_INTRO}" }
                                span { class: "code-display", "{p.code}" }
                                span { class: "caption", "{s::PAIR_STATION_ID} " span { class: "mono", "{logic::grouped_id(&device_id)}" } }
                            }
                            p { class: "caption", role: "timer",
                                "{s::pairing_open_for(&logic::remaining(p.remaining_ms), left)}"
                            }
                            div { class: "row link-row",
                                span { class: "caption", "{s::PAIR_LINK}" }
                                a { class: "mono link grow", href: "{link}", "{link}" }
                                button {
                                    class: "btn ghost",
                                    r#type: "button",
                                    onclick: move |_| {
                                        if copy.copy_text(&link2) {
                                            copied.set(true);
                                        }
                                    },
                                    if *copied.read() { "{s::COPIED}" } else { "{s::COPY}" }
                                }
                            }
                        }
                    }
                }
                if !paired.is_empty() {
                    div { class: "card stack-s just-paired", role: "status",
                        span { class: "caption", "{s::PAIR_JUST_PAIRED}" }
                        for (i, (name, number)) in paired.iter().enumerate() {
                            div { key: "{i}", class: "row-between",
                                span { "{name}" }
                                span { class: "verify", "{number}" }
                            }
                        }
                    }
                }
                button {
                    class: "btn ghost block",
                    r#type: "button",
                    onclick: move |_| {
                        close.close_pairing();
                        onclose.call(());
                    },
                    "{s::DONE}"
                }
            }
        }
    }
}

/// Station settings sheet (FR-ST-7, FR-ST-8, FR-AU-5, FR-TB-3, FR-WEB-2).
#[component]
fn StationSettings(model: StationModel, onclose: EventHandler<()>) -> Element {
    let backend = use_context::<BackendHandle>();
    let mut name = use_signal(|| model.name.clone());
    let mut tb_draft = use_signal(|| None::<u32>);
    let set = model.settings.clone();
    let tb = tb_draft
        .read()
        .unwrap_or(logic::volume_percent(set.talkback_volume));
    let gains = [
        Gain::Auto,
        Gain::Db(0),
        Gain::Db(6),
        Gain::Db(12),
        Gain::Db(18),
    ];
    let gain_opts: Vec<(String, bool)> = gains
        .iter()
        .map(|g| {
            let label = match g {
                Gain::Auto => s::GAIN_AUTO.to_owned(),
                Gain::Db(0) => format!("0 {}", s::DB),
                Gain::Db(n) => format!("+{n} {}", s::DB),
            };
            (label, *g == set.gain)
        })
        .collect();
    let now = backend.unix_ms();
    let (b_name, b_cam, b_talk, b_gain, b_tb, b_stop) = (
        backend.clone(),
        backend.clone(),
        backend.clone(),
        backend.clone(),
        backend.clone(),
        backend.clone(),
    );
    let cam_note = match model.camera_here {
        CameraHere::Available => None,
        CameraHere::Unavailable => Some(s::CAMERA_UNAVAILABLE_HERE),
        CameraHere::Denied => Some(s::CAMERA_DENIED_HERE),
    };
    rsx! {
        div { class: "overlay", role: "dialog", aria_modal: "true", aria_label: s::STATION_SETTINGS,
            div { class: "sheet stack settings-sheet",
                div { class: "row-between",
                    h2 { "{s::STATION_SETTINGS}" }
                    button {
                        class: "btn icon ghost",
                        r#type: "button",
                        aria_label: s::CLOSE,
                        onclick: move |_| onclose.call(()),
                        "×"
                    }
                }
                form {
                    class: "field",
                    onsubmit: move |e: FormEvent| {
                        e.prevent_default();
                        let _ = b_name.rename_station(&name.peek());
                    },
                    label { r#for: "station-name", "{s::STATION_NAME}" }
                    div { class: "row",
                        input {
                            id: "station-name",
                            class: "input grow",
                            maxlength: "64",
                            value: "{name}",
                            oninput: move |e: FormEvent| name.set(e.value()),
                        }
                        button { class: "btn", r#type: "submit", "{s::SAVE}" }
                    }
                }
                SwitchRow {
                    label: s::CAMERA_ALLOWED.to_owned(),
                    checked: set.camera_enabled,
                    onchange: move |v: bool| b_cam.set_camera_enabled(v),
                }
                if let Some(n) = cam_note {
                    span { class: "caption", "{n}" }
                }
                SwitchRow {
                    label: s::TALK_ALLOWED.to_owned(),
                    checked: set.talk_enabled,
                    onchange: move |v: bool| b_talk.set_talk_enabled(v),
                }
                div { class: "field",
                    span { class: "label", "{s::MIC_SENSITIVITY}" }
                    Segmented {
                        label: s::MIC_SENSITIVITY.to_owned(),
                        options: gain_opts,
                        onselect: move |i: usize| b_gain.set_gain(gains[i]),
                    }
                }
                div { class: "field",
                    label { r#for: "tb-volume", "{s::TALKBACK_VOLUME}" }
                    input {
                        id: "tb-volume",
                        class: "range",
                        r#type: "range",
                        min: "0",
                        max: "100",
                        value: "{tb}",
                        aria_valuetext: "{s::percent(tb)}",
                        oninput: move |e: FormEvent| tb_draft.set(e.value().parse().ok()),
                        onchange: move |e: FormEvent| {
                            if let Ok(v) = e.value().parse::<u32>() {
                                b_tb.set_talkback_volume(v as f32 / 100.0);
                            }
                            tb_draft.set(None);
                        },
                    }
                }
                hr { class: "divider" }
                h3 { "{s::TRUSTED_LISTENERS}" }
                if model.trusted.is_empty() {
                    p { class: "muted", "{s::NO_TRUSTED}" }
                }
                div { class: "list",
                    for t in model.trusted.iter() {
                        {
                            let b = backend.clone();
                            let id = t.id.clone();
                            let caption = s::paired_seen(
                                &logic::ago(t.paired_unix_ms, now),
                                &logic::ago(t.last_seen_unix_ms, now),
                            );
                            let name = if t.name.is_empty() { s::UNNAMED.to_owned() } else { t.name.clone() };
                            rsx! {
                                div { key: "{t.id}", class: "item",
                                    div { class: "grow title",
                                        span { "{name}" }
                                        span { class: "caption", "{caption}" }
                                    }
                                    button {
                                        class: "btn danger",
                                        r#type: "button",
                                        aria_label: "{s::REVOKE} {name}",
                                        onclick: move |_| b.revoke(&id),
                                        "{s::REVOKE}"
                                    }
                                }
                            }
                        }
                    }
                }
                hr { class: "divider" }
                if let Some(u) = &model.http_url {
                    div { class: "field",
                        span { class: "label", "{s::BROWSER_ADDRESS}" }
                        span { class: "mono", "{u}" }
                        if let Some(h) = &model.https_url {
                            span { class: "mono", "{h}" }
                        }
                    }
                }
                if let Some(f) = &model.fingerprint {
                    div { class: "field",
                        span { class: "label", "{s::HTTPS_FINGERPRINT}" }
                        span { class: "mono fingerprint", "{f}" }
                        span { class: "caption", "{s::HTTPS_FINGERPRINT_HINT}" }
                    }
                }
                ThemePicker {}
                button {
                    class: "btn ghost",
                    r#type: "button",
                    onclick: move |_| {
                        let f = b_stop.stop_station();
                        spawn_forever(f);
                        onclose.call(());
                    },
                    "{s::STATION_STOP}"
                }
            }
        }
    }
}
