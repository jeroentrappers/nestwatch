// SPDX-License-Identifier: AGPL-3.0-or-later
//! Server-side renders of the screens with the demo backend: every status
//! has words, controls have labels, and the strings come from `strings`.

use std::rc::Rc;

use dioxus::prelude::*;
use nestwatch_ui::components::app::NestwatchAppProps;
use nestwatch_ui::demo::DemoBackend;
use nestwatch_ui::model::{Platform, Prefs, Role, Theme};
use nestwatch_ui::{BackendHandle, NestwatchApp, strings as s};

fn render(platform: Platform, prefs: Prefs, invitation: Option<String>) -> String {
    let backend = BackendHandle::new(DemoBackend::new(
        platform,
        prefs,
        Rc::new(|_| Box::pin(std::future::pending())),
        Rc::new(|_: Theme| {}),
    ));
    let mut dom = VirtualDom::new_with_props(
        NestwatchApp,
        NestwatchAppProps {
            backend,
            invitation,
            children: VNode::empty(),
        },
    );
    dom.rebuild_in_place();
    // Apply signal writes made while mounting (the demo station starts).
    for _ in 0..3 {
        dom.process_events();
        dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
    }
    dioxus_ssr::render(&dom)
}

fn escaped(t: &str) -> String {
    t.replace('&', "&amp;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
}

fn prefs(role: Role) -> Prefs {
    Prefs {
        role: Some(role),
        disclaimer_ok: true,
        ..Prefs::default()
    }
}

#[test]
fn first_launch_shows_the_disclaimer_and_roles() {
    let html = render(Platform::Desktop, Prefs::default(), None);
    assert!(html.contains(s::DISCLAIMER), "{html}");
    assert!(html.contains(s::ROLE_STATION) && html.contains(s::ROLE_LISTENER));
    // The browser listens first; the relay-only station is a secondary
    // choice (product.md FR-WEBST-1).
    let web = render(Platform::Web, Prefs::default(), None);
    assert!(web.contains(s::DISCLAIMER) && web.contains(s::DISCLAIMER_ACCEPT));
    assert!(!web.contains(s::ROLE_STATION));
    assert!(
        web.contains(s::STATION_USE_THIS_DEVICE)
            && web.contains(&escaped(s::ROLE_STATION_HINT_WEB))
    );
}

/// product.md §3.10: the browser station screen warns about the tab, never
/// asks whether to use the relay (always on), and the listener home offers
/// the station role.
#[test]
fn browser_station_and_its_entry_render() {
    let html = render(Platform::Web, prefs(Role::Station), None);
    for needle in [
        s::STATION_WEB_KEEP_VISIBLE,
        s::STATION_KEEPS_RUNNING_WEB,
        s::PAIR_LISTENER,
        s::ROLE_TAB_LISTENING,
    ] {
        assert!(
            html.contains(&escaped(needle)),
            "missing {needle:?} in {html}"
        );
    }
    assert!(!html.contains(s::RELAY_SETUP_TITLE), "{html}");
    assert!(!html.contains(&escaped(s::STATION_KEEPS_RUNNING)));
    let home = render(Platform::Web, prefs(Role::Listener), None);
    assert!(home.contains(s::STATION_USE_THIS_DEVICE), "{home}");
    // No role switch in the browser until the station is in use.
    assert!(!home.contains(s::ROLE_TAB_STATION), "{home}");
    let desktop = render(Platform::Desktop, prefs(Role::Listener), None);
    assert!(!desktop.contains(s::STATION_USE_THIS_DEVICE));
}

#[test]
fn listener_home_renders_tiles_with_words() {
    let html = render(Platform::Desktop, prefs(Role::Listener), None);
    for needle in [
        "Nursery",
        &escaped("Leo's room"),
        &escaped("Grandma's"),
        s::STATUS_SOUND,
        s::STATUS_QUIET,
        s::STATUS_RECONNECTING,
        s::HOLD_TO_TALK,
        s::CAMERA_LIVE,
        "card tile alert",
        "role=\"meter\"",
        "aria-pressed",
        &escaped(&s::unreachable_for("0:42")),
        s::KEEPS_LISTENING,
        // FR-RV-4: the path on the tile.
        s::PATH_VPN,
        s::PATH_RELAYED,
    ] {
        assert!(html.contains(needle), "missing {needle:?}");
    }
    // CSS custom properties are set as style properties, not inline CSS text.
    assert!(html.contains("--level:"), "{html}");
}

#[test]
fn station_and_add_station_render() {
    let html = render(Platform::Desktop, prefs(Role::Station), None);
    assert!(html.contains("2 listening"), "{html}");
    assert!(html.contains(s::PAIR_LISTENER) && html.contains(s::DIM_SCREEN));
    assert!(html.contains(s::CAMERA_ON));
    // FR-RV-1: setup asks once about the relay, with what the server sees.
    assert!(html.contains(&escaped(s::RELAY_SETUP_TITLE)), "{html}");
    assert!(html.contains(s::RELAY_CAN_SEE) && html.contains(s::RELAY_CANNOT_SEE));
    assert!(html.contains(s::RELAY_ONLINE));
    let asked = render(
        Platform::Desktop,
        Prefs {
            relay_asked: true,
            ..prefs(Role::Station)
        },
        None,
    );
    assert!(!asked.contains(&escaped(s::RELAY_SETUP_TITLE)));
    let add = render(
        Platform::Web,
        prefs(Role::Listener),
        Some("http://127.0.0.1:47180/#nestwatch%3A%2F%2Finvite".into()),
    );
    assert!(add.contains(s::PAIRING), "{add}");
    assert!(add.contains(s::NEARBY_UNAVAILABLE));
}
