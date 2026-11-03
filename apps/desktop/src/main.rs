// SPDX-License-Identifier: AGPL-3.0-or-later
//! Nestwatch desktop app (Linux, macOS, Windows): a Dioxus window over the
//! station and listener engines (platforms.md §2.4).
//!
//! Closing the window hides it; the station and listener keep running and
//! the tray icon (tooltip = current state) brings the window back or quits.

use std::cell::RefCell;
use std::sync::Arc;

use dioxus::desktop::trayicon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use dioxus::desktop::trayicon::{Icon as TrayIcon, init_tray_icon};
use dioxus::desktop::{
    Config, LogicalSize, WindowBuilder, WindowCloseBehaviour, use_tray_menu_event_handler, window,
};
use dioxus::prelude::*;
use nestwatch_core::listener::{ListenerConfig, ListenerEngine};
use nestwatch_core::{Clock, SystemClock};
use nestwatch_desktop::station_host::StationOptions;
use nestwatch_desktop::{DesktopHost, listener_host};
use nestwatch_platform::{DesktopOptions, DesktopPlatform};
use nestwatch_ui::model::{StationRun, Theme};
use nestwatch_ui::{APP_CSS, BackendHandle, NESTWATCH_CSS, NestwatchApp, Ui, logic};

struct Boot {
    platform: DesktopPlatform,
    storage: Arc<dyn nestwatch_core::Storage>,
    clock: Arc<dyn Clock>,
    listener: ListenerEngine,
}

thread_local! {
    static BOOT: RefCell<Option<Boot>> = const { RefCell::new(None) };
}

fn theme_script(t: Theme) -> String {
    match t.attr() {
        Some(a) => format!("document.documentElement.setAttribute('data-theme', '{a}');"),
        None => "document.documentElement.removeAttribute('data-theme');".into(),
    }
}

fn js_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The tray tooltip: state in words (platforms.md §2.4).
fn tray_state(ui: &Ui) -> String {
    let mut parts = vec![nestwatch_ui::strings::APP.to_owned()];
    if let StationRun::Running(m) = &*ui.station.read() {
        parts.push(format!(
            "{}: {}",
            m.name,
            logic::listening_count(m.listeners.len())
        ));
    }
    for t in &ui.listener.read().tiles {
        parts.push(format!("{}: {}", t.name, logic::tile_pill(t).text));
    }
    parts.join(" · ")
}

#[component]
fn App() -> Element {
    let backend = use_hook(|| {
        let b = BOOT.with(|b| b.borrow_mut().take()).expect("booted once");
        let host = DesktopHost {
            platform: b.platform,
            storage: b.storage,
            clock: b.clock,
            station: StationOptions::from_env(),
            theme: Box::new(|t| {
                document::eval(&theme_script(t));
            }),
            clipboard: Box::new(|text| {
                document::eval(&format!(
                    "navigator.clipboard && navigator.clipboard.writeText({});",
                    js_string(text)
                ));
                true
            }),
        };
        nestwatch_desktop::backend(host, b.listener)
    });
    rsx! {
        NestwatchApp { backend: backend.clone() }
        Tray { backend }
    }
}

/// Tray icon with Show and Quit; the tooltip follows the state.
#[component]
fn Tray(backend: BackendHandle) -> Element {
    let tray = use_hook(|| {
        let menu = Menu::new();
        let show = MenuItem::with_id("show", "Show Nestwatch", true, None);
        let quit = MenuItem::with_id("quit", "Quit Nestwatch", true, None);
        let _ = menu.append_items(&[&show, &PredefinedMenuItem::separator(), &quit]);
        let icon = TrayIcon::from_rgba(nestwatch_desktop::logo_rgba(32), 32, 32).ok();
        init_tray_icon(menu, icon)
    });
    let ui = use_context::<Ui>();
    {
        let tray = tray.clone();
        use_effect(move || {
            let _ = tray.set_tooltip(Some(tray_state(&ui)));
        });
    }
    use_tray_menu_event_handler(move |e: &MenuEvent| match e.id().as_ref() {
        "show" => {
            let w = window();
            w.set_visible(true);
            w.set_focus();
        }
        "quit" => {
            let b = backend.clone();
            spawn(async move {
                b.stop_station().await;
                std::process::exit(0);
            });
        }
        _ => {}
    });
    rsx! {}
}

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    let _enter = rt.enter();
    let platform = DesktopPlatform::detect(&DesktopOptions::default());
    log::info!("platform backends:\n{}", platform.report);
    let storage = nestwatch_desktop::engine_storage(&platform);
    let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
    let mut cfg = ListenerConfig::new("Desktop listener");
    cfg.discovery = std::env::var("NESTWATCH_NO_MDNS").is_err();
    let listener = match ListenerEngine::start(cfg, storage.clone(), clock.clone()) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("nestwatch-desktop: cannot start the listener: {e}");
            std::process::exit(1);
        }
    };
    // Kept for the whole process: speaker, talk-back microphone, alerts.
    let _devices = listener_host::start(&platform, &listener);
    BOOT.with(|b| {
        *b.borrow_mut() = Some(Boot {
            platform,
            storage,
            clock,
            listener,
        })
    });
    let icon =
        dioxus::desktop::tao::window::Icon::from_rgba(nestwatch_desktop::logo_rgba(64), 64, 64)
            .ok();
    let head = format!("<style>{NESTWATCH_CSS}</style><style>{APP_CSS}</style>");
    let config = Config::new()
        .with_window(
            WindowBuilder::new()
                .with_title("Nestwatch")
                .with_window_icon(icon.clone())
                .with_inner_size(LogicalSize::new(1180.0, 820.0))
                .with_min_inner_size(LogicalSize::new(360.0, 560.0)),
        )
        .with_custom_head(head)
        .with_background_color((0x0f, 0x10, 0x17, 0xff))
        .with_menu(None)
        // Keep monitoring when the window closes (platforms.md §2.4).
        .with_close_behaviour(WindowCloseBehaviour::WindowHides)
        .with_tray_icon_show_window_on_click(true);
    let config = match icon {
        Some(i) => config.with_icon(i),
        None => config,
    };
    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .launch(App);
}
