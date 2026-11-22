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
    format!("{} listener", crate::station::browser_name())
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

/// Shows a system notification; `on_click` runs when its body is tapped
/// (FR-AL-6: tapping a sound alert or alarm acknowledges it).
fn show_notification(station_id: &str, title: &str, body: &str, on_click: Option<Box<dyn Fn()>>) {
    if !notifications_supported()
        || web_sys::Notification::permission() != web_sys::NotificationPermission::Granted
    {
        return;
    }
    let opts = web_sys::NotificationOptions::new();
    opts.set_body(body);
    opts.set_tag(station_id);
    opts.set_icon("logo.svg");
    let Ok(n) = web_sys::Notification::new_with_options(title, &opts) else {
        return;
    };
    if let Some(f) = on_click {
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_e: web_sys::Event| {
            if let Some(w) = web_sys::window() {
                let _ = w.focus();
            }
            f();
        });
        n.set_onclick(Some(cb.as_ref().unchecked_ref()));
        cb.forget();
    }
}

fn alert_title(name: &str, alert: Alert) -> String {
    match alert {
        Alert::SoundStarted => s::alert_sound(name),
        Alert::Unreachable => s::alert_unreachable(name),
        Alert::Reachable => s::alert_reachable(name),
        Alert::BatteryLow { percent } => s::alert_battery(name, percent),
        Alert::Alarm => s::alarm_title(name),
    }
}

/// Follows the browser's languages (NFR-I18N-2) and time zone.
fn init_locale() {
    let Some(window) = web_sys::window() else {
        return;
    };
    let nav = window.navigator();
    let mut tags: Vec<String> = nav
        .languages()
        .iter()
        .filter_map(|v| v.as_string())
        .collect();
    if let Some(l) = nav.language() {
        tags.push(l);
    }
    let lang = nestwatch_ui::i18n::Lang::from_tags(&tags);
    nestwatch_ui::i18n::set_language(lang);
    nestwatch_ui::i18n::set_utc_offset(|unix_ms| {
        let d = js_sys::Date::new(&JsValue::from_f64(unix_ms as f64));
        -(d.get_timezone_offset() as i32)
    });
    if let Some(root) = window.document().and_then(|d| d.document_element()) {
        let _ = root.set_attribute("lang", lang.tag());
    }
}

/// Plays the alarm while any station's alarm rings (FR-AL-5), with
/// vibration where the browser offers it.
fn spawn_alarm_watch(engine: ListenerEngine, audio: Rc<Audio>) {
    spawn_local(async move {
        let mut rx = engine.state();
        let mut last_buzz = 0.0;
        loop {
            let ringing =
                rx.borrow_and_update().stations.iter().any(|v| {
                    matches!(v.alarm, nestwatch_core::listener::AlarmView::Ringing { .. })
                });
            audio.set_alarm(ringing);
            let now = js_sys::Date::now();
            if ringing
                && now - last_buzz >= 1_300.0
                && let Some(w) = web_sys::window()
            {
                last_buzz = now;
                let pattern = js_sys::Array::of3(
                    &JsValue::from_f64(400.0),
                    &JsValue::from_f64(200.0),
                    &JsValue::from_f64(400.0),
                );
                let _ = w.navigator().vibrate_with_pattern(&pattern);
            }
            let tick = gloo_timers::future::sleep(std::time::Duration::from_millis(1_300));
            if let Some(Err(_)) = nestwatch_ui::backend::race(rx.changed(), tick).await {
                audio.set_alarm(false);
                break;
            }
        }
    });
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
                    // The alarm has its own sound (`spawn_alarm_watch`).
                    if !matches!(alert, Alert::Reachable | Alert::Alarm) {
                        audio.beep();
                    }
                    let body = if alert == Alert::Alarm {
                        s::alarm_body()
                    } else {
                        s::alert_body()
                    };
                    let on_click: Option<Box<dyn Fn()>> =
                        matches!(alert, Alert::SoundStarted | Alert::Alarm).then(|| {
                            let (e, id) = (engine.clone(), station_id.clone());
                            Box::new(move || {
                                let _ = e.acknowledge(&id);
                            }) as Box<dyn Fn()>
                        });
                    show_notification(&station_id, &alert_title(&name, alert), body, on_click);
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
    });
}

/// The first tap or key press anywhere unlocks audio (browser rule); any
/// later one resumes a browser station's sound that the browser suspended.
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
            crate::station::resume_audio();
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
    init_locale();

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
        cfg.relay_origin = relay.clone();
        let engine = match ListenerEngine::start(cfg, storage.clone(), clock.clone()) {
            Ok(e) => e,
            Err(e) => {
                web_sys::console::error_1(&JsValue::from_str(&format!("Nestwatch: {e}")));
                return;
            }
        };
        let audio = Audio::new(engine.clone());
        unlock_on_first_gesture(audio.clone());
        spawn_event_loop(engine.clone(), audio.clone());
        spawn_alarm_watch(engine.clone(), audio.clone());
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let host = WebHost {
            storage: local.ok(),
            audio,
            ws_url,
            relay_origin: relay,
            engine_storage: storage,
            clock: clock.clone(),
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
