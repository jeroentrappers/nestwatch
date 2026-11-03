// SPDX-License-Identifier: AGPL-3.0-or-later
//! Alerts and pairing notices (product.md §3.5), announced to screen
//! readers and always written out in words.

use dioxus::prelude::*;

use crate::backend::{BackendHandle, Ui};
use crate::model::{AlertKind, Notice, NoticeKind};
use crate::strings as s;

/// Seconds after which informational notices disappear.
const AUTO_DISMISS_MS: u64 = 20_000;

/// Text, pill class and whether the notice stays until dismissed.
pub fn notice_text(kind: &NoticeKind) -> (String, &'static str, bool) {
    match kind {
        NoticeKind::Alert { station, kind, .. } => {
            let name = if station.is_empty() {
                s::UNNAMED
            } else {
                station
            };
            match kind {
                AlertKind::Sound => (s::alert_sound(name), "sound", false),
                AlertKind::Unreachable => (s::alert_unreachable(name), "warn", true),
                AlertKind::Reachable => (s::alert_reachable(name), "calm", false),
                AlertKind::BatteryLow(p) => (s::alert_battery(name, *p), "warn", true),
            }
        }
        NoticeKind::ListenerPaired { name, verification } => (
            format!("{} · {verification}", s::listener_paired(name)),
            "calm",
            false,
        ),
        NoticeKind::Info(t) => (t.clone(), "plain", false),
    }
}

/// The notice stack.
#[component]
pub fn Notices() -> Element {
    let ui = use_context::<Ui>();
    let list = ui.notices.read().clone();
    rsx! {
        section { class: "notices", aria_label: s::NOTICES, aria_live: "assertive",
            for n in list {
                NoticeItem { key: "{n.id}", notice: n.clone() }
            }
        }
    }
}

#[component]
fn NoticeItem(notice: Notice) -> Element {
    let ui = use_context::<Ui>();
    let backend = use_context::<BackendHandle>();
    let (text, class, sticky) = notice_text(&notice.kind);
    let id = notice.id;
    use_hook(move || {
        if !sticky {
            let sleep = backend.sleep(AUTO_DISMISS_MS);
            spawn(async move {
                sleep.await;
                ui.dismiss(id);
            });
        }
    });
    rsx! {
        div { class: "card notice", role: "alert",
            span { class: "pill {class}", "{text}" }
            button {
                class: "btn icon ghost",
                r#type: "button",
                aria_label: s::DISMISS,
                onclick: move |_| ui.dismiss(id),
                "×"
            }
        }
    }
}
