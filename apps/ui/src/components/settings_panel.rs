// SPDX-License-Identifier: AGPL-3.0-or-later
//! Per-station listener settings (design README §5.7): listening mode,
//! calibration chart with a draggable threshold (FR-AU-2), hold time,
//! volume, alert toggles, addresses and forget.

use dioxus::prelude::*;

use super::app::Nav;
use super::common::{Segmented, SwitchRow};
use crate::backend::{BackendHandle, Ui};
use crate::logic::{self, ThresholdKey};
use crate::model::{Conn, Mode, NoticeKind, Tile};
use crate::strings as s;

/// The side panel (desktop) / bottom sheet (phone width).
#[component]
pub fn SettingsPanel(tile: Tile) -> Element {
    let backend = use_context::<BackendHandle>();
    let ui = use_context::<Ui>();
    let mut nav = use_context::<Nav>();
    let set = tile.settings;
    let id = tile.id.clone();
    let mut hold_draft = use_signal(|| None::<u32>);
    let mut volume_draft = use_signal(|| None::<u32>);
    let mut address = use_signal(String::new);
    let mut address_error = use_signal(|| None::<String>);
    let mut confirm_forget = use_signal(|| false);
    let hold_s = hold_draft
        .read()
        .unwrap_or(logic::hold_seconds(set.hold_ms));
    let volume = volume_draft
        .read()
        .unwrap_or(logic::volume_percent(set.volume));

    let mode_opts = vec![
        (s::MODE_ALWAYS.to_owned(), set.mode == Mode::Continuous),
        (s::MODE_SOUND.to_owned(), set.mode == Mode::Activated),
    ];
    let b = backend.clone();
    let i = id.clone();
    let on_mode = move |idx: usize| {
        let mode = if idx == 0 {
            Mode::Continuous
        } else {
            Mode::Activated
        };
        let _ = b.set_listen(&i, mode, set.threshold_dbfs, set.hold_ms);
    };
    let (bh, ih) = (backend.clone(), id.clone());
    let (bv, iv) = (backend.clone(), id.clone());
    let on_sound = {
        let b = backend.clone();
        let i = id.clone();
        move |v: bool| {
            let _ = b.set_alerts(&i, v, set.alert_unreachable, set.alert_battery);
        }
    };
    let on_unreach = {
        let b = backend.clone();
        let i = id.clone();
        move |v: bool| {
            let _ = b.set_alerts(&i, set.alert_sound, v, set.alert_battery);
        }
    };
    let on_battery = {
        let b = backend.clone();
        let i = id.clone();
        move |v: bool| {
            let _ = b.set_alerts(&i, set.alert_sound, set.alert_unreachable, v);
        }
    };
    let (ba, ia) = (backend.clone(), id.clone());
    let (bf, idf) = (backend.clone(), id.clone());
    let (bp, ip) = (backend.clone(), id.clone());
    let following = tile.conn != Conn::Paused;
    let title = s::station_settings_of(&tile.name);
    rsx! {
        aside { class: "card stack panel", aria_label: "{title}",
            div { class: "row-between",
                div { class: "title",
                    h3 { "{tile.name}" }
                    span { class: "caption", "{s::LISTENING_SETTINGS}" }
                }
                button {
                    class: "btn icon ghost",
                    r#type: "button",
                    aria_label: s::CLOSE,
                    onclick: move |_| nav.panel.set(None),
                    "×"
                }
            }
            Segmented { label: s::LISTEN_MODE.to_owned(), options: mode_opts, onselect: on_mode }
            Calibration { tile: tile.clone() }
            p { class: "caption",
                if set.mode == Mode::Activated {
                    "{s::CALIBRATE_HINT}"
                } else {
                    "{s::CALIBRATE_CONTINUOUS}"
                }
            }
            if set.mode == Mode::Activated {
                div { class: "field",
                    label { r#for: "hold-{tile.id}", "{s::HOLD_TIME}" }
                    input {
                        id: "hold-{tile.id}",
                        class: "range",
                        r#type: "range",
                        min: "1",
                        max: "15",
                        step: "1",
                        value: "{hold_s}",
                        aria_valuetext: "{s::seconds(hold_s)}",
                        oninput: move |e: FormEvent| hold_draft.set(e.value().parse().ok()),
                        onchange: move |e: FormEvent| {
                            if let Ok(v) = e.value().parse::<u32>() {
                                let _ = bh.set_listen(&ih, set.mode, set.threshold_dbfs, v * 1000);
                            }
                            hold_draft.set(None);
                        },
                    }
                    span { class: "caption", "{s::seconds(hold_s)}" }
                }
            }
            div { class: "field",
                label { r#for: "vol-{tile.id}", "{s::VOLUME}" }
                input {
                    id: "vol-{tile.id}",
                    class: "range",
                    r#type: "range",
                    min: "0",
                    max: "100",
                    step: "1",
                    value: "{volume}",
                    aria_valuetext: "{s::percent(volume)}",
                    oninput: move |e: FormEvent| volume_draft.set(e.value().parse().ok()),
                    onchange: move |e: FormEvent| {
                        if let Ok(v) = e.value().parse::<u32>() {
                            let _ = bv.set_volume(&iv, v as f32 / 100.0);
                        }
                        volume_draft.set(None);
                    },
                }
            }
            hr { class: "divider" }
            SwitchRow { label: s::ALERT_SOUND.to_owned(), checked: set.alert_sound, onchange: on_sound }
            SwitchRow { label: s::ALERT_UNREACHABLE.to_owned(), checked: set.alert_unreachable, onchange: on_unreach }
            SwitchRow { label: s::ALERT_BATTERY.to_owned(), checked: set.alert_battery, onchange: on_battery }
            hr { class: "divider" }
            if let Some(r) = &tile.route {
                p { class: "caption", "{s::connected_via(r)}" }
            }
            form {
                class: "field",
                onsubmit: move |e: FormEvent| {
                    e.prevent_default();
                    let a = address.peek().trim().to_owned();
                    if a.is_empty() {
                        return;
                    }
                    match ba.add_address(&ia, &a) {
                        Ok(()) => {
                            address.set(String::new());
                            address_error.set(None);
                            ui.notify(NoticeKind::Info(s::ADDRESS_ADDED.into()));
                        }
                        Err(err) => address_error.set(Some(err.message())),
                    }
                },
                label { r#for: "addr-{tile.id}", "{s::ADD_ADDRESS}" }
                div { class: "row",
                    input {
                        id: "addr-{tile.id}",
                        class: "input grow",
                        placeholder: "host:47100",
                        autocomplete: "off",
                        spellcheck: "false",
                        value: "{address}",
                        oninput: move |e: FormEvent| address.set(e.value()),
                    }
                    button { class: "btn", r#type: "submit", "{s::SAVE}" }
                }
                span { class: "caption", "{s::ADDRESS_HINT}" }
                if let Some(err) = address_error.read().as_ref() {
                    span { class: "error", role: "alert", "{err}" }
                }
            }
            button {
                class: "btn ghost",
                r#type: "button",
                onclick: move |_| { let _ = bp.set_follow(&ip, !following); },
                if following { "{s::PAUSE}" } else { "{s::RESUME}" }
            }
            if *confirm_forget.read() {
                div { class: "card stack-s confirm", role: "alertdialog", aria_label: s::FORGET_CONFIRM,
                    p { "{s::FORGET_CONFIRM}" }
                    div { class: "row",
                        button {
                            class: "btn ghost grow",
                            r#type: "button",
                            onclick: move |_| confirm_forget.set(false),
                            "{s::CANCEL}"
                        }
                        button {
                            class: "btn danger grow",
                            r#type: "button",
                            onclick: move |_| {
                                let _ = bf.forget(&idf);
                                nav.panel.set(None);
                            },
                            "{s::FORGET_YES}"
                        }
                    }
                }
            } else {
                button {
                    class: "btn ghost",
                    r#type: "button",
                    onclick: move |_| confirm_forget.set(true),
                    "{s::FORGET}"
                }
            }
        }
    }
}

/// Last minute of the room level with a draggable threshold line. Works
/// with a pointer (drag) and the keyboard (WAI-ARIA slider).
#[component]
fn Calibration(tile: Tile) -> Element {
    let backend = use_context::<BackendHandle>();
    let ui = use_context::<Ui>();
    let mut draft = use_signal(|| None::<f32>);
    let mut dragging = use_signal(|| false);
    let set = tile.settings;
    let threshold = draft.read().unwrap_or(set.threshold_dbfs);
    let (path, _) = {
        let h = ui.history.read();
        match h.get(&tile.id) {
            Some(hist) => (
                logic::chart_path(hist.points(), hist.latest().unwrap_or(0)),
                hist.len(),
            ),
            None => (String::new(), 0),
        }
    };
    let top = format!("{:.1}%", logic::threshold_top_percent(threshold));
    let label = logic::db(threshold);
    let commit = {
        let backend = backend.clone();
        let id = tile.id.clone();
        move |v: f32| {
            let _ = backend.set_listen(&id, set.mode, v, set.hold_ms);
        }
    };
    let c1 = commit.clone();
    let c2 = commit.clone();
    let c3 = commit;
    let height = f64::from(logic::CHART_H);
    rsx! {
        div {
            class: "calibrate",
            role: "slider",
            tabindex: "0",
            aria_label: s::THRESHOLD,
            aria_orientation: "vertical",
            aria_valuemin: "{logic::THRESHOLD_MIN_DBFS}",
            aria_valuemax: "{logic::THRESHOLD_MAX_DBFS}",
            aria_valuenow: "{threshold}",
            aria_valuetext: "{label}",
            onpointerdown: move |e: PointerEvent| {
                dragging.set(true);
                draft.set(Some(logic::threshold_at(e.element_coordinates().y, height)));
            },
            onpointermove: move |e: PointerEvent| {
                if *dragging.peek() {
                    draft.set(Some(logic::threshold_at(e.element_coordinates().y, height)));
                }
            },
            onpointerup: move |_| {
                if *dragging.peek() {
                    dragging.set(false);
                    if let Some(v) = *draft.peek() {
                        c1(v);
                    }
                }
            },
            onpointerleave: move |_| {
                if *dragging.peek() {
                    dragging.set(false);
                    if let Some(v) = *draft.peek() {
                        c2(v);
                    }
                }
            },
            onkeydown: move |e: KeyboardEvent| {
                let key = match e.key() {
                    Key::ArrowUp | Key::ArrowRight => ThresholdKey::Up,
                    Key::ArrowDown | Key::ArrowLeft => ThresholdKey::Down,
                    Key::PageUp => ThresholdKey::PageUp,
                    Key::PageDown => ThresholdKey::PageDown,
                    Key::Home => ThresholdKey::Home,
                    Key::End => ThresholdKey::End,
                    _ => return,
                };
                e.prevent_default();
                let current = draft.peek().unwrap_or(set.threshold_dbfs);
                let v = logic::threshold_step(current, key);
                draft.set(Some(v));
                c3(v);
            },
            svg {
                view_box: "0 0 {logic::CHART_W} {logic::CHART_H}",
                preserve_aspect_ratio: "none",
                "aria-hidden": "true",
                path { class: "level-line", d: "{path}" }
            }
            div { class: "threshold", top: "{top}",
                span { "{label}" }
            }
        }
    }
}
