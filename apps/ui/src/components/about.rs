// SPDX-License-Identifier: AGPL-3.0-or-later
//! About: promise, medical disclaimer (product.md §1), theme, device ID.

use dioxus::prelude::*;

use super::app::{Nav, Screen};
use super::common::{Logo, Segmented};
use crate::backend::{BackendHandle, Ui};
use crate::logic;
use crate::model::{Role, StationRun, Theme};
use crate::strings as s;

/// Theme choice, also used in station settings.
#[component]
pub fn ThemePicker() -> Element {
    let backend = use_context::<BackendHandle>();
    let nav = use_context::<Nav>();
    let theme = nav.prefs.read().theme;
    let options = vec![
        (s::theme_system().to_owned(), theme == Theme::System),
        (s::theme_night().to_owned(), theme == Theme::Night),
        (s::theme_day().to_owned(), theme == Theme::Day),
    ];
    rsx! {
        div { class: "row-between wrap",
            span { "{s::theme()}" }
            Segmented {
                label: s::theme().to_owned(),
                options,
                onselect: move |i: usize| {
                    let t = [Theme::System, Theme::Night, Theme::Day][i];
                    nav.update_prefs(&backend, |p| p.theme = t);
                },
            }
        }
    }
}

/// The about screen.
#[component]
pub fn About() -> Element {
    let backend = use_context::<BackendHandle>();
    let ui = use_context::<Ui>();
    let mut nav = use_context::<Nav>();
    let role = nav.prefs.read().role;
    let device_id = match (role, &*ui.station.read()) {
        (Some(Role::Station), StationRun::Running(m)) => m.device_id.clone(),
        _ => ui.listener.read().device_id.clone(),
    };
    rsx! {
        main { class: "page stack narrow",
            div { class: "row",
                button {
                    class: "btn icon ghost",
                    r#type: "button",
                    aria_label: s::back(),
                    onclick: move |_| nav.screen.set(Screen::Home),
                    "‹"
                }
                h1 { "{s::about()}" }
            }
            div { class: "card stack",
                div { class: "row",
                    Logo { size: 48 }
                    div {
                        h2 { "{s::app()}" }
                        span { class: "caption", "{s::version()} {backend.version()}" }
                    }
                }
                p { "{s::about_body()}" }
                p { class: "disclaimer", role: "note", strong { "{s::disclaimer()}" } }
                if !device_id.is_empty() {
                    div { class: "field",
                        span { class: "caption", "{s::device_id()}" }
                        span { class: "mono", "{logic::grouped_id(&device_id)}" }
                    }
                }
                p { class: "caption", "{s::license()}" }
            }
            div { class: "card stack", ThemePicker {} }
        }
    }
}
