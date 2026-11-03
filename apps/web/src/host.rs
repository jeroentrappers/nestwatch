// SPDX-License-Identifier: AGPL-3.0-or-later
//! The browser [`Host`]: preferences in `localStorage`, theme on `<html>`,
//! audio unlock, notifications and the page's own station WebSocket.

use std::rc::Rc;

use dioxus::core::spawn_forever;
use dioxus::prelude::*;
use lull_ui::adapter::Host;
use lull_ui::model::{Capabilities, Platform, Prefs, Theme};
use lull_ui::{LocalFuture, Ui};
use tokio::sync::mpsc;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::audio::Audio;
use crate::storage::LocalStorage;

const PREFS_KEY: &str = "ui-prefs";

/// Messages from browser callbacks to the UI thread's Dioxus tasks.
#[derive(Debug)]
pub enum HostMsg {
    /// Playback runs.
    AudioUnlocked,
    /// Whether a notification permission prompt can still be shown.
    NotificationsAskable(bool),
}

/// The browser host.
pub struct WebHost {
    pub storage: Option<LocalStorage>,
    pub audio: Rc<Audio>,
    pub ws_url: String,
    pub tx: mpsc::UnboundedSender<HostMsg>,
    pub rx: std::cell::RefCell<Option<mpsc::UnboundedReceiver<HostMsg>>>,
}

impl std::fmt::Debug for WebHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WebHost")
            .field("ws_url", &self.ws_url)
            .finish_non_exhaustive()
    }
}

/// Whether the page runs in a secure context (HTTPS or localhost).
pub fn secure_context() -> bool {
    web_sys::window().is_some_and(|w| w.is_secure_context())
}

/// The Notification API exists and may be used here.
pub fn notifications_supported() -> bool {
    secure_context()
        && web_sys::window().is_some_and(|w| {
            js_sys::Reflect::has(&w, &JsValue::from_str("Notification")).unwrap_or(false)
        })
}

/// `?day` / `?night` in the address force a theme (as in the mockups).
pub fn forced_theme() -> Option<Theme> {
    let search = web_sys::window()?.location().search().ok()?;
    if search.contains("day") {
        Some(Theme::Day)
    } else if search.contains("night") {
        Some(Theme::Night)
    } else {
        None
    }
}

/// Sets `data-theme` on `<html>`.
pub fn apply_theme(theme: Theme) {
    let Some(root) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.document_element())
    else {
        return;
    };
    match theme.attr() {
        Some(t) => {
            let _ = root.set_attribute("data-theme", t);
        }
        None => {
            let _ = root.remove_attribute("data-theme");
        }
    }
}

/// Browser timer.
pub fn sleep(ms: u64) -> LocalFuture<()> {
    Box::pin(gloo_timers::future::sleep(
        std::time::Duration::from_millis(ms),
    ))
}

impl Host for WebHost {
    fn capabilities(&self) -> Capabilities {
        let mic = secure_context()
            && web_sys::window().is_some_and(|w| w.navigator().media_devices().is_ok());
        Capabilities {
            platform: Platform::Web,
            station: false,
            discovery: false,
            talk: mic,
        }
    }

    fn attach(&self, mut ui: Ui) {
        ui.audio_locked.set(!self.audio.is_unlocked());
        ui.notifications_askable.set(
            notifications_supported()
                && web_sys::Notification::permission() == web_sys::NotificationPermission::Default,
        );
        let tx = self.tx.clone();
        self.audio.on_unlocked(move || {
            let _ = tx.send(HostMsg::AudioUnlocked);
        });
        if let Some(mut rx) = self.rx.borrow_mut().take() {
            spawn_forever(async move {
                while let Some(m) = rx.recv().await {
                    match m {
                        HostMsg::AudioUnlocked => ui.audio_locked.set(false),
                        HostMsg::NotificationsAskable(a) => ui.notifications_askable.set(a),
                    }
                }
            });
        }
    }

    fn load_prefs(&self) -> Prefs {
        let mut p = self
            .storage
            .as_ref()
            .and_then(|s| s.get_text(PREFS_KEY))
            .map(|s| Prefs::decode(&s))
            .unwrap_or_default();
        if let Some(t) = forced_theme() {
            p.theme = t;
        }
        p
    }

    fn save_prefs(&self, prefs: &Prefs) {
        if let Some(s) = &self.storage {
            s.set_text(PREFS_KEY, &prefs.encode());
        }
    }

    fn apply_theme(&self, theme: Theme) {
        apply_theme(theme);
    }

    fn sleep(&self, ms: u64) -> LocalFuture<()> {
        sleep(ms)
    }

    fn unlock_audio(&self) {
        self.audio.unlock();
    }

    fn request_notifications(&self) {
        if !notifications_supported() {
            return;
        }
        let Ok(p) = web_sys::Notification::request_permission() else {
            return;
        };
        let tx = self.tx.clone();
        spawn_local(async move {
            let _ = JsFuture::from(p).await;
            let _ = tx.send(HostMsg::NotificationsAskable(
                web_sys::Notification::permission() == web_sys::NotificationPermission::Default,
            ));
        });
    }

    fn default_route(&self) -> Option<String> {
        Some(self.ws_url.clone())
    }
}
