// SPDX-License-Identifier: AGPL-3.0-or-later
//! Add a station (design README §5.5): invitation link, nearby stations,
//! pairing code, verification number check and manual address.

use dioxus::core::spawn_forever;
use dioxus::prelude::*;

use super::app::{Nav, PairFlow, Screen, start_invitation_pairing};
use crate::backend::{BackendHandle, Ui};
use crate::logic;
use crate::model::Nearby;
use crate::strings as s;

/// The add-a-station screen.
#[component]
pub fn AddStation() -> Element {
    let backend = use_context::<BackendHandle>();
    let ui = use_context::<Ui>();
    let mut nav = use_context::<Nav>();
    let caps = backend.capabilities();
    let mut invite = use_signal(String::new);
    let mut code = use_signal(String::new);
    let mut station_id = use_signal(String::new);
    let mut chosen = use_signal(|| None::<Nearby>);
    let mut address = use_signal(String::new);
    let flow = nav.pair.read().clone();
    let nearby = ui.listener.read().nearby.clone();
    let busy = flow == PairFlow::Busy;

    let back = move |_| {
        if !matches!(*nav.pair.peek(), PairFlow::Busy) {
            nav.pair.set(PairFlow::Idle);
        }
        nav.screen.set(Screen::Home);
    };

    // Verification (FR-PAIR-5) replaces the form once paired.
    if let PairFlow::Verify(outcome) = flow.clone() {
        let forget = backend.clone();
        let id = outcome.station_id.clone();
        return rsx! {
            main { class: "page stack narrow",
                div { class: "row",
                    h1 { "{s::ADD_STATION}" }
                }
                div { class: "card stack centered verify-card", role: "alertdialog", aria_label: s::VERIFY_PROMPT,
                    h2 { "{outcome.name}" }
                    span { class: "caption", "{s::VERIFY_PROMPT}" }
                    span { class: "verify", "{outcome.verification}" }
                    div { class: "row full",
                        button {
                            class: "btn ghost grow",
                            r#type: "button",
                            onclick: move |_| {
                                let _ = forget.forget(&id);
                                nav.pair.set(PairFlow::Removed);
                            },
                            "{s::VERIFY_DIFFERENT}"
                        }
                        button {
                            class: "btn primary grow",
                            r#type: "button",
                            onclick: move |_| {
                                nav.pair.set(PairFlow::Idle);
                                nav.screen.set(Screen::Home);
                            },
                            "{s::VERIFY_SAME}"
                        }
                    }
                }
            }
        };
    }

    let b_inv = backend.clone();
    let b_code = backend.clone();
    let code_label = match &*chosen.read() {
        Some(n) => s::code_field_for(&n.name),
        None => s::CODE_FIELD.to_owned(),
    };
    let need_id = chosen.read().is_none();
    rsx! {
        main { class: "page stack narrow",
            div { class: "row",
                button {
                    class: "btn icon ghost",
                    r#type: "button",
                    aria_label: s::BACK,
                    onclick: back,
                    "‹"
                }
                h1 { "{s::ADD_STATION}" }
            }
            match &flow {
                PairFlow::Busy => rsx! { p { class: "card", role: "status", "{s::PAIRING}" } },
                PairFlow::Failed(m) => rsx! { p { class: "card error", role: "alert", "{m}" } },
                PairFlow::Removed => rsx! { p { class: "card", role: "status", "{s::VERIFY_REMOVED}" } },
                _ => rsx! {},
            }
            section { class: "card stack", aria_label: s::NEARBY,
                h3 { "{s::NEARBY}" }
                if !caps.discovery {
                    p { class: "caption", "{s::NEARBY_UNAVAILABLE}" }
                } else if nearby.is_empty() {
                    p { class: "caption", role: "status", "{s::NEARBY_SEARCHING}" }
                } else {
                    div { class: "list",
                        for n in nearby.iter() {
                            {
                                let pick = n.clone();
                                let selected = chosen.read().as_ref().is_some_and(|c| c.id == n.id);
                                rsx! {
                                    button {
                                        key: "{n.id}",
                                        class: "item nearby",
                                        r#type: "button",
                                        aria_pressed: "{selected}",
                                        disabled: !n.pairing || n.known,
                                        onclick: move |_| {
                                            station_id.set(pick.id.clone());
                                            chosen.set(Some(pick.clone()));
                                        },
                                        span { class: "grow", "{n.name}" }
                                        if n.known {
                                            span { class: "caption", "{s::ALREADY_PAIRED}" }
                                        } else if n.pairing {
                                            span { class: "pill calm plain", "{s::PAIRING_OPEN}" }
                                        } else {
                                            span { class: "caption", "{s::NOT_PAIRING}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            form {
                class: "stack-s",
                onsubmit: move |e: FormEvent| {
                    e.prevent_default();
                    let v = invite.peek().trim().to_owned();
                    if !v.is_empty() {
                        start_invitation_pairing(b_inv.clone(), nav, v);
                    }
                },
                div { class: "field",
                    label { r#for: "invite", "{s::INVITE_FIELD}" }
                    input {
                        id: "invite",
                        class: "input",
                        autocomplete: "off",
                        spellcheck: "false",
                        placeholder: "lull://invite?…",
                        value: "{invite}",
                        oninput: move |e: FormEvent| invite.set(e.value()),
                    }
                    span { class: "caption", "{s::INVITE_HINT}" }
                }
                button { class: "btn primary", r#type: "submit", disabled: busy, "{s::INVITE_SUBMIT}" }
            }
            p { class: "caption centered", "{s::OR}" }
            form {
                class: "stack-s",
                onsubmit: move |e: FormEvent| {
                    e.prevent_default();
                    let c = code.peek().clone();
                    let id = station_id.peek().trim().to_owned();
                    let addr = address.peek().trim().to_owned();
                    let routes = match (&*chosen.peek(), addr.is_empty()) {
                        (_, false) => Some(addr),
                        (Some(n), true) => n.endpoints.first().cloned(),
                        (None, true) => None,
                    };
                    let f = b_code.pair_code(c, id, routes);
                    nav.pair.set(PairFlow::Busy);
                    spawn_forever(async move {
                        let r = f.await;
                        nav.pair.set(match r {
                            Ok(o) => PairFlow::Verify(o),
                            Err(e) => PairFlow::Failed(e.message()),
                        });
                    });
                },
                div { class: "field",
                    label { r#for: "code", "{code_label}" }
                    input {
                        id: "code",
                        class: "input code-input",
                        autocomplete: "off",
                        spellcheck: "false",
                        inputmode: "text",
                        placeholder: "XXXXX-XXXXX",
                        value: "{code}",
                        oninput: move |e: FormEvent| code.set(logic::format_code_input(&e.value())),
                    }
                }
                if need_id {
                    div { class: "field",
                        label { r#for: "station-id", "{s::STATION_ID_FIELD}" }
                        input {
                            id: "station-id",
                            class: "input mono",
                            autocomplete: "off",
                            spellcheck: "false",
                            placeholder: "abcd-efgh-ijkl-mnop",
                            value: "{station_id}",
                            oninput: move |e: FormEvent| station_id.set(e.value()),
                        }
                    }
                }
                div { class: "field",
                    label { r#for: "address", "{s::ADDRESS_FIELD}" }
                    input {
                        id: "address",
                        class: "input",
                        autocomplete: "off",
                        spellcheck: "false",
                        placeholder: "host:47100",
                        value: "{address}",
                        oninput: move |e: FormEvent| address.set(e.value()),
                    }
                    span { class: "caption", "{s::ADDRESS_HINT}" }
                }
                button {
                    class: "btn primary",
                    r#type: "submit",
                    disabled: busy || !logic::code_complete(&code.read()),
                    "{s::CODE_SUBMIT}"
                }
            }
        }
    }
}
