// SPDX-License-Identifier: AGPL-3.0-or-later
//! The station's relay setting (rendezvous.md FR-RV-1, FR-RV-6, §3.2, §7):
//! the one-time setup question, the settings section and the plain-language
//! explanation of what the server can and cannot see.

use dioxus::prelude::*;

use super::app::Nav;
use super::common::SwitchRow;
use crate::backend::BackendHandle;
use crate::logic;
use crate::model::{RelayView, StationModel};
use crate::strings as s;

/// What the relay is for and what its operator can learn (§7).
#[component]
pub fn RelayExplainer(server: String) -> Element {
    rsx! {
        div { class: "stack-s relay-explainer",
            p { "{s::RELAY_INTRO}" }
            p { class: "caption mono", "{s::relay_server_named(&server)}" }
            div { class: "stack-s",
                strong { "{s::RELAY_CAN_SEE_HEADING}" }
                p { class: "muted", "{s::RELAY_CAN_SEE}" }
            }
            div { class: "stack-s",
                strong { "{s::RELAY_CANNOT_SEE_HEADING}" }
                p { class: "muted", "{s::RELAY_CANNOT_SEE}" }
            }
            p { class: "caption", "{s::RELAY_TRAFFIC_NOTE}" }
        }
    }
}

/// Station setup asks once whether to use the relay; it stays off unless
/// the user opts in (FR-RV-1).
#[component]
pub fn RelaySetup(server: String) -> Element {
    let backend = use_context::<BackendHandle>();
    let nav = use_context::<Nav>();
    let (yes, no) = (backend.clone(), backend.clone());
    rsx! {
        div { class: "overlay", role: "dialog", aria_modal: "true", aria_label: s::RELAY_SETUP_TITLE,
            div { class: "sheet stack relay-setup",
                h2 { "{s::RELAY_SETUP_TITLE}" }
                RelayExplainer { server }
                div { class: "stack-s",
                    button {
                        class: "btn primary block",
                        r#type: "button",
                        onclick: move |_| {
                            let _ = yes.set_relay_enabled(true);
                            nav.update_prefs(&yes, |p| p.relay_asked = true);
                        },
                        "{s::RELAY_SETUP_YES}"
                    }
                    button {
                        class: "btn ghost block",
                        r#type: "button",
                        onclick: move |_| {
                            let _ = no.set_relay_enabled(false);
                            nav.update_prefs(&no, |p| p.relay_asked = true);
                        },
                        "{s::RELAY_SETUP_NO}"
                    }
                    p { class: "caption footnote", "{s::RELAY_SETUP_LATER}" }
                }
            }
        }
    }
}

/// The relay section of the station settings: toggle, status pill, server
/// address (FR-RV-6) and "reset relay identity" (§3.2).
#[component]
pub fn RelaySettings(model: StationModel) -> Element {
    let backend = use_context::<BackendHandle>();
    let nav = use_context::<Nav>();
    let set = model.settings.clone();
    let mut server = use_signal(|| set.relay_server.clone());
    let mut error = use_signal(|| None::<String>);
    let mut confirm = use_signal(|| false);
    let pill = logic::relay_pill(&model.relay);
    let (b_toggle, b_server, b_reset) = (backend.clone(), backend.clone(), backend.clone());
    let detail = match &model.relay {
        RelayView::Error {
            permanent: true, ..
        } => Some(s::RELAY_REFUSED_HINT.to_owned()),
        RelayView::Error { message, .. } => Some(message.clone()),
        _ => None,
    };
    rsx! {
        section { class: "stack-s relay-settings", aria_label: s::RELAY_TITLE,
            SwitchRow {
                label: s::RELAY_TITLE.to_owned(),
                checked: set.relay_enabled,
                onchange: move |v: bool| {
                    let _ = b_toggle.set_relay_enabled(v);
                    nav.update_prefs(&b_toggle, |p| p.relay_asked = true);
                },
            }
            div { class: "row wrap", role: "status",
                span { class: "{pill.classes()}", "{pill.text}" }
                if let Some(d) = detail {
                    span { class: "caption", "{d}" }
                }
            }
            RelayExplainer { server: set.relay_server.clone() }
            form {
                class: "field",
                onsubmit: move |e: FormEvent| {
                    e.prevent_default();
                    match b_server.set_relay_server(&server.peek()) {
                        Ok(()) => error.set(None),
                        Err(e) => error.set(Some(e.message())),
                    }
                },
                label { r#for: "relay-server", "{s::RELAY_SERVER}" }
                div { class: "row",
                    input {
                        id: "relay-server",
                        class: "input grow mono",
                        r#type: "url",
                        inputmode: "url",
                        spellcheck: "false",
                        autocomplete: "off",
                        value: "{server}",
                        aria_describedby: "relay-server-hint",
                        oninput: move |e: FormEvent| server.set(e.value()),
                    }
                    button { class: "btn", r#type: "submit", "{s::SAVE}" }
                }
                span { id: "relay-server-hint", class: "caption", "{s::RELAY_SERVER_HINT}" }
                if let Some(e) = error.read().as_ref() {
                    span { class: "caption", role: "alert", "{e}" }
                }
            }
            div { class: "stack-s",
                if *confirm.read() {
                    p { class: "caption", "{s::RELAY_RESET_HINT}" }
                    div { class: "row",
                        button {
                            class: "btn danger",
                            r#type: "button",
                            onclick: move |_| {
                                let _ = b_reset.reset_relay_identity();
                                confirm.set(false);
                            },
                            "{s::RELAY_RESET_CONFIRM}"
                        }
                        button {
                            class: "btn ghost",
                            r#type: "button",
                            onclick: move |_| confirm.set(false),
                            "{s::CANCEL}"
                        }
                    }
                } else {
                    button {
                        class: "btn ghost",
                        r#type: "button",
                        onclick: move |_| confirm.set(true),
                        "{s::RELAY_RESET}"
                    }
                }
            }
        }
    }
}
