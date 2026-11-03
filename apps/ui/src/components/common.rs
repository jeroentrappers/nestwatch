// SPDX-License-Identifier: AGPL-3.0-or-later
//! Small building blocks: logo, pills, meters, ring, switches.

use std::sync::LazyLock;

use base64::Engine as _;
use dioxus::prelude::*;

use crate::logic::{self, Pill};
use crate::strings as s;

/// The mark from `design/logo.svg` as a `data:` URI (the CSP allows
/// `img-src data:`; no extra file to serve).
pub static LOGO_SRC: LazyLock<String> = LazyLock::new(|| {
    format!(
        "data:image/svg+xml;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(crate::LOGO_SVG)
    )
});

/// CSS custom properties as style attributes. Dioxus sets `style`
/// namespace attributes with `style.setProperty`, which a strict CSP
/// (`style-src 'self'`, no inline styles) allows.
pub fn css_vars(vars: &[(&'static str, String)]) -> Vec<Attribute> {
    vars.iter()
        .map(|(k, v)| Attribute::new(k, v.clone(), Some("style"), false))
        .collect()
}

/// The brand mark.
#[component]
pub fn Logo(#[props(default = 32)] size: u32) -> Element {
    let src = LOGO_SRC.as_str();
    rsx! {
        img { src: "{src}", width: "{size}", height: "{size}", alt: "" }
    }
}

/// Brand with a title next to the mark.
#[component]
pub fn Brand(title: String) -> Element {
    rsx! {
        span { class: "brand",
            Logo {}
            span { "{title}" }
        }
    }
}

/// A status pill: always a word, never colour alone (NFR-A11Y-1).
#[component]
pub fn StatusPill(pill: Pill) -> Element {
    rsx! {
        span { class: "{pill.classes()}", role: "status", "{pill.text}" }
    }
}

/// Level bar with this listener's threshold mark (listener tiles).
#[component]
pub fn Meter(level: Option<f32>, threshold: Option<f32>) -> Element {
    let f = level.map(logic::level_fraction).unwrap_or(0.0);
    let mut vars = vec![("--level", logic::css_fraction(f))];
    if let Some(t) = threshold {
        vars.push(("--threshold", logic::css_fraction(logic::level_fraction(t))));
    }
    let attrs = css_vars(&vars);
    let now = level.unwrap_or(logic::METER_FLOOR_DBFS).round();
    let text = level
        .map(logic::db)
        .unwrap_or_else(|| s::NO_LEVEL.to_owned());
    rsx! {
        div {
            class: "meter",
            role: "meter",
            aria_label: s::METER_LABEL,
            aria_valuemin: "{logic::METER_FLOOR_DBFS}",
            aria_valuemax: "{logic::METER_CEIL_DBFS}",
            aria_valuenow: "{now}",
            aria_valuetext: "{text}",
            ..attrs,
            div { class: "fill" }
            if threshold.is_some() {
                div { class: "mark" }
            }
        }
    }
}

/// The station's level ring with the strictest threshold tick.
#[component]
pub fn Ring(
    level: f32,
    threshold: Option<f32>,
    sound: bool,
    #[props(default)] small: bool,
) -> Element {
    let mut vars = vec![("--level", logic::css_fraction(logic::level_fraction(level)))];
    if let Some(t) = threshold {
        vars.push(("--threshold", logic::css_fraction(logic::level_fraction(t))));
    }
    let attrs = css_vars(&vars);
    let mut class = String::from("ring");
    if threshold.is_none() {
        class.push_str(" no-tick");
    }
    if small {
        class.push_str(" small");
    }
    let word = if sound { s::RING_SOUND } else { s::RING_QUIET };
    let caption = if small {
        word.to_owned()
    } else {
        format!("{} · {word}", s::DB)
    };
    let text = format!("{} · {word}", logic::db(level));
    rsx! {
        div {
            class: "{class}",
            role: "meter",
            aria_label: s::RING_LABEL,
            aria_valuemin: "{logic::METER_FLOOR_DBFS}",
            aria_valuemax: "{logic::METER_CEIL_DBFS}",
            aria_valuenow: "{level.round()}",
            aria_valuetext: "{text}",
            ..attrs,
            div { class: "value",
                strong { "{logic::db_number(level)}" }
                span { class: "caption", "{caption}" }
            }
        }
    }
}

/// A labelled switch row (`.switch` is a styled checkbox with
/// `role="switch"`).
#[component]
pub fn SwitchRow(
    label: String,
    checked: bool,
    onchange: EventHandler<bool>,
    #[props(default)] disabled: bool,
) -> Element {
    rsx! {
        label { class: "row-between switch-row",
            span { "{label}" }
            input {
                class: "switch",
                r#type: "checkbox",
                role: "switch",
                aria_checked: "{checked}",
                checked,
                disabled,
                onchange: move |e: FormEvent| onchange.call(e.checked()),
            }
        }
    }
}

/// A toggle button with `aria-pressed`.
#[component]
pub fn Toggle(
    label: String,
    pressed: bool,
    onclick: EventHandler<()>,
    #[props(default)] disabled: bool,
    #[props(default)] aria_label: Option<String>,
) -> Element {
    let aria = aria_label.unwrap_or_else(|| label.clone());
    rsx! {
        button {
            class: "btn",
            r#type: "button",
            aria_pressed: "{pressed}",
            aria_label: "{aria}",
            disabled,
            onclick: move |_| onclick.call(()),
            "{label}"
        }
    }
}

/// A segmented control (`.segmented`, buttons with `aria-pressed`).
#[component]
pub fn Segmented(
    label: String,
    options: Vec<(String, bool)>,
    onselect: EventHandler<usize>,
) -> Element {
    rsx! {
        div { class: "segmented", role: "group", aria_label: "{label}",
            for (i, (text, on)) in options.into_iter().enumerate() {
                button {
                    key: "{i}",
                    r#type: "button",
                    aria_pressed: "{on}",
                    onclick: move |_| onselect.call(i),
                    "{text}"
                }
            }
        }
    }
}
