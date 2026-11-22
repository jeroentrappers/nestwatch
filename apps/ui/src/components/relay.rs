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
            p { "{s::relay_intro()}" }
            p { class: "caption mono", "{s::relay_server_named(&server)}" }
            div { class: "stack-s",
                strong { "{s::relay_can_see_heading()}" }
                p { class: "muted", "{s::relay_can_see()}" }
            }
            div { class: "stack-s",
                strong { "{s::relay_cannot_see_heading()}" }
                p { class: "muted", "{s::relay_cannot_see()}" }
            }
            p { class: "caption", "{s::relay_traffic_note()}" }
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
        div { class: "overlay", role: "dialog", aria_modal: "true", aria_label: s::relay_setup_title(),
            div { class: "sheet stack relay-setup",
                h2 { "{s::relay_setup_title()}" }
                RelayExplainer { server }
                div { class: "stack-s",
                    button {
                        class: "btn primary block",
                        r#type: "button",
                        onclick: move |_| {
                            let _ = yes.set_relay_enabled(true);
                            nav.update_prefs(&yes, |p| p.relay_asked = true);
                        },
                        "{s::relay_setup_yes()}"
                    }
                    button {
                        class: "btn ghost block",
                        r#type: "button",
                        onclick: move |_| {
                            let _ = no.set_relay_enabled(false);
                            nav.update_prefs(&no, |p| p.relay_asked = true);
                        },
                        "{s::relay_setup_no()}"
                    }
                    p { class: "caption footnote", "{s::relay_setup_later()}" }
                }
            }
        }
    }
}

/// The relay section of the station settings: toggle, status pill, server
/// address (FR-RV-6) and "reset relay identity" (§3.2). A relay-only
/// station (FR-RV-7) has no toggle, and a server fixed to the page's origin
/// (product.md FR-WEBST-2) no address field.
#[component]
pub fn RelaySettings(model: StationModel) -> Element {
    let backend = use_context::<BackendHandle>();
    let nav = use_context::<Nav>();
    let caps = backend.capabilities();
    let set = model.settings.clone();
    let mut server = use_signal(|| set.relay_server.clone());
    let mut error = use_signal(|| None::<String>);
    let mut confirm = use_signal(|| false);
    let pill = logic::relay_pill(&model.relay);
    let (b_toggle, b_server, b_reset) = (backend.clone(), backend.clone(), backend.clone());
    let detail = match &model.relay {
        RelayView::Error {
            permanent: true, ..
        } => Some(s::relay_refused_hint().to_owned()),
        RelayView::Error { message, .. } => Some(message.clone()),
        _ => None,
    };
    rsx! {
        section { class: "stack-s relay-settings", aria_label: s::relay_title(),
            if caps.relay_only {
                h3 { "{s::relay_title()}" }
                p { class: "caption", "{s::relay_only_note()}" }
            } else {
                SwitchRow {
                    label: s::relay_title().to_owned(),
                    checked: set.relay_enabled,
                    onchange: move |v: bool| {
                        let _ = b_toggle.set_relay_enabled(v);
                        nav.update_prefs(&b_toggle, |p| p.relay_asked = true);
                    },
                }
            }
            div { class: "row wrap", role: "status",
                span { class: "{pill.classes()}", "{pill.text}" }
                if let Some(d) = detail {
                    span { class: "caption", "{d}" }
                }
            }
            RelayExplainer { server: set.relay_server.clone() }
            if caps.relay_server_fixed {
                p { class: "caption", "{s::relay_server_fixed()}" }
            } else {
            form {
                class: "field",
                onsubmit: move |e: FormEvent| {
                    e.prevent_default();
                    match b_server.set_relay_server(&server.peek()) {
                        Ok(()) => error.set(None),
                        Err(e) => error.set(Some(e.message())),
                    }
                },
                label { r#for: "relay-server", "{s::relay_server()}" }
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
                    button { class: "btn", r#type: "submit", "{s::save()}" }
                }
                span { id: "relay-server-hint", class: "caption", "{s::relay_server_hint()}" }
                if let Some(e) = error.read().as_ref() {
                    span { class: "caption", role: "alert", "{e}" }
                }
                if caps.relay_only {
                    span { class: "caption", "{s::relay_open_app_hint(&set.relay_server)}" }
                }
            }
            }
            div { class: "stack-s",
                if *confirm.read() {
                    p { class: "caption", "{s::relay_reset_hint()}" }
                    div { class: "row",
                        button {
                            class: "btn danger",
                            r#type: "button",
                            onclick: move |_| {
                                let _ = b_reset.reset_relay_identity();
                                confirm.set(false);
                            },
                            "{s::relay_reset_confirm()}"
                        }
                        button {
                            class: "btn ghost",
                            r#type: "button",
                            onclick: move |_| confirm.set(false),
                            "{s::cancel()}"
                        }
                    }
                } else {
                    button {
                        class: "btn ghost",
                        r#type: "button",
                        onclick: move |_| confirm.set(true),
                        "{s::relay_reset()}"
                    }
                }
            }
        }
    }
}
