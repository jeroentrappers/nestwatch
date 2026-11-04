// SPDX-License-Identifier: AGPL-3.0-or-later
//! Welcome: promise, medical disclaimer (product.md §1) and role choice
//! (design README §5.1).

use dioxus::prelude::*;

use super::app::Nav;
use super::common::Logo;
use crate::backend::BackendHandle;
use crate::model::{Platform, Role};
use crate::strings as s;

/// First launch.
#[component]
pub fn Welcome() -> Element {
    let backend = use_context::<BackendHandle>();
    let nav = use_context::<Nav>();
    let caps = backend.capabilities();
    let choose = {
        let backend = backend.clone();
        move |role: Role| {
            nav.update_prefs(&backend, |p| {
                p.role = Some(role);
                p.disclaimer_ok = true;
                if role == Role::Station {
                    p.station_autostart = true;
                }
            });
        }
    };
    let choose2 = choose.clone();
    let choose3 = choose.clone();
    // The browser station is the exception (product.md §3.10): listening
    // stays the main path, the station a secondary choice.
    let cards = caps.station && caps.platform == Platform::Desktop;
    rsx! {
        main { class: "page welcome stack",
            div { class: "stack hero-brand",
                Logo { size: 64 }
                h1 { "{s::APP}" }
                p { class: "muted", "{s::PROMISE}" }
            }
            div { class: "card disclaimer", role: "note",
                p { "{s::DISCLAIMER}" }
            }
            if cards {
                div { class: "role-choice",
                    button {
                        class: "card role-card",
                        r#type: "button",
                        onclick: move |_| choose(Role::Station),
                        h2 { "{s::ROLE_STATION}" }
                        p { class: "muted", "{s::ROLE_STATION_HINT}" }
                    }
                    button {
                        class: "card role-card",
                        r#type: "button",
                        onclick: move |_| choose2(Role::Listener),
                        h2 { "{s::ROLE_LISTENER}" }
                        p { class: "muted", "{s::ROLE_LISTENER_HINT}" }
                    }
                }
            } else {
                button {
                    class: "btn primary block",
                    r#type: "button",
                    onclick: move |_| choose(Role::Listener),
                    "{s::DISCLAIMER_ACCEPT}"
                }
                if caps.station {
                    div { class: "stack-s web-station-entry",
                        button {
                            class: "btn ghost block",
                            r#type: "button",
                            onclick: move |_| choose3(Role::Station),
                            "{s::STATION_USE_THIS_DEVICE}"
                        }
                        p { class: "caption", "{s::ROLE_STATION_HINT_WEB}" }
                    }
                }
            }
        }
    }
}
