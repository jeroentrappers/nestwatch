// SPDX-License-Identifier: AGPL-3.0-or-later
//! The root component: context, preferences, theme and screen routing.

use dioxus::core::spawn_forever;
use dioxus::prelude::*;

use super::about::About;
use super::add_station::AddStation;
use super::listener::ListenerHome;
use super::notices::Notices;
use super::station::StationScreen;
use super::welcome::Welcome;
use crate::backend::{BackendHandle, Ui};
use crate::model::{PairOutcome, Platform, Prefs, Role};

/// Top-level screens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Screen {
    /// The role's home (listener tiles or station).
    Home,
    /// Add a station (listener).
    AddStation,
    /// About Nestwatch.
    About,
}

/// State of a pairing started from the UI or from the browser address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairFlow {
    /// Nothing going on.
    Idle,
    /// Handshake running.
    Busy,
    /// Paired; the user compares the number (FR-PAIR-5).
    Verify(PairOutcome),
    /// Failed with a message.
    Failed(String),
    /// The user said the numbers differ; the station was forgotten.
    Removed,
}

/// Navigation and app-level state shared through context.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Nav {
    /// Current screen.
    pub screen: Signal<Screen>,
    /// Station whose settings panel is open (listener).
    pub panel: Signal<Option<String>>,
    /// Pairing flow on the listener.
    pub pair: Signal<PairFlow>,
    /// Preferences.
    pub prefs: Signal<Prefs>,
}

impl Nav {
    /// Updates and stores the preferences.
    pub fn update_prefs(mut self, backend: &BackendHandle, f: impl FnOnce(&mut Prefs)) {
        let mut p = self.prefs.peek().clone();
        f(&mut p);
        backend.save_prefs(&p);
        self.prefs.set(p);
    }
}

/// Runs an invitation pairing and records the outcome in `nav.pair`.
pub fn start_invitation_pairing(backend: BackendHandle, mut nav: Nav, input: String) {
    nav.pair.set(PairFlow::Busy);
    nav.screen.set(Screen::AddStation);
    // Not tied to the screen: the pairing finishes even if the user
    // navigates away meanwhile.
    spawn_forever(async move {
        let r = backend.pair_invitation(input).await;
        nav.pair.set(match r {
            Ok(o) => PairFlow::Verify(o),
            Err(e) => PairFlow::Failed(e.message()),
        });
    });
}

/// The Nestwatch app. `invitation` is a pairing invitation to open at start
/// (the browser app's URL fragment, protocol.md §3.5).
#[component]
pub fn NestwatchApp(backend: BackendHandle, #[props(default)] invitation: Option<String>) -> Element {
    let ui = use_hook(|| {
        let ui = Ui::new();
        backend.attach(ui);
        ui
    });
    use_context_provider(|| ui);
    use_context_provider(|| backend.clone());
    let caps = backend.capabilities();
    let nav = use_context_provider(|| Nav {
        screen: Signal::new(Screen::Home),
        panel: Signal::new(None),
        pair: Signal::new(PairFlow::Idle),
        prefs: Signal::new({
            let mut p = backend.load_prefs();
            if !caps.station {
                p.role = Some(Role::Listener);
            }
            p
        }),
    });
    // Theme follows the preference.
    {
        let backend = backend.clone();
        use_effect(move || {
            let theme = nav.prefs.read().theme;
            backend.apply_theme(theme);
        });
    }
    // A station that ran when the app last closed starts again, whatever
    // screen shows (FR-ST-5: monitoring continues).
    {
        let backend = backend.clone();
        use_effect(move || {
            if caps.station && nav.prefs.peek().station_autostart {
                let f = backend.start_station();
                spawn_forever(async move {
                    let _ = f.await;
                });
            }
        });
    }
    // An invitation from the address bar pairs right away.
    {
        let backend = backend.clone();
        let mut pending = Some(invitation);
        use_effect(move || {
            if let Some(Some(inv)) = pending.take() {
                start_invitation_pairing(backend.clone(), nav, inv);
            }
        });
    }

    let prefs = nav.prefs.read().clone();
    let screen = nav.screen.read().clone();
    let body = if !prefs.disclaimer_ok || prefs.role.is_none() {
        rsx! { Welcome {} }
    } else {
        match screen {
            Screen::About => rsx! { About {} },
            Screen::AddStation => rsx! { AddStation {} },
            Screen::Home => match prefs.role {
                Some(Role::Station) if caps.station => rsx! { StationScreen {} },
                _ => rsx! { ListenerHome {} },
            },
        }
    };
    let platform = match caps.platform {
        Platform::Desktop => "desktop",
        Platform::Web => "web",
    };
    rsx! {
        div { class: "app", "data-platform": platform,
            Notices {}
            {body}
        }
    }
}
