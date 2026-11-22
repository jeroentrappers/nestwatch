// SPDX-License-Identifier: AGPL-3.0-or-later
//! Server-side renders of the screens with the demo backend: every status
//! has words, controls have labels, and the strings come from `strings`.

use std::rc::Rc;

use dioxus::prelude::*;
use nestwatch_ui::components::app::{NestwatchAppProps, Screen};
use nestwatch_ui::demo::DemoBackend;
use nestwatch_ui::i18n::{Lang, with_language};
use nestwatch_ui::model::{Platform, Prefs, Role, Theme};
use nestwatch_ui::{BackendHandle, NestwatchApp, strings as s};

fn render(platform: Platform, prefs: Prefs, invitation: Option<String>) -> String {
    render_at(platform, prefs, invitation, None)
}

fn render_at(
    platform: Platform,
    prefs: Prefs,
    invitation: Option<String>,
    start: Option<Screen>,
) -> String {
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
            start,
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
    assert!(html.contains(s::disclaimer()), "{html}");
    assert!(html.contains(s::role_station()) && html.contains(s::role_listener()));
    // The browser listens first; the relay-only station is a secondary
    // choice (product.md FR-WEBST-1).
    let web = render(Platform::Web, Prefs::default(), None);
    assert!(web.contains(s::disclaimer()) && web.contains(s::disclaimer_accept()));
    assert!(!web.contains(s::role_station()));
    assert!(
        web.contains(s::station_use_this_device())
            && web.contains(&escaped(s::role_station_hint_web()))
    );
}

/// product.md §3.10: the browser station screen warns about the tab, never
/// asks whether to use the relay (always on), and the listener home offers
/// the station role.
#[test]
fn browser_station_and_its_entry_render() {
    let html = render(Platform::Web, prefs(Role::Station), None);
    for needle in [
        s::station_web_keep_visible(),
        s::station_keeps_running_web(),
        s::pair_listener(),
        s::role_tab_listening(),
    ] {
        assert!(
            html.contains(&escaped(needle)),
            "missing {needle:?} in {html}"
        );
    }
    assert!(!html.contains(s::relay_setup_title()), "{html}");
    assert!(!html.contains(&escaped(s::station_keeps_running())));
    let home = render(Platform::Web, prefs(Role::Listener), None);
    assert!(home.contains(s::station_use_this_device()), "{home}");
    // No role switch in the browser until the station is in use.
    assert!(!home.contains(s::role_tab_station()), "{home}");
    let desktop = render(Platform::Desktop, prefs(Role::Listener), None);
    assert!(!desktop.contains(s::station_use_this_device()));
}

#[test]
fn listener_home_renders_tiles_with_words() {
    let html = render(Platform::Desktop, prefs(Role::Listener), None);
    for needle in [
        "Nursery",
        &escaped("Leo's room"),
        &escaped("Grandma's"),
        s::status_sound(),
        s::status_quiet(),
        s::status_reconnecting(),
        s::hold_to_talk(),
        s::camera_live(),
        "card tile alert",
        "role=\"meter\"",
        "aria-pressed",
        &escaped(&s::unreachable_for("0:42")),
        s::keeps_listening(),
        // FR-RV-4: the path on the tile.
        s::path_vpn(),
        s::path_relayed(),
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
    assert!(html.contains(s::pair_listener()) && html.contains(s::dim_screen()));
    assert!(html.contains(s::camera_on()));
    // FR-RV-1: setup asks once about the relay, with what the server sees.
    assert!(html.contains(&escaped(s::relay_setup_title())), "{html}");
    assert!(html.contains(s::relay_can_see()) && html.contains(s::relay_cannot_see()));
    assert!(html.contains(s::relay_online()));
    let asked = render(
        Platform::Desktop,
        Prefs {
            relay_asked: true,
            ..prefs(Role::Station)
        },
        None,
    );
    assert!(!asked.contains(&escaped(s::relay_setup_title())));
    let add = render(
        Platform::Web,
        prefs(Role::Listener),
        Some("http://127.0.0.1:47180/#nestwatch%3A%2F%2Finvite".into()),
    );
    assert!(add.contains(s::pairing()), "{add}");
    assert!(add.contains(s::nearby_unavailable()));
}

/// FR-AL-5..7, FR-SL-3: the ringing alarm banner with its answers, the
/// room status, "Not the baby" while awake and the paused notice.
#[test]
fn alarm_room_status_and_pause_render() {
    let html = render(Platform::Desktop, prefs(Role::Listener), None);
    for needle in [
        "card tile alert ringing".to_owned(),
        "alert-banner ringing".to_owned(),
        s::alarm_banner("Nursery"),
        s::alarm_banner_hint().to_owned(),
        escaped(s::im_on_it()),
        s::not_the_baby().to_owned(),
        s::awake_for("3 min"),
        s::quiet_for("1 h 20 min"),
        s::resume_alerts().to_owned(),
    ] {
        assert!(html.contains(&needle), "missing {needle:?} in {html}");
    }
    // The demo clock is 22:23 UTC; Leo's pause ends 7 minutes later.
    assert!(html.contains(&s::alerts_paused_until("22:30")), "{html}");
}

#[test]
fn night_log_screen_renders_nights_and_badges() {
    let html = render_at(
        Platform::Desktop,
        prefs(Role::Listener),
        None,
        Some(Screen::NightLog("nurserynurseryaa".into())),
    );
    for needle in [
        s::night_of("Tue 14 Nov"),
        s::night_of("Mon 13 Nov"),
        s::going_on().to_owned(),
        s::badge_alarm().to_owned(),
        s::not_the_baby().to_owned(),
        s::clear_log().to_owned(),
        s::night_log_footer().to_owned(),
        "wake-up not-baby".to_owned(),
    ] {
        assert!(
            html.contains(&escaped(&needle)),
            "missing {needle:?} in {html}"
        );
    }
    assert!(html.contains("3 wake-ups"), "{html}");
    let empty = render_at(
        Platform::Desktop,
        prefs(Role::Listener),
        None,
        Some(Screen::NightLog("leosroomleosrooo".into())),
    );
    assert!(empty.contains(s::night_log_empty()), "{empty}");
    assert!(!empty.contains(s::clear_log()));
}

/// NFR-I18N-2: the whole listener home in another language, with
/// translated words and no English left in the new parts.
#[test]
fn listener_home_renders_in_french_and_polish() {
    for lang in [Lang::Fr, Lang::Pl, Lang::De] {
        let html = with_language(lang, || {
            let h = render(Platform::Desktop, prefs(Role::Listener), None);
            for needle in [
                s::im_on_it(),
                s::not_the_baby(),
                s::resume_alerts(),
                s::keeps_listening(),
            ] {
                assert!(
                    h.contains(&escaped(needle)),
                    "{lang:?}: missing {needle:?} in {h}"
                );
            }
            h
        });
        for english in [
            "I&#39;m on it",
            "Not the baby",
            "Resume alerts",
            "Hold to talk",
            "Awake for",
        ] {
            assert!(
                !html.contains(english),
                "{lang:?}: English {english:?} left in {html}"
            );
        }
    }
}
