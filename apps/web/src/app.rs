// SPDX-License-Identifier: AGPL-3.0-or-later
//! Boot: engine, storage, audio, alerts and the Dioxus root.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use dioxus::prelude::*;
use nestwatch_core::listener::{Alert, ListenerConfig, ListenerEngine, ListenerEvent};
use nestwatch_core::{Clock, MemoryStorage, Storage, SystemClock};
use nestwatch_ui::{BackendHandle, NestwatchApp, strings as s};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;

use crate::audio::Audio;
use crate::host::{WebHost, notifications_supported};
use crate::storage::LocalStorage;

thread_local! {
    static BOOT: RefCell<Option<(BackendHandle, Option<String>)>> = const { RefCell::new(None) };
}

#[component]
fn App() -> Element {
    let (backend, invitation) = use_hook(|| {
        BOOT.with(|b| b.borrow().clone())
            .expect("nestwatch-web booted before launch")
    });
    rsx! { NestwatchApp { backend, invitation } }
}

fn default_name() -> String {
    let ua = web_sys::window()
        .and_then(|w| w.navigator().user_agent().ok())
        .unwrap_or_default();
    let browser = if ua.contains("Firefox") {
        "Firefox"
    } else if ua.contains("Edg/") {
        "Edge"
    } else if ua.contains("Chrome") {
        "Chrome"
    } else if ua.contains("Safari") {
        "Safari"
    } else {
        "Browser"
    };
    format!("{browser} listener")
}

/// Takes the invitation from the URL fragment (protocol.md §3.5) and
/// removes it from the address bar and history.
fn take_invitation() -> Option<String> {
    let window = web_sys::window()?;
    let location = window.location();
    let hash = location.hash().ok()?;
    if hash.len() <= 1 {
        return None;
    }
    let href = location.href().ok()?;
    let clean = format!(
        "{}{}",
        location.pathname().unwrap_or_else(|_| "/".into()),
        location.search().unwrap_or_default()
    );
    if let Ok(h) = window.history() {
        let _ = h.replace_state_with_url(&JsValue::NULL, "", Some(&clean));
    }
    Some(href)
}

/// The relay server origin when the app is served by a relay server
/// (rendezvous.md FR-RV-5): stations serve it at `/`, the relay server at
/// `/app/` (OPEN-QUESTIONS RELAY-10, RVAPP-5).
fn relay_origin() -> Option<String> {
    let location = web_sys::window()?.location();
    let path = location.pathname().ok()?;
    (path == "/app" || path.starts_with("/app/"))
        .then(|| location.origin().ok())
        .flatten()
}

fn websocket_url() -> String {
    let location = web_sys::window().expect("window").location();
    let scheme = if location.protocol().as_deref() == Ok("https:") {
        "wss"
    } else {
        "ws"
    };
    format!(
        "{scheme}://{}/nestwatch/1",
        location.host().unwrap_or_else(|_| "localhost".into())
    )
}

fn show_notification(station_id: &str, title: &str) {
    if !notifications_supported()
        || web_sys::Notification::permission() != web_sys::NotificationPermission::Granted
    {
        return;
    }
    let opts = web_sys::NotificationOptions::new();
    opts.set_body(s::ALERT_BODY);
    opts.set_tag(station_id);
    opts.set_icon("logo.svg");
    let _ = web_sys::Notification::new_with_options(title, &opts);
}

fn alert_title(name: &str, alert: Alert) -> String {
    match alert {
        Alert::SoundStarted => s::alert_sound(name),
        Alert::Unreachable => s::alert_unreachable(name),
        Alert::Reachable => s::alert_reachable(name),
        Alert::BatteryLow { percent } => s::alert_battery(name, percent),
    }
}

/// Routes engine events to the microphone, the alert sound and system
/// notifications (FR-AL-1..4: alerts ignore mute).
fn spawn_event_loop(engine: ListenerEngine, audio: Rc<Audio>) {
    spawn_local(async move {
        let mut ev = engine.events();
        loop {
            match ev.recv().await {
                Ok(ListenerEvent::MicrophoneWanted { .. }) => audio.start_mic(),
                Ok(ListenerEvent::MicrophoneNotWanted) => audio.stop_mic(),
                Ok(ListenerEvent::Alert { station_id, alert }) => {
                    let name = engine
                        .snapshot()
                        .stations
                        .iter()
                        .find(|v| v.id == station_id)
                        .map(|v| v.name.clone())
                        .unwrap_or_default();
                    if !matches!(alert, Alert::Reachable) {
                        audio.beep();
                    }
                    show_notification(&station_id, &alert_title(&name, alert));
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
    });
}

/// The first tap or key press anywhere unlocks audio (browser rule).
fn unlock_on_first_gesture(audio: Rc<Audio>) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    for event in ["pointerdown", "keydown"] {
        let a = audio.clone();
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
            if !a.is_unlocked() {
                a.unlock();
            }
        });
        let _ =
            doc.add_event_listener_with_callback_and_bool(event, cb.as_ref().unchecked_ref(), true);
        cb.forget();
    }
}

// The browser is single-threaded; engine ports are `Arc` by API.
#[allow(clippy::arc_with_non_send_sync)]
pub fn run() {
    std::panic::set_hook(Box::new(|info| {
        web_sys::console::error_1(&JsValue::from_str(&format!("Nestwatch panicked: {info}")));
    }));
    let invitation = take_invitation();

    #[cfg(feature = "demo")]
    {
        let prefs = {
            let mut p = nestwatch_ui::model::Prefs {
                role: Some(nestwatch_ui::model::Role::Listener),
                disclaimer_ok: true,
                ..Default::default()
            };
            if let Some(t) = crate::host::forced_theme() {
                p.theme = t;
            }
            p
        };
        let platform = if web_sys::window()
            .and_then(|w| w.location().search().ok())
            .is_some_and(|q| q.contains("desktop"))
        {
            nestwatch_ui::model::Platform::Desktop
        } else {
            nestwatch_ui::model::Platform::Web
        };
        let mut prefs = prefs;
        if web_sys::window()
            .and_then(|w| w.location().search().ok())
            .is_some_and(|q| q.contains("station"))
        {
            prefs.role = Some(nestwatch_ui::model::Role::Station);
        }
        if web_sys::window()
            .and_then(|w| w.location().search().ok())
            .is_some_and(|q| q.contains("welcome"))
        {
            prefs = nestwatch_ui::model::Prefs::default();
        }
        let backend = BackendHandle::new(nestwatch_ui::demo::DemoBackend::new(
            platform,
            prefs,
            Rc::new(crate::host::sleep),
            Rc::new(crate::host::apply_theme),
        ));
        BOOT.with(|b| *b.borrow_mut() = Some((backend, invitation)));
        dioxus::launch(App);
        return;
    }

    #[cfg(not(feature = "demo"))]
    {
        // Hosted on a relay server: every connection goes through its
        // /nwr/1; served by a station: its own WebSocket (protocol.md §3.3).
        let relay = relay_origin();
        let ws_url = relay.is_none().then(websocket_url);
        let local = LocalStorage::open();
        let storage: Arc<dyn Storage> = match &local {
            Ok(l) => Arc::new(l.clone()),
            Err(e) => {
                web_sys::console::warn_1(&JsValue::from_str(&format!(
                    "Nestwatch: {e}; pairings are forgotten on reload"
                )));
                Arc::new(MemoryStorage::new())
            }
        };
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let mut cfg = ListenerConfig::new(default_name());
        cfg.discovery = false;
        cfg.default_websocket = ws_url.clone();
        cfg.relay_origin = relay;
        let engine = match ListenerEngine::start(cfg, storage, clock.clone()) {
            Ok(e) => e,
            Err(e) => {
                web_sys::console::error_1(&JsValue::from_str(&format!("Nestwatch: {e}")));
                return;
            }
        };
        let audio = Audio::new(engine.clone());
        unlock_on_first_gesture(audio.clone());
        spawn_event_loop(engine.clone(), audio.clone());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let host = WebHost {
            storage: local.ok(),
            audio,
            ws_url,
            tx,
            rx: RefCell::new(Some(rx)),
        };
        let backend = BackendHandle::new(nestwatch_ui::adapter::EngineBackend::new(
            Rc::new(host),
            clock,
            engine,
        ));
        BOOT.with(|b| *b.borrow_mut() = Some((backend, invitation)));
        dioxus::launch(App);
    }
}
