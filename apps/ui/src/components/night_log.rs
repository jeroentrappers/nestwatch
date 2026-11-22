// SPDX-License-Identifier: AGPL-3.0-or-later
//! The night log of one station (product.md FR-SL-4): wake-ups grouped by
//! night, newest night first, with clearing behind a confirmation.

use dioxus::prelude::*;

use super::app::{Nav, Screen};
use crate::backend::{BackendHandle, Ui};
use crate::logic;
use crate::strings as s;

/// The night log screen of station `id`.
#[component]
pub fn NightLog(id: String) -> Element {
    let backend = use_context::<BackendHandle>();
    let ui = use_context::<Ui>();
    let mut nav = use_context::<Nav>();
    let mut confirm = use_signal(|| false);
    let now = *ui.now_unix_ms.read();
    // Re-read when the log changes (`log_revision`) and with the clock
    // while a wake-up goes on; both re-render this screen.
    let (name, _revision) = {
        let l = ui.listener.read();
        match l.tile(&id) {
            Some(t) => (t.name.clone(), t.log_revision),
            None => (String::new(), 0),
        }
    };
    let entries = backend.night_log(&id).unwrap_or_default();
    let nights = logic::nights(&entries, now);
    let title = s::night_log_of(&name);
    let question = s::clear_log_confirm(&name);
    let clear = {
        let backend = backend.clone();
        let id = id.clone();
        move |_| {
            let _ = backend.clear_night_log(&id);
            confirm.set(false);
        }
    };
    rsx! {
        main { class: "page stack narrow night-log", aria_label: "{title}",
            div { class: "row",
                button {
                    class: "btn icon ghost",
                    r#type: "button",
                    aria_label: s::back(),
                    onclick: move |_| nav.screen.set(Screen::Home),
                    "‹"
                }
                div { class: "title",
                    h1 { "{s::night_log()}" }
                    span { class: "caption", "{name}" }
                }
            }
            if nights.is_empty() {
                div { class: "card stack empty",
                    p { "{s::night_log_empty()}" }
                }
            }
            for night in nights {
                section { key: "{night.day}", class: "card stack-s night",
                    h2 { "{night.title}" }
                    p { class: "caption", "{night.summary}" }
                    ul { class: "wake-ups",
                        for e in night.entries {
                            li {
                                key: "{e.key}",
                                class: if e.not_the_baby { "wake-up not-baby" } else { "wake-up" },
                                span { "{e.text}" }
                                if e.ongoing {
                                    span { class: "pill sound", "{s::going_on()}" }
                                }
                                if e.alarm {
                                    span { class: "pill alarm", "{s::badge_alarm()}" }
                                }
                                if e.not_the_baby {
                                    span { class: "pill plain muted", "{s::not_the_baby()}" }
                                }
                            }
                        }
                    }
                }
            }
            p { class: "caption footnote", "{s::night_log_footer()}" }
            if *confirm.read() {
                div { class: "card stack-s confirm", role: "alertdialog", aria_label: "{question}",
                    p { "{question}" }
                    div { class: "row",
                        button {
                            class: "btn ghost grow",
                            r#type: "button",
                            onclick: move |_| confirm.set(false),
                            "{s::cancel()}"
                        }
                        button {
                            class: "btn danger grow",
                            r#type: "button",
                            onclick: clear,
                            "{s::clear()}"
                        }
                    }
                }
            } else if !entries.is_empty() {
                button {
                    class: "btn ghost",
                    r#type: "button",
                    onclick: move |_| confirm.set(true),
                    "{s::clear_log()}"
                }
            }
        }
    }
}
